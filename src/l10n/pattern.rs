//! ICU `MessageFormat` patterns, compiled once into a flat arena that formats without allocating.
//!
//! The grammar is ICU's `MessagePattern` (`ApostropheMode::DOUBLE_OPTIONAL`, the `MessageFormat`
//! default): literal text, `{name}`, `{name, number, style}`, `{name, plural, offset:n =n {…}
//! one {…} other {…}}`, `{name, selectordinal, …}`, `{name, select, key {…} other {…}}`, and
//! `#` for the offset number inside a plural case. Quoting follows ICU: `''` is one apostrophe;
//! an apostrophe before `{`, `}`, or (in a plural case) `#` starts literal text that the next
//! single apostrophe ends; any other apostrophe is literal.
//!
//! Every pattern of a bundle generation compiles into one [`Arena`]: parts, cases, text, and
//! argument slots in four vectors, so a bundle of a thousand messages is a handful of
//! allocations and a [`Pattern`] is two spans into it. A message's parts are contiguous, and so
//! are each case's, so formatting walks slices.
//!
//! Date, time, spellout, ordinal, duration, and choice arguments are rejected at compile time
//! with a diagnostic; none of them has a backend here yet.

use std::fmt;

use icu_plurals::PluralCategory;

/// The deepest a plural or select may nest. ICU has no limit; this keeps formatting's recursion
/// bounded by the pattern, not the attacker.
const MAX_DEPTH: usize = 16;

/// The compiled patterns of one bundle generation.
#[derive(Clone, Debug)]
pub struct Arena {
    pub(crate) parts: Vec<Part>,
    pub(crate) cases: Vec<Case>,
    pub(crate) text: String,
    pub(crate) slots: Vec<Slot>,
    /// Distinct number styles; index 0 is [`NumberStyle::DEFAULT`].
    pub(crate) styles: Vec<NumberStyle>,
    /// Parts of the messages being parsed, innermost last, drained into `parts` as each ends.
    scratch_parts: Vec<Part>,
    /// The same for cases.
    scratch_cases: Vec<Case>,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

/// One compiled pattern: a handle into an [`Arena`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pattern {
    /// The top-level parts.
    pub(crate) root: Span,
    /// The argument slots, in `Arena::slots`.
    pub(crate) slots: Span,
}

/// A contiguous run in one of the arenas.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Span {
    pub(crate) start: u32,
    pub(crate) len: u32,
}

impl Span {
    pub(crate) fn range(self) -> std::ops::Range<usize> {
        let start = self.start as usize;
        start..start + self.len as usize
    }
}

/// One piece of a message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    /// Literal text: a span of the text arena.
    Text(Span),
    /// `{name}`: the argument as text; numbers take the locale's default number format.
    Simple { slot: u16 },
    /// `{name, number, style}`.
    Number { slot: u16, style: u16 },
    /// `{name, plural, …}` or `{name, selectordinal, …}`: the cases, in a span of the case arena.
    Plural {
        slot: u16,
        ordinal: bool,
        offset: i32,
        cases: Span,
    },
    /// `{name, select, …}`.
    Select { slot: u16, cases: Span },
    /// `#` inside a plural case: the argument minus the offset, in the default number format.
    Pound { slot: u16, offset: i32 },
}

/// One case of a plural or select.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Case {
    pub(crate) selector: Selector,
    pub(crate) parts: Span,
}

/// What a case matches.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Selector {
    /// `=n`: the argument's exact value, before the offset.
    Exact(f64),
    /// A CLDR plural category keyword.
    Category(PluralCategory),
    /// A select keyword: a span of the text arena.
    Keyword(Span),
    /// `other`: the required last resort.
    Other,
}

/// An argument slot: the name (a span of the text arena) and how the pattern uses it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Slot {
    pub(crate) name: Span,
    pub(crate) kind: ArgumentKind,
}

/// An argument a pattern names, and how it uses it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ArgumentSpec {
    /// The name as written in the pattern.
    pub name: String,
    /// The strictest use the pattern makes of it.
    pub kind: ArgumentKind,
}

/// How a pattern uses an argument, which decides the value types it accepts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArgumentKind {
    /// `{name}` only: a string, number, or boolean.
    Any,
    /// A number argument, plural, or selectordinal: a number.
    Number,
    /// A select: a string or boolean.
    Select,
}

/// How a number is written.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NumberStyle {
    pub(crate) grouping: Grouping,
    /// The fewest fraction digits written; zeros pad up to it.
    pub(crate) min_fraction: u8,
    /// The most fraction digits written; the number is rounded half-even to it.
    pub(crate) max_fraction: u8,
    /// The power of ten the number is multiplied by first.
    pub(crate) scale: i8,
    /// Whether a percent sign follows the number.
    pub(crate) percent: bool,
}

impl NumberStyle {
    /// ICU's default number format, which `{name}`, `{name, number}`, and `#` use: up to three
    /// fraction digits (`DecimalFormat`'s default).
    pub(crate) const DEFAULT: Self = Self {
        grouping: Grouping::Auto,
        min_fraction: 0,
        max_fraction: 3,
        scale: 0,
        percent: false,
    };

    /// What a `::` skeleton starts from: up to six fraction digits (ICU's `NumberFormatter`
    /// default precision).
    const SKELETON: Self = Self {
        max_fraction: 6,
        ..Self::DEFAULT
    };
}

/// When grouping separators are written.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Grouping {
    /// The locale's usual grouping (`1,234` in `en`).
    #[default]
    Auto,
    /// Never (`1234`).
    Off,
    /// Only from five digits (`1234` but `12,345`).
    Min2,
}

/// A pattern that doesn't compile: where in the pattern (a byte offset) and why.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PatternError {
    /// The byte offset into the pattern where the problem was found.
    pub offset: usize,
    /// What's wrong.
    pub kind: PatternErrorKind,
}

/// What's wrong with a pattern.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PatternErrorKind {
    /// A `}` with no `{` open.
    UnmatchedBrace,
    /// The pattern ended inside an argument.
    UnterminatedArgument,
    /// `{` followed by no name.
    MissingArgumentName,
    /// An argument name with characters ICU doesn't allow.
    InvalidArgumentName(String),
    /// An argument type the grammar doesn't have.
    UnknownArgumentType(String),
    /// An argument type ICU has and jess doesn't format yet.
    UnsupportedArgumentType(String),
    /// A number style that isn't a supported skeleton.
    UnsupportedNumberStyle(String),
    /// A plural or select with no cases.
    MissingCases,
    /// A plural or select without an `other` case.
    MissingOther,
    /// The same selector twice in one plural or select.
    DuplicateSelector(String),
    /// A plural selector that is neither `=n` nor a CLDR category.
    InvalidSelector(String),
    /// `offset:` followed by something other than an integer.
    InvalidOffset(String),
    /// A case name with no `{…}` after it, or text where a case was expected.
    ExpectedCaseBody(String),
    /// More than 16 nested plurals and selects.
    TooDeep,
    /// The same name used as a number in one place and a select key in another.
    ConflictingUse(String),
    /// More distinct arguments than a pattern may name.
    TooManyArguments,
    /// A bundle whose compiled patterns pass 4 GiB.
    TooLarge,
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at offset {}: {}", self.offset, self.kind)
    }
}

impl std::error::Error for PatternError {}

impl fmt::Display for PatternErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnmatchedBrace => f.write_str("'}' without a matching '{' (quote it as '}' with apostrophes)"),
            Self::UnterminatedArgument => f.write_str("the pattern ends inside an argument; a '}' is missing"),
            Self::MissingArgumentName => f.write_str("an argument needs a name between the braces"),
            Self::InvalidArgumentName(name) => {
                write!(f, "'{name}' isn't an argument name (letters, digits, and '_' only)")
            }
            Self::UnknownArgumentType(kind) => write!(
                f,
                "'{kind}' isn't an argument type; use number, plural, selectordinal, or select"
            ),
            Self::UnsupportedArgumentType(kind) => write!(
                f,
                "'{kind}' arguments aren't formatted by jess yet; use number, plural, selectordinal, or select"
            ),
            Self::UnsupportedNumberStyle(style) => write!(
                f,
                "number style '{style}' isn't supported; use integer, percent, or a skeleton of :: group-off, group-min2, percent, scale/100, precision-integer, or .00 (fraction digits: 0 required, # optional)"
            ),
            Self::MissingCases => f.write_str("a plural or select needs at least an other {...} case"),
            Self::MissingOther => f.write_str("a plural or select needs an other {...} case"),
            Self::DuplicateSelector(selector) => write!(f, "case '{selector}' appears twice"),
            Self::InvalidSelector(selector) => write!(
                f,
                "'{selector}' isn't a plural case; use =n, zero, one, two, few, many, or other"
            ),
            Self::InvalidOffset(offset) => write!(f, "'offset:{offset}' isn't an integer offset"),
            Self::ExpectedCaseBody(text) => write!(f, "expected a case like other {{...}}, found '{text}'"),
            Self::TooDeep => write!(f, "plurals and selects nest deeper than {MAX_DEPTH}"),
            Self::ConflictingUse(name) => write!(
                f,
                "'{name}' is used both as a number and as a select key; one argument has one type"
            ),
            Self::TooManyArguments => f.write_str("a pattern names at most 65535 distinct arguments"),
            Self::TooLarge => f.write_str("the bundle's compiled patterns pass 4 GiB"),
        }
    }
}

impl Arena {
    /// An empty arena.
    pub(crate) fn new() -> Self {
        Self {
            parts: Vec::new(),
            cases: Vec::new(),
            text: String::new(),
            slots: Vec::new(),
            styles: vec![NumberStyle::DEFAULT],
            scratch_parts: Vec::new(),
            scratch_cases: Vec::new(),
        }
    }

    /// Compiles a pattern into the arena. On an error the arena is as it was.
    pub(crate) fn compile(&mut self, source: &str) -> Result<Pattern, PatternError> {
        let mark = (self.parts.len(), self.cases.len(), self.text.len(), self.slots.len());
        let result = Parser {
            source,
            pos: 0,
            arena: self,
            slots_start: mark.3,
        }
        .pattern();
        if result.is_err() {
            self.parts.truncate(mark.0);
            self.cases.truncate(mark.1);
            self.text.truncate(mark.2);
            self.slots.truncate(mark.3);
            self.scratch_parts.clear();
            self.scratch_cases.clear();
        }
        result
    }

    /// Frees the capacity loading left over.
    pub(crate) fn shrink_to_fit(&mut self) {
        self.parts.shrink_to_fit();
        self.cases.shrink_to_fit();
        self.text.shrink_to_fit();
        self.slots.shrink_to_fit();
        self.styles.shrink_to_fit();
        self.scratch_parts = Vec::new();
        self.scratch_cases = Vec::new();
    }

    /// The arguments a pattern names, by slot.
    pub(crate) fn arguments(&self, pattern: Pattern) -> Vec<ArgumentSpec> {
        self.slots[pattern.slots.range()]
            .iter()
            .map(|slot| ArgumentSpec {
                name: self.text(slot.name).to_owned(),
                kind: slot.kind,
            })
            .collect()
    }

    /// The text of a pattern that is nothing but text, or `None` if it has arguments.
    pub(crate) fn literal(&self, pattern: Pattern) -> Option<&str> {
        match self.parts[pattern.root.range()] {
            [] => Some(""),
            [Part::Text(text)] => Some(self.text(text)),
            _ => None,
        }
    }

    pub(crate) fn text(&self, span: Span) -> &str {
        &self.text[span.range()]
    }

    pub(crate) fn slot(&self, pattern: Pattern, slot: u16) -> &Slot {
        &self.slots[pattern.slots.start as usize + usize::from(slot)]
    }
}

fn span(start: usize, end: usize) -> Result<Span, PatternErrorKind> {
    let start = u32::try_from(start).map_err(|_| PatternErrorKind::TooLarge)?;
    let end = u32::try_from(end).map_err(|_| PatternErrorKind::TooLarge)?;
    Ok(Span { start, len: end - start })
}

/// The innermost plural a case body sits in, for `#`.
#[derive(Clone, Copy)]
struct Enclosing {
    slot: u16,
    offset: i32,
}

struct Parser<'s, 'a> {
    source: &'s str,
    pos: usize,
    arena: &'a mut Arena,
    /// Where this pattern's slots begin in the arena.
    slots_start: usize,
}

impl Parser<'_, '_> {
    fn pattern(mut self) -> Result<Pattern, PatternError> {
        let root = self.message(0, None)?;
        if self.pos < self.source.len() {
            return Err(self.error(PatternErrorKind::UnmatchedBrace));
        }
        let slots = span(self.slots_start, self.arena.slots.len()).map_err(|kind| self.error(kind))?;
        Ok(Pattern { root, slots })
    }

    fn error(&self, kind: PatternErrorKind) -> PatternError {
        PatternError { offset: self.pos, kind }
    }

    fn error_at(offset: usize, kind: PatternErrorKind) -> PatternError {
        PatternError { offset, kind }
    }

    fn rest(&self) -> &str {
        &self.source[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.bump();
            true
        } else {
            false
        }
    }

    /// Moves the scratch parts above `base` into the arena, where they become contiguous.
    fn commit_parts(&mut self, base: usize) -> Result<Span, PatternError> {
        let start = self.arena.parts.len();
        self.arena.parts.extend(self.arena.scratch_parts.drain(base..));
        span(start, self.arena.parts.len()).map_err(|kind| self.error(kind))
    }

    /// Ends the text run that began at `run_start` in the arena's text.
    fn flush_run(&mut self, run_start: usize) -> Result<(), PatternError> {
        if run_start < self.arena.text.len() {
            let text = span(run_start, self.arena.text.len()).map_err(|kind| self.error(kind))?;
            self.arena.scratch_parts.push(Part::Text(text));
        }
        Ok(())
    }

    fn intern_text(&mut self, text: &str) -> Result<Span, PatternError> {
        let start = self.arena.text.len();
        self.arena.text.push_str(text);
        span(start, self.arena.text.len()).map_err(|kind| self.error(kind))
    }

    /// Parses message text up to an unmatched `}` (a case body's end) or the end of the pattern.
    /// `enclosing` is the plural whose case this is, if any: where `#` is special.
    fn message(&mut self, depth: usize, enclosing: Option<Enclosing>) -> Result<Span, PatternError> {
        let base = self.arena.scratch_parts.len();
        let mut run_start = self.arena.text.len();
        while let Some(c) = self.peek() {
            match c {
                '}' => break,
                '{' => {
                    self.flush_run(run_start)?;
                    self.bump();
                    self.argument(depth)?;
                    run_start = self.arena.text.len();
                }
                '#' if enclosing.is_some() => {
                    self.flush_run(run_start)?;
                    self.bump();
                    let Enclosing { slot, offset } = enclosing.expect("checked");
                    self.arena.scratch_parts.push(Part::Pound { slot, offset });
                    run_start = self.arena.text.len();
                }
                '\'' => {
                    self.bump();
                    self.apostrophe(enclosing.is_some());
                }
                _ => {
                    self.bump();
                    self.arena.text.push(c);
                }
            }
        }
        self.flush_run(run_start)?;
        self.commit_parts(base)
    }

    /// The apostrophe was consumed; decides what it quotes, per ICU's `DOUBLE_OPTIONAL` mode.
    fn apostrophe(&mut self, in_plural: bool) {
        match self.peek() {
            Some('\'') => {
                self.bump();
                self.arena.text.push('\'');
            }
            Some(c) if c == '{' || c == '}' || (c == '#' && in_plural) => {
                // Quoted literal text: everything to the next single apostrophe; a doubled
                // apostrophe inside is one literal apostrophe and quoting goes on. Unterminated
                // quoting runs to the end of the pattern, as in ICU.
                loop {
                    match self.bump() {
                        None => return,
                        Some('\'') => {
                            if self.peek() == Some('\'') {
                                self.bump();
                                self.arena.text.push('\'');
                            } else {
                                return;
                            }
                        }
                        Some(c) => self.arena.text.push(c),
                    }
                }
            }
            _ => self.arena.text.push('\''),
        }
    }

    /// Parses an argument; the `{` was consumed.
    fn argument(&mut self, depth: usize) -> Result<(), PatternError> {
        self.skip_whitespace();
        let name_start = self.pos;
        while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
            self.bump();
        }
        let name_end = self.pos;
        if name_start == name_end {
            let bad: String = self.rest().chars().take_while(|c| !matches!(c, ',' | '}')).collect();
            let bad = bad.trim();
            return Err(if bad.is_empty() {
                self.error(PatternErrorKind::MissingArgumentName)
            } else {
                self.error(PatternErrorKind::InvalidArgumentName(bad.to_owned()))
            });
        }
        self.skip_whitespace();
        match self.bump() {
            Some('}') => {
                let slot = self.slot(name_start..name_end, ArgumentKind::Any)?;
                self.arena.scratch_parts.push(Part::Simple { slot });
                Ok(())
            }
            Some(',') => self.typed_argument(depth, name_start..name_end),
            None => Err(self.error(PatternErrorKind::UnterminatedArgument)),
            Some(_) => {
                self.pos -= 1;
                let bad: String = self.rest().chars().take_while(|c| !matches!(c, ',' | '}')).collect();
                Err(self.error(PatternErrorKind::InvalidArgumentName(format!(
                    "{}{}",
                    &self.source[name_start..name_end],
                    bad.trim_end()
                ))))
            }
        }
    }

    /// The `{name,` was consumed.
    fn typed_argument(&mut self, depth: usize, name: std::ops::Range<usize>) -> Result<(), PatternError> {
        self.skip_whitespace();
        let type_start = self.pos;
        while self.peek().is_some_and(char::is_alphanumeric) {
            self.bump();
        }
        let kind = self.source[type_start..self.pos].to_owned();
        self.skip_whitespace();
        let has_style = match self.bump() {
            Some(',') => true,
            Some('}') => false,
            None => return Err(self.error(PatternErrorKind::UnterminatedArgument)),
            Some(_) => return Err(Self::error_at(type_start, PatternErrorKind::UnknownArgumentType(kind))),
        };
        match kind.as_str() {
            "number" => {
                let style = if has_style { self.style_text()? } else { 0..0 };
                let style = Self::number_style(self.source[style].trim())
                    .map_err(|style| Self::error_at(type_start, PatternErrorKind::UnsupportedNumberStyle(style)))?;
                let style = self.style_index(style)?;
                let slot = self.slot(name, ArgumentKind::Number)?;
                self.arena.scratch_parts.push(Part::Number { slot, style });
                Ok(())
            }
            "plural" | "selectordinal" => {
                if !has_style {
                    return Err(self.error(PatternErrorKind::MissingCases));
                }
                if depth >= MAX_DEPTH {
                    return Err(self.error(PatternErrorKind::TooDeep));
                }
                let slot = self.slot(name, ArgumentKind::Number)?;
                let offset = self.offset()?;
                let cases = self.cases(depth, Some(Enclosing { slot, offset }), true)?;
                self.arena.scratch_parts.push(Part::Plural {
                    slot,
                    ordinal: kind == "selectordinal",
                    offset,
                    cases,
                });
                Ok(())
            }
            "select" => {
                if !has_style {
                    return Err(self.error(PatternErrorKind::MissingCases));
                }
                if depth >= MAX_DEPTH {
                    return Err(self.error(PatternErrorKind::TooDeep));
                }
                let slot = self.slot(name, ArgumentKind::Select)?;
                let cases = self.cases(depth, None, false)?;
                self.arena.scratch_parts.push(Part::Select { slot, cases });
                Ok(())
            }
            "date" | "time" | "spellout" | "ordinal" | "duration" | "choice" => {
                Err(Self::error_at(type_start, PatternErrorKind::UnsupportedArgumentType(kind)))
            }
            _ => Err(Self::error_at(type_start, PatternErrorKind::UnknownArgumentType(kind))),
        }
    }

    /// A simple style: the source range up to the argument's `}`. Braces may nest (ICU allows
    /// them in styles); apostrophes quote as in text.
    fn style_text(&mut self) -> Result<std::ops::Range<usize>, PatternError> {
        let start = self.pos;
        let mut depth = 0usize;
        loop {
            match self.bump() {
                None => return Err(self.error(PatternErrorKind::UnterminatedArgument)),
                Some('{') => depth += 1,
                Some('}') if depth == 0 => return Ok(start..self.pos - 1),
                Some('}') => depth -= 1,
                Some('\'') => {
                    // Skip quoted text so a '}' inside it doesn't end the style.
                    if self.peek() == Some('\'') {
                        self.bump();
                    } else if self.peek().is_some_and(|c| c == '{' || c == '}') {
                        while let Some(c) = self.bump() {
                            if c == '\'' && self.peek() != Some('\'') {
                                break;
                            }
                        }
                    }
                }
                Some(_) => {}
            }
        }
    }

    /// Parses a number style: empty, `integer`, `percent`, or a `::` number skeleton.
    ///
    /// The keywords are `DecimalFormat`'s styles: `integer` rounds to a whole number, `percent`
    /// multiplies by 100, rounds to a whole number, and adds the sign. A skeleton is ICU's
    /// `NumberFormatter` syntax, where `percent` only adds the sign and `scale/100` multiplies.
    fn number_style(style: &str) -> Result<NumberStyle, String> {
        match style {
            "" => return Ok(NumberStyle::DEFAULT),
            "integer" => {
                return Ok(NumberStyle {
                    max_fraction: 0,
                    ..NumberStyle::DEFAULT
                });
            }
            "percent" => {
                return Ok(NumberStyle {
                    max_fraction: 0,
                    scale: 2,
                    percent: true,
                    ..NumberStyle::DEFAULT
                });
            }
            _ => {}
        }
        let Some(skeleton) = style.strip_prefix("::") else {
            return Err(style.to_owned());
        };
        let mut result = NumberStyle::SKELETON;
        for token in skeleton.split_whitespace() {
            match token {
                "group-off" | ",_" => result.grouping = Grouping::Off,
                "group-min2" | ",?" => result.grouping = Grouping::Min2,
                "group-auto" => result.grouping = Grouping::Auto,
                "percent" | "%" => result.percent = true,
                "%x100" => {
                    result.percent = true;
                    result.scale = 2;
                }
                "precision-integer" => (result.min_fraction, result.max_fraction) = (0, 0),
                _ => {
                    if let Some(scale) = token.strip_prefix("scale/") {
                        result.scale = Self::power_of_ten(scale).ok_or_else(|| style.to_owned())?;
                        continue;
                    }
                    let Some(digits) = token.strip_prefix('.') else {
                        return Err(style.to_owned());
                    };
                    let required = digits.chars().take_while(|c| *c == '0').count();
                    let optional = digits[required..].chars().take_while(|c| *c == '#').count();
                    if required + optional != digits.len() || digits.len() > 20 {
                        return Err(style.to_owned());
                    }
                    #[allow(clippy::cast_possible_truncation)] // At most 20.
                    let (min, max) = (required as u8, (required + optional) as u8);
                    (result.min_fraction, result.max_fraction) = (min, max);
                }
            }
        }
        Ok(result)
    }

    /// The exponent of a `scale/` that is a power of ten (`100` is 2, `0.01` is -2); any other
    /// scale is unsupported.
    fn power_of_ten(scale: &str) -> Option<i8> {
        let zeros = |digits: &str| digits.bytes().all(|byte| byte == b'0').then_some(digits.len());
        if let Some(fraction) = scale.strip_prefix("0.") {
            let exponent = zeros(fraction.strip_suffix('1')?)? + 1;
            return i8::try_from(exponent)
                .ok()
                .filter(|exponent| *exponent <= 20)
                .map(|exponent| -exponent);
        }
        let exponent = zeros(scale.strip_prefix('1')?)?;
        i8::try_from(exponent).ok().filter(|exponent| *exponent <= 20)
    }

    fn style_index(&mut self, style: NumberStyle) -> Result<u16, PatternError> {
        let index = self.arena.styles.iter().position(|known| *known == style).unwrap_or_else(|| {
            self.arena.styles.push(style);
            self.arena.styles.len() - 1
        });
        u16::try_from(index).map_err(|_| self.error(PatternErrorKind::TooLarge))
    }

    /// An optional `offset:n` at the start of a plural's cases.
    fn offset(&mut self) -> Result<i32, PatternError> {
        self.skip_whitespace();
        if !self.rest().starts_with("offset:") {
            return Ok(0);
        }
        self.pos += "offset:".len();
        self.skip_whitespace();
        let start = self.pos;
        while self.peek().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+') {
            self.bump();
        }
        let text = &self.source[start..self.pos];
        text.parse()
            .map_err(|_| Self::error_at(start, PatternErrorKind::InvalidOffset(text.to_owned())))
    }

    /// The cases of a plural or select up to and including the closing `}`.
    fn cases(&mut self, depth: usize, enclosing: Option<Enclosing>, plural: bool) -> Result<Span, PatternError> {
        let base = self.arena.scratch_cases.len();
        let mut has_other = false;
        loop {
            self.skip_whitespace();
            let selector_start = self.pos;
            match self.peek() {
                None => return Err(self.error(PatternErrorKind::UnterminatedArgument)),
                Some('}') => {
                    self.bump();
                    break;
                }
                Some(_) => {}
            }
            while self.peek().is_some_and(|c| !c.is_whitespace() && c != '{' && c != '}') {
                self.bump();
            }
            let selector_end = self.pos;
            self.skip_whitespace();
            if selector_start == selector_end || !self.eat('{') {
                let found: String = self.source[selector_start..].chars().take(20).collect();
                return Err(Self::error_at(
                    selector_start,
                    PatternErrorKind::ExpectedCaseBody(found.trim_end().to_owned()),
                ));
            }
            let selector_text = &self.source[selector_start..selector_end];
            let selector = if selector_text == "other" {
                has_other = true;
                Selector::Other
            } else if plural {
                Self::plural_selector(selector_text).ok_or_else(|| {
                    Self::error_at(selector_start, PatternErrorKind::InvalidSelector(selector_text.to_owned()))
                })?
            } else {
                Selector::Keyword(self.intern_text(selector_text)?)
            };
            let duplicate = self.arena.scratch_cases[base..]
                .iter()
                .any(|case| match (case.selector, selector) {
                    (Selector::Keyword(known), Selector::Keyword(_)) => self.arena.text(known) == selector_text,
                    (known, selector) => known == selector,
                });
            if duplicate {
                return Err(Self::error_at(
                    selector_start,
                    PatternErrorKind::DuplicateSelector(selector_text.to_owned()),
                ));
            }
            let parts = self.message(depth + 1, enclosing)?;
            if !self.eat('}') {
                return Err(self.error(PatternErrorKind::UnterminatedArgument));
            }
            self.arena.scratch_cases.push(Case { selector, parts });
        }
        if self.arena.scratch_cases.len() == base {
            return Err(self.error(PatternErrorKind::MissingCases));
        }
        if !has_other {
            return Err(self.error(PatternErrorKind::MissingOther));
        }
        let start = self.arena.cases.len();
        self.arena.cases.extend(self.arena.scratch_cases.drain(base..));
        span(start, self.arena.cases.len()).map_err(|kind| self.error(kind))
    }

    fn plural_selector(text: &str) -> Option<Selector> {
        if let Some(exact) = text.strip_prefix('=') {
            return exact.parse().ok().map(Selector::Exact);
        }
        let category = match text {
            "zero" => PluralCategory::Zero,
            "one" => PluralCategory::One,
            "two" => PluralCategory::Two,
            "few" => PluralCategory::Few,
            "many" => PluralCategory::Many,
            _ => return None,
        };
        Some(Selector::Category(category))
    }

    /// The slot of an argument, recording the strictest kind it's used as. `name` is a range of
    /// the source.
    fn slot(&mut self, name: std::ops::Range<usize>, kind: ArgumentKind) -> Result<u16, PatternError> {
        let at = name.start;
        let name = &self.source[name];
        let known = self.arena.slots[self.slots_start..]
            .iter()
            .position(|slot| self.arena.text(slot.name) == name);
        if let Some(index) = known {
            let slot = &mut self.arena.slots[self.slots_start + index];
            match (slot.kind, kind) {
                (ArgumentKind::Number, ArgumentKind::Select) | (ArgumentKind::Select, ArgumentKind::Number) => {
                    return Err(Self::error_at(at, PatternErrorKind::ConflictingUse(name.to_owned())));
                }
                (ArgumentKind::Any, stricter) => slot.kind = stricter,
                _ => {}
            }
            return Ok(u16::try_from(index).expect("checked on insert"));
        }
        let index = u16::try_from(self.arena.slots.len() - self.slots_start)
            .map_err(|_| self.error(PatternErrorKind::TooManyArguments))?;
        let name = self.intern_text(name)?;
        self.arena.slots.push(Slot { name, kind });
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(source: &str) -> (Arena, Pattern) {
        let mut arena = Arena::new();
        let pattern = arena.compile(source).unwrap();
        (arena, pattern)
    }

    fn parts(source: &str) -> Vec<Part> {
        let (arena, pattern) = compile(source);
        arena.parts[pattern.root.range()].to_vec()
    }

    #[test]
    fn plain_text_is_one_part() {
        let (arena, pattern) = compile("Open File");
        assert_eq!(arena.literal(pattern), Some("Open File"));
        assert_eq!(arena.parts.len(), 1);
        assert_eq!(arena.arguments(pattern), []);
        let (arena, empty) = compile("");
        assert_eq!(arena.literal(empty), Some(""));
    }

    #[test]
    fn simple_arguments_get_slots() {
        let (arena, pattern) = compile("{greeting}, {name}! {greeting}");
        let arguments = arena.arguments(pattern);
        assert_eq!(arguments.len(), 2);
        assert_eq!(arguments[0].name, "greeting");
        assert_eq!(arguments[1].kind, ArgumentKind::Any);
        assert_eq!(arena.literal(pattern), None);
        let root = &arena.parts[pattern.root.range()];
        assert!(matches!(root[0], Part::Simple { slot: 0 }));
        assert!(matches!(root[1], Part::Text(text) if arena.text(text) == ", "));
        assert!(matches!(root[2], Part::Simple { slot: 1 }));
        assert!(matches!(root[3], Part::Text(text) if arena.text(text) == "! "));
        assert!(matches!(root[4], Part::Simple { slot: 0 }));
    }

    #[test]
    fn patterns_share_one_arena_and_keep_their_own_slots() {
        let mut arena = Arena::new();
        let first = arena.compile("{a} and {b}").unwrap();
        let second = arena.compile("{b} then {c, number}").unwrap();
        assert_eq!(
            arena.arguments(first).iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(
            arena.arguments(second).iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["b", "c"]
        );
        assert_eq!(arena.slot(second, 0).kind, ArgumentKind::Any);
        assert_eq!(arena.slot(second, 1).kind, ArgumentKind::Number);
        assert_eq!(arena.parts.len(), 6);
    }

    #[test]
    fn a_failed_compile_leaves_the_arena_as_it_was() {
        let mut arena = Arena::new();
        let good = arena.compile("{n, plural, one {# thing} other {# things}}").unwrap();
        let sizes = (arena.parts.len(), arena.cases.len(), arena.text.len(), arena.slots.len());
        assert!(arena.compile("{n, plural, one {# thing} other {# things} more {").is_err());
        assert_eq!(
            sizes,
            (arena.parts.len(), arena.cases.len(), arena.text.len(), arena.slots.len())
        );
        assert!(arena.scratch_parts.is_empty() && arena.scratch_cases.is_empty());
        let after = arena.compile("{m} more").unwrap();
        assert_eq!(arena.arguments(good)[0].name, "n");
        assert_eq!(arena.arguments(after)[0].name, "m");
    }

    #[test]
    fn apostrophes_follow_icu() {
        let text = |source: &str| {
            let (arena, pattern) = compile(source);
            arena.literal(pattern).unwrap().to_owned()
        };
        assert_eq!(text("don't"), "don't");
        assert_eq!(text("it''s"), "it's");
        assert_eq!(text("'{'literal'}'"), "{literal}");
        assert_eq!(text("'{one''s}'"), "{one's}");
        assert_eq!(text("'{unterminated"), "{unterminated");
        assert_eq!(text("a '# b"), "a '# b");
        let (arena, pattern) = compile("{n, plural, other {'#' # '{'}}");
        let case = arena.cases[0];
        let body = &arena.parts[case.parts.range()];
        assert!(matches!(body[0], Part::Text(text) if arena.text(text) == "# "));
        assert_eq!(body[1], Part::Pound { slot: 0, offset: 0 });
        assert!(matches!(body[2], Part::Text(text) if arena.text(text) == " {"));
        assert_eq!(arena.parts[pattern.root.range()].len(), 1);
    }

    #[test]
    fn plural_cases_and_offset() {
        let (arena, pattern) =
            compile("{count, plural, offset:1 =0 {nobody} one {you and # other} other {you and # others}}");
        let root = &arena.parts[pattern.root.range()];
        let Part::Plural {
            slot: 0,
            ordinal: false,
            offset: 1,
            cases,
        } = root[0]
        else {
            panic!("{root:?}");
        };
        let cases = &arena.cases[cases.range()];
        assert_eq!(cases[0].selector, Selector::Exact(0.0));
        assert_eq!(cases[1].selector, Selector::Category(PluralCategory::One));
        assert_eq!(cases[2].selector, Selector::Other);
        assert!(arena.parts[cases[1].parts.range()].contains(&Part::Pound { slot: 0, offset: 1 }));
        assert_eq!(arena.arguments(pattern)[0].kind, ArgumentKind::Number);
    }

    #[test]
    fn select_and_nesting() {
        let (arena, pattern) = compile(
            "{gender, select, female {{n, plural, one {she has # item} other {she has # items}}} other {{n, plural, one {they have # item} other {they have # items}}}}",
        );
        let arguments = arena.arguments(pattern);
        assert_eq!(arguments[0].kind, ArgumentKind::Select);
        assert_eq!(arguments[1].kind, ArgumentKind::Number);
        assert_eq!(arena.cases.len(), 6);
        // Each case's parts are contiguous, and the nested plurals' cases come before the select's.
        let Part::Select { cases, .. } = arena.parts[pattern.root.range()][0] else {
            panic!()
        };
        assert_eq!(cases.start, 4);
        let (arena, pattern) = compile("{n, plural, other {{g, select, other {#}}}}");
        assert_eq!(&arena.text[..1], "n");
        let Part::Plural { cases, .. } = arena.parts[pattern.root.range()][0] else {
            panic!()
        };
        let Part::Select { cases: inner, .. } = arena.parts[arena.cases[cases.range()][0].parts.range()][0] else {
            panic!()
        };
        let body = &arena.parts[arena.cases[inner.range()][0].parts.range()];
        assert!(
            matches!(body, [Part::Text(text)] if arena.text(*text) == "#"),
            "'#' is literal in a select, even inside a plural: {body:?}"
        );
    }

    #[test]
    fn number_styles() {
        assert_eq!(parts("{v, number}"), [Part::Number { slot: 0, style: 0 }]);
        let (arena, _) = compile("{v, number, :: group-off .00} {w, number, percent} {x, number, integer}");
        assert_eq!(
            arena.styles[1],
            NumberStyle {
                grouping: Grouping::Off,
                min_fraction: 2,
                max_fraction: 2,
                scale: 0,
                percent: false
            }
        );
        assert_eq!(
            arena.styles[2],
            NumberStyle {
                max_fraction: 0,
                scale: 2,
                percent: true,
                ..NumberStyle::DEFAULT
            }
        );
        assert_eq!(arena.styles[3].max_fraction, 0);
        let style = |style: &str| Parser::number_style(style).unwrap();
        assert_eq!((style("::.0#").min_fraction, style("::.0#").max_fraction), (1, 2));
        assert_eq!(
            style("::").max_fraction,
            6,
            "a skeleton's default precision is ICU's six digits"
        );
        assert_eq!((style(":: percent").percent, style(":: percent").scale), (true, 0));
        assert_eq!(style(":: percent scale/100"), style(":: %x100"));
        assert_eq!(style(":: scale/0.001").scale, -3);
        assert_eq!(style(":: ,_").grouping, Grouping::Off);
        assert_eq!(style(":: precision-integer").max_fraction, 0);
        for unsupported in ["#,##0.00", "::currency/EUR", ":: scale/3", ":: scale/", "::.0x", "currency"] {
            assert!(Parser::number_style(unsupported).is_err(), "{unsupported}");
        }
        // Styles are shared across a bundle's patterns.
        let mut arena = Arena::new();
        arena.compile("{a, number, :: group-off}").unwrap();
        arena.compile("{b, number, :: group-off}").unwrap();
        assert_eq!(arena.styles.len(), 2);
    }

    #[test]
    fn errors_are_placed() {
        let error = |source: &str| Arena::new().compile(source).unwrap_err();
        assert_eq!(error("a } b").kind, PatternErrorKind::UnmatchedBrace);
        assert_eq!(error("a {b").kind, PatternErrorKind::UnterminatedArgument);
        assert_eq!(error("{}").kind, PatternErrorKind::MissingArgumentName);
        assert_eq!(error("{a-b}").kind, PatternErrorKind::InvalidArgumentName("a-b".into()));
        assert_eq!(
            error("{a, bogus}").kind,
            PatternErrorKind::UnknownArgumentType("bogus".into())
        );
        assert_eq!(
            error("{a, date}").kind,
            PatternErrorKind::UnsupportedArgumentType("date".into())
        );
        assert_eq!(
            error("{a, date, short}").kind,
            PatternErrorKind::UnsupportedArgumentType("date".into())
        );
        assert_eq!(error("{a, plural}").kind, PatternErrorKind::MissingCases);
        assert_eq!(error("{a, plural, one {x}}").kind, PatternErrorKind::MissingOther);
        assert_eq!(
            error("{a, plural, one {x} one {y} other {z}}").kind,
            PatternErrorKind::DuplicateSelector("one".into())
        );
        assert_eq!(
            error("{a, select, x {1} x {2} other {z}}").kind,
            PatternErrorKind::DuplicateSelector("x".into())
        );
        assert_eq!(
            error("{a, plural, some {x} other {z}}").kind,
            PatternErrorKind::InvalidSelector("some".into())
        );
        assert_eq!(
            error("{a, plural, offset:x other {z}}").kind,
            PatternErrorKind::InvalidOffset(String::new())
        );
        assert_eq!(
            error("{a, select, x y other {z}}").kind,
            PatternErrorKind::ExpectedCaseBody("x y other {z}}".into())
        );
        assert_eq!(
            error("{a, number, bogus}").kind,
            PatternErrorKind::UnsupportedNumberStyle("bogus".into())
        );
        assert_eq!(
            error("{a, number} {a, select, other {x}}").kind,
            PatternErrorKind::ConflictingUse("a".into())
        );
        assert_eq!(
            error("{a, plural, one {x} other {z}").kind,
            PatternErrorKind::UnterminatedArgument
        );
        let mut deep = String::new();
        for _ in 0..17 {
            deep.push_str("{n, plural, other {");
        }
        assert_eq!(error(&deep).kind, PatternErrorKind::TooDeep);
        let placed = error("hello {a, bogus}");
        assert_eq!(placed.offset, 10);
    }
}
