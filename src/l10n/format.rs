//! Formatting a compiled pattern: argument values, per-locale rules, and the executor.
//!
//! The executor walks the pattern's arena and writes into the caller's `String`. It allocates
//! nothing itself: text is copied from the arena, numbers are written digit by digit by the
//! backend's formatter, and arguments are matched to slots by comparing names, a handful at most.

use std::fmt;

use fixed_decimal::{Decimal, FloatPrecision};
use icu_decimal::options::{DecimalFormatterOptions, GroupingStrategy};
use icu_decimal::{DecimalFormatter, DecimalFormatterPreferences};
use icu_plurals::{PluralCategory, PluralOperands, PluralRuleType, PluralRules, PluralRulesOptions, PluralRulesPreferences};
use writeable::Writeable;

use crate::l10n::locale::Locale;
use crate::l10n::pattern::{Arena, ArgumentKind, Case, Grouping, NumberStyle, Part, Pattern, Selector, Span};

/// A value for a message argument.
///
/// These are the types a message can use. Anything richer is the caller's to reduce to one of
/// them first: localization formats semantic values, it doesn't stringify objects.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Value<'a> {
    /// Text: substituted as it is, or matched against a select's cases.
    Str(&'a str),
    /// An integer: formatted for the locale, or matched against a plural's cases.
    Int(i64),
    /// A number: formatted for the locale, or matched against a plural's cases.
    Float(f64),
    /// A boolean: `true` or `false` as text or as a select key.
    Bool(bool),
}

impl Value<'_> {
    /// The type's name, as errors spell it.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Str(_) => "string",
            Self::Int(_) | Self::Float(_) => "number",
            Self::Bool(_) => "boolean",
        }
    }
}

impl<'a> From<&'a str> for Value<'a> {
    fn from(value: &'a str) -> Self {
        Self::Str(value)
    }
}

impl<'a> From<&'a String> for Value<'a> {
    fn from(value: &'a String) -> Self {
        Self::Str(value)
    }
}

impl From<bool> for Value<'_> {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<f64> for Value<'_> {
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

macro_rules! value_from_integer {
    ($($ty:ty),*) => {$(
        impl From<$ty> for Value<'_> {
            fn from(value: $ty) -> Self {
                Self::Int(i64::from(value))
            }
        }
    )*};
}

value_from_integer!(i8, i16, i32, i64, u8, u16, u32);

/// Why a message that exists didn't format.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ArgumentError {
    /// The pattern names an argument the caller didn't pass.
    Missing {
        /// The argument's name.
        argument: String,
    },
    /// The argument was passed with a type the pattern can't use there.
    WrongType {
        /// The argument's name.
        argument: String,
        /// What the pattern needs: `number`, or `string or boolean`.
        expected: &'static str,
        /// What was passed, as [`Value::type_name`] or a caller's own type name.
        received: &'static str,
    },
    /// A number argument that is NaN, infinite, or too large to write.
    NotFinite {
        /// The argument's name.
        argument: String,
    },
}

impl fmt::Display for ArgumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing { argument } => write!(f, "argument '{argument}' is missing"),
            Self::WrongType {
                argument,
                expected,
                received,
            } => write!(f, "argument '{argument}' must be a {expected}, not a {received}"),
            Self::NotFinite { argument } => {
                write!(f, "argument '{argument}' is not a finite number")
            }
        }
    }
}

impl std::error::Error for ArgumentError {}

/// What formatting needs from a locale: its plural rules and number formatters, built once.
#[derive(Debug)]
pub struct LocaleRules {
    cardinal: PluralRules,
    ordinal: PluralRules,
    grouped: DecimalFormatter,
    ungrouped: DecimalFormatter,
    min2: DecimalFormatter,
}

impl LocaleRules {
    /// Builds the rules for a locale from the backend's compiled CLDR data. A locale CLDR has no
    /// data for takes its parent's, down to the root rules; the error is the backend's.
    pub(crate) fn new(locale: &Locale) -> Result<Self, String> {
        let id = locale.id();
        let plural = |rule_type| {
            let mut options = PluralRulesOptions::default();
            options.rule_type = Some(rule_type);
            PluralRules::try_new(PluralRulesPreferences::from(id), options).map_err(|error| error.to_string())
        };
        let decimal = |grouping| {
            let mut options = DecimalFormatterOptions::default();
            options.grouping_strategy = Some(grouping);
            DecimalFormatter::try_new(DecimalFormatterPreferences::from(id), options).map_err(|error| error.to_string())
        };
        Ok(Self {
            cardinal: plural(PluralRuleType::Cardinal)?,
            ordinal: plural(PluralRuleType::Ordinal)?,
            grouped: decimal(GroupingStrategy::Auto)?,
            ungrouped: decimal(GroupingStrategy::Never)?,
            min2: decimal(GroupingStrategy::Min2)?,
        })
    }

    const fn formatter(&self, grouping: Grouping) -> &DecimalFormatter {
        match grouping {
            Grouping::Auto => &self.grouped,
            Grouping::Off => &self.ungrouped,
            Grouping::Min2 => &self.min2,
        }
    }
}

/// A pattern being formatted: the arena it lives in, its slots, and the call's arguments.
struct Run<'a> {
    arena: &'a Arena,
    pattern: Pattern,
    rules: &'a LocaleRules,
    args: &'a [(&'a str, Value<'a>)],
}

impl Arena {
    /// Formats a pattern into `out`.
    ///
    /// On an error `out` holds whatever was written before it; the caller owns truncating.
    pub(crate) fn format(
        &self,
        pattern: Pattern,
        rules: &LocaleRules,
        args: &[(&str, Value<'_>)],
        out: &mut String,
    ) -> Result<(), ArgumentError> {
        let run = Run {
            arena: self,
            pattern,
            rules,
            args,
        };
        run.parts(pattern.root, out)
    }
}

impl Run<'_> {
    fn parts(&self, span: Span, out: &mut String) -> Result<(), ArgumentError> {
        for part in &self.arena.parts[span.range()] {
            match *part {
                Part::Text(text) => out.push_str(self.arena.text(text)),
                Part::Simple { slot } => match self.value(slot)? {
                    Value::Str(text) => out.push_str(text),
                    Value::Bool(value) => out.push_str(if value { "true" } else { "false" }),
                    number => {
                        let mut decimal = self.decimal(slot, number, 0)?;
                        write_number(out, self.rules, NumberStyle::DEFAULT, &mut decimal);
                    }
                },
                Part::Number { slot, style } => {
                    let mut decimal = self.decimal(slot, self.value(slot)?, 0)?;
                    write_number(out, self.rules, self.arena.styles[usize::from(style)], &mut decimal);
                }
                Part::Pound { slot, offset } => {
                    let mut decimal = self.decimal(slot, self.value(slot)?, offset)?;
                    write_number(out, self.rules, NumberStyle::DEFAULT, &mut decimal);
                }
                Part::Plural {
                    slot,
                    ordinal,
                    offset,
                    cases,
                } => {
                    let body = self.plural_case(slot, self.value(slot)?, ordinal, offset, cases)?;
                    self.parts(body, out)?;
                }
                Part::Select { slot, cases } => {
                    let key = match self.value(slot)? {
                        Value::Str(key) => key,
                        Value::Bool(true) => "true",
                        Value::Bool(false) => "false",
                        other => return Err(self.wrong_type(slot, other)),
                    };
                    let mut body = None;
                    for case in &self.arena.cases[cases.range()] {
                        match case.selector {
                            Selector::Keyword(keyword) if self.arena.text(keyword) == key => {
                                body = Some(case.parts);
                                break;
                            }
                            Selector::Other if body.is_none() => body = Some(case.parts),
                            _ => {}
                        }
                    }
                    // Compiling guarantees an `other` case.
                    if let Some(body) = body {
                        self.parts(body, out)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The case a plural picks: an exact match on the value before the offset, else the
    /// category of the value minus the offset, else `other`. This is ICU's `PluralFormat` order.
    fn plural_case(
        &self,
        slot: u16,
        value: Value<'_>,
        ordinal: bool,
        offset: i32,
        cases: Span,
    ) -> Result<Span, ArgumentError> {
        let cases = &self.arena.cases[cases.range()];
        #[allow(clippy::cast_precision_loss)] // An exact-match comparison; `=n` is written as a small number.
        let exact = match value {
            Value::Int(value) => value as f64,
            Value::Float(value) => value,
            other => return Err(self.wrong_type(slot, other)),
        };
        #[allow(clippy::float_cmp)] // `=n` is an exact match by definition.
        if let Some(case) = cases
            .iter()
            .find(|case| matches!(case.selector, Selector::Exact(n) if n == exact))
        {
            return Ok(case.parts);
        }
        let rules = if ordinal { &self.rules.ordinal } else { &self.rules.cardinal };
        let category = match value {
            Value::Int(value) => rules.category_for(value.saturating_sub(i64::from(offset))),
            _ => rules.category_for(PluralOperands::from(&self.decimal(slot, value, offset)?)),
        };
        Ok(Self::category_case(cases, category))
    }

    fn category_case(cases: &[Case], category: PluralCategory) -> Span {
        let mut other = None;
        for case in cases {
            match case.selector {
                Selector::Category(wanted) if wanted == category => return case.parts,
                Selector::Other => other = Some(case.parts),
                _ => {}
            }
        }
        // Compiling guarantees an `other` case.
        other.unwrap_or(Span { start: 0, len: 0 })
    }

    fn name(&self, slot: u16) -> &str {
        self.arena.text(self.arena.slot(self.pattern, slot).name)
    }

    fn value(&self, slot: u16) -> Result<Value<'_>, ArgumentError> {
        let name = self.name(slot);
        let Some((_, value)) = self.args.iter().find(|(known, _)| *known == name) else {
            return Err(ArgumentError::Missing {
                argument: name.to_owned(),
            });
        };
        Ok(*value)
    }

    /// A number argument minus an offset, as the backend's decimal.
    fn decimal(&self, slot: u16, value: Value<'_>, offset: i32) -> Result<Decimal, ArgumentError> {
        match value {
            Value::Int(value) => Ok(Decimal::from(value.saturating_sub(i64::from(offset)))),
            Value::Float(value) => {
                Decimal::try_from_f64(value - f64::from(offset), FloatPrecision::RoundTrip).map_err(|_| {
                    ArgumentError::NotFinite {
                        argument: self.name(slot).to_owned(),
                    }
                })
            }
            other => Err(self.wrong_type(slot, other)),
        }
    }

    fn wrong_type(&self, slot: u16, value: Value<'_>) -> ArgumentError {
        let spec = self.arena.slot(self.pattern, slot);
        ArgumentError::WrongType {
            argument: self.arena.text(spec.name).to_owned(),
            expected: match spec.kind {
                ArgumentKind::Select => "string or boolean",
                ArgumentKind::Number | ArgumentKind::Any => "number",
            },
            received: value.type_name(),
        }
    }
}

/// Writes a number in a style. The scale and precision are applied to the decimal; the formatter
/// supplies the locale's digits, separators, and sign. Rounding is half-even, ICU's default.
fn write_number(out: &mut String, rules: &LocaleRules, style: NumberStyle, decimal: &mut Decimal) {
    if style.scale != 0 {
        decimal.absolute.multiply_pow10(i16::from(style.scale));
        decimal.absolute.trim_start();
    }
    decimal.round(-i16::from(style.max_fraction));
    decimal.absolute.trim_end();
    decimal.absolute.pad_end(-i16::from(style.min_fraction));
    // Writing to a `String` can't fail.
    let _ = rules.formatter(style.grouping).format(decimal).write_to(out);
    if style.percent {
        out.push('%');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(locale: &str, pattern: &str, args: &[(&str, Value<'_>)]) -> Result<String, ArgumentError> {
        let rules = LocaleRules::new(&Locale::parse(locale).unwrap()).unwrap();
        let mut arena = Arena::new();
        let pattern = arena.compile(pattern).unwrap();
        let mut out = String::new();
        arena.format(pattern, &rules, args, &mut out)?;
        Ok(out)
    }

    #[test]
    fn simple_substitution() {
        assert_eq!(
            format("en", "Hello, {name}!", &[("name", "Fargoth".into())]).unwrap(),
            "Hello, Fargoth!"
        );
        assert_eq!(format("en", "{on}", &[("on", true.into())]).unwrap(), "true");
        assert_eq!(
            format("en", "{a}{b}{a}", &[("b", "2".into()), ("a", "1".into()), ("c", "3".into())]).unwrap(),
            "121",
            "arguments match by name, in any order, extras ignored"
        );
    }

    #[test]
    fn english_plural_with_exact_and_pound() {
        let pattern = "{count, plural, =0 {No files} one {# file} other {# files}}";
        let text = |count: i64| format("en", pattern, &[("count", count.into())]).unwrap();
        assert_eq!(text(0), "No files");
        assert_eq!(text(1), "1 file");
        assert_eq!(text(2), "2 files");
        assert_eq!(text(1234), "1,234 files");
        assert_eq!(format("en", pattern, &[("count", Value::Float(1.0))]).unwrap(), "1 file");
        assert_eq!(format("en", pattern, &[("count", Value::Float(1.5))]).unwrap(), "1.5 files");
    }

    #[test]
    fn polish_and_russian_plural_categories() {
        let pattern = "{count, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}";
        let text = |count: f64| format("pl", pattern, &[("count", count.into())]).unwrap();
        assert_eq!(text(1.0), "1 plik");
        assert_eq!(text(2.0), "2 pliki");
        assert_eq!(text(5.0), "5 plików");
        assert_eq!(text(22.0), "22 pliki");
        assert_eq!(text(25.0), "25 plików");
        assert_eq!(text(1.5), "1,5 pliku");
        let pattern = "{n, plural, one {# файл} few {# файла} many {# файлов} other {# файла}}";
        let text = |count: i64| format("ru", pattern, &[("n", count.into())]).unwrap();
        assert_eq!(text(1), "1 файл");
        assert_eq!(text(3), "3 файла");
        assert_eq!(text(11), "11 файлов");
        assert_eq!(text(21), "21 файл");
        assert_eq!(text(-1), "-1 файл");
    }

    #[test]
    fn arabic_has_all_six_categories() {
        let pattern = "{n, plural, zero {zero} one {one} two {two} few {few} many {many} other {other}}";
        let text = |count: i64| format("ar", pattern, &[("n", count.into())]).unwrap();
        assert_eq!(text(0), "zero");
        assert_eq!(text(1), "one");
        assert_eq!(text(2), "two");
        assert_eq!(text(3), "few");
        assert_eq!(text(11), "many");
        assert_eq!(text(100), "other");
    }

    #[test]
    fn offset_shifts_the_category_and_pound_but_not_exact_matches() {
        let pattern =
            "{guests, plural, offset:1 =0 {nobody} =1 {{host}} one {{host} and # other} other {{host} and # others}}";
        let text = |guests: i64| format("en", pattern, &[("guests", guests.into()), ("host", "Caius".into())]).unwrap();
        assert_eq!(text(0), "nobody");
        assert_eq!(text(1), "Caius");
        assert_eq!(text(2), "Caius and 1 other");
        assert_eq!(text(3), "Caius and 2 others");
    }

    #[test]
    fn selectordinal() {
        let pattern = "{place, selectordinal, one {#st} two {#nd} few {#rd} other {#th}}";
        let text = |place: i64| format("en", pattern, &[("place", place.into())]).unwrap();
        assert_eq!(text(1), "1st");
        assert_eq!(text(2), "2nd");
        assert_eq!(text(3), "3rd");
        assert_eq!(text(4), "4th");
        assert_eq!(text(11), "11th");
        assert_eq!(text(21), "21st");
    }

    #[test]
    fn select_matches_strings_and_booleans() {
        let pattern = "{mode, select, edit {Editing} view {Viewing} other {Idle}}";
        let text = |mode: &str| format("en", pattern, &[("mode", mode.into())]).unwrap();
        assert_eq!(text("edit"), "Editing");
        assert_eq!(text("view"), "Viewing");
        assert_eq!(text("anything"), "Idle");
        let pattern = "{on, select, true {On} other {Off}}";
        assert_eq!(format("en", pattern, &[("on", true.into())]).unwrap(), "On");
        assert_eq!(format("en", pattern, &[("on", false.into())]).unwrap(), "Off");
    }

    #[test]
    fn numbers_follow_the_locale() {
        let args = [("v", Value::Float(1_234_567.891))];
        assert_eq!(format("en", "{v}", &args).unwrap(), "1,234,567.891");
        assert_eq!(format("de", "{v}", &args).unwrap(), "1.234.567,891");
        assert_eq!(format("fr", "{v, number}", &args).unwrap(), "1\u{202f}234\u{202f}567,891");
        assert_eq!(format("en", "{v, number, integer}", &args).unwrap(), "1,234,568");
        assert_eq!(format("en", "{v, number, :: group-off}", &args).unwrap(), "1234567.891");
        assert_eq!(format("en", "{v, number, ::.00}", &args).unwrap(), "1,234,567.89");
        assert_eq!(format("en", "{v, number, ::.00}", &[("v", 2.into())]).unwrap(), "2.00");
        assert_eq!(format("en", "{v, number, ::.0#}", &[("v", 2.into())]).unwrap(), "2.0");
        assert_eq!(format("en", "{v}", &[("v", Value::Float(1.999_96))]).unwrap(), "2");
        assert_eq!(format("en", "{v}", &[("v", Value::Float(-0.5))]).unwrap(), "-0.5");
        assert_eq!(
            format("en", "{v, number, :: group-min2}", &[("v", 1234.into())]).unwrap(),
            "1234"
        );
        assert_eq!(
            format("en", "{v, number, :: group-min2}", &[("v", 12345.into())]).unwrap(),
            "12,345"
        );
        assert_eq!(
            format("ar-EG", "{v}", &[("v", 123.into())]).unwrap(),
            "١٢٣",
            "the locale's own digits"
        );
    }

    #[test]
    fn percent() {
        let percent =
            |locale: &str, pattern: &str, value: f64| format(locale, pattern, &[("v", Value::Float(value))]).unwrap();
        // The style keyword is `DecimalFormat`'s percent: times 100, a whole number.
        assert_eq!(percent("en", "{v, number, percent}", 0.42), "42%");
        assert_eq!(
            percent("en", "{v, number, percent}", 0.425),
            "42%",
            "half-even, as ICU rounds"
        );
        assert_eq!(percent("en", "{v, number, percent}", 0.435), "44%");
        assert_eq!(percent("en", "{v, number, percent}", 12.5), "1,250%");
        // A skeleton's percent is only the sign; scale/100 multiplies.
        assert_eq!(percent("en", "{v, number, :: percent}", 42.5), "42.5%");
        assert_eq!(percent("de", "{v, number, :: percent scale/100 .0}", 0.4256), "42,6%");
        assert_eq!(percent("de", "{v, number, :: %x100 .0}", 0.4256), "42,6%");
    }

    #[test]
    fn skeletons_keep_six_fraction_digits() {
        let args = [("v", Value::Float(0.123_456_789))];
        assert_eq!(format("en", "{v}", &args).unwrap(), "0.123");
        assert_eq!(format("en", "{v, number}", &args).unwrap(), "0.123");
        assert_eq!(format("en", "{v, number, ::}", &args).unwrap(), "0.123457");
        assert_eq!(format("en", "{v, number, :: scale/1000}", &args).unwrap(), "123.456789");
        assert_eq!(
            format("en", "{v, number, :: scale/0.01}", &[("v", 1250.into())]).unwrap(),
            "12.5"
        );
    }

    #[test]
    fn argument_errors_name_the_argument() {
        assert_eq!(
            format("en", "{count, plural, other {#}}", &[]),
            Err(ArgumentError::Missing {
                argument: "count".into()
            })
        );
        assert_eq!(
            format("en", "{count, plural, other {#}}", &[("count", "three".into())]),
            Err(ArgumentError::WrongType {
                argument: "count".into(),
                expected: "number",
                received: "string"
            })
        );
        assert_eq!(
            format("en", "{mode, select, other {x}}", &[("mode", 3.into())]),
            Err(ArgumentError::WrongType {
                argument: "mode".into(),
                expected: "string or boolean",
                received: "number"
            })
        );
        assert_eq!(
            format("en", "{v, number}", &[("v", Value::Float(f64::NAN))]),
            Err(ArgumentError::NotFinite { argument: "v".into() })
        );
        assert_eq!(
            format("en", "{v, number}", &[("v", true.into())]),
            Err(ArgumentError::WrongType {
                argument: "v".into(),
                expected: "number",
                received: "boolean"
            })
        );
    }
}
