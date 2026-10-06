//! Locale identity: a validated BCP 47 tag and the parents it falls back through.

use std::fmt;

use icu_locale_core::LanguageIdentifier;

/// A validated locale: a BCP 47 language tag with an optional script, region, and variants.
///
/// Parsing canonicalizes the casing (`pt-br` is `pt-BR`, `zh-hant` is `zh-Hant`) and the
/// separator (`en_US` is `en-US`), so two spellings of one locale compare equal and name one
/// file. Extensions and private-use subtags are rejected: nothing in jess reads them, and a
/// locale that carries one would silently name a different file from the one it falls back to.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct Locale {
    id: LanguageIdentifier,
    name: Box<str>,
}

/// Why a string isn't a locale.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum LocaleError {
    /// The string is empty.
    Empty,
    /// The string has a subtag BCP 47 doesn't allow where it is: the backend's reason.
    Invalid {
        /// The string as given.
        input: String,
        /// The backend's reason, in its words.
        reason: String,
    },
    /// The string is a valid tag with an extension or private-use subtag, which jess doesn't take.
    Extension {
        /// The string as given.
        input: String,
    },
    /// The locale is valid and the backend couldn't build plural or number rules for it.
    Unsupported {
        /// The locale's canonical tag.
        input: String,
        /// The backend's reason, in its words.
        reason: String,
    },
}

impl fmt::Display for LocaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a locale can't be empty"),
            Self::Invalid { input, reason } => {
                write!(f, "'{input}' isn't a BCP 47 locale: {reason}")
            }
            Self::Extension { input } => write!(
                f,
                "'{input}' has an extension or private-use subtag; jess locales are a language, an optional script and region, and variants"
            ),
            Self::Unsupported { input, reason } => {
                write!(f, "no plural or number rules could be built for '{input}': {reason}")
            }
        }
    }
}

impl std::error::Error for LocaleError {}

impl Locale {
    /// Parses and canonicalizes a BCP 47 tag (`en`, `pt-BR`, `zh-Hant-TW`, `de-DE-1996`).
    ///
    /// # Errors
    ///
    /// Returns why the string isn't a locale jess takes: empty, not BCP 47, or carrying an extension.
    pub fn parse(input: &str) -> Result<Self, LocaleError> {
        if input.is_empty() {
            return Err(LocaleError::Empty);
        }
        if input.split(['-', '_']).skip(1).any(|subtag| subtag.len() == 1) {
            return Err(LocaleError::Extension { input: input.to_owned() });
        }
        // ICU4C, and so `OpenMW`, spell locales `en_US`; BCP 47 spells them `en-US`.
        let hyphenated = if input.contains('_') {
            std::borrow::Cow::Owned(input.replace('_', "-"))
        } else {
            std::borrow::Cow::Borrowed(input)
        };
        let id = LanguageIdentifier::try_from_str(&hyphenated).map_err(|error| LocaleError::Invalid {
            input: input.to_owned(),
            reason: error.to_string(),
        })?;
        Ok(Self::from_id(id))
    }

    fn from_id(id: LanguageIdentifier) -> Self {
        let name = id.to_string().into_boxed_str();
        Self { id, name }
    }

    /// The canonical tag (`pt-BR`).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.name
    }

    /// The language subtag (`pt` of `pt-BR`).
    #[must_use]
    pub fn language(&self) -> String {
        self.id.language.to_string()
    }

    /// The script subtag, if any (`Hant` of `zh-Hant-TW`).
    #[must_use]
    pub fn script(&self) -> Option<String> {
        self.id.script.map(|script| script.to_string())
    }

    /// The region subtag, if any (`BR` of `pt-BR`).
    #[must_use]
    pub fn region(&self) -> Option<String> {
        self.id.region.map(|region| region.to_string())
    }

    /// The variant subtags, in canonical order (`1996` of `de-DE-1996`).
    #[must_use]
    pub fn variants(&self) -> Vec<String> {
        self.id.variants.iter().map(ToString::to_string).collect()
    }

    /// The next less specific locale, or `None` for a bare language.
    ///
    /// The variants go first, all at once, then the region, then the script:
    /// `zh-Hant-TW-pinyin` → `zh-Hant-TW` → `zh-Hant` → `zh`. This is `OpenMW`'s order
    /// (`MessageBundles::setPreferredLocales` in `components/l10n/messagebundles.cpp` tries a
    /// locale, then it without its variant, then without its country) with one extension: `OpenMW`
    /// rebuilds the stripped locales from language and country alone, which drops a script along
    /// with the variant; jess keeps the script until the region is gone, so `zh-Hant-TW` reaches
    /// `zh-Hant` before `zh`. Locales without a script fall back identically.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let mut id = self.id.clone();
        if !id.variants.is_empty() {
            id.variants.clear();
        } else if id.region.is_some() {
            id.region = None;
        } else if id.script.is_some() {
            id.script = None;
        } else {
            return None;
        }
        Some(Self::from_id(id))
    }

    /// This locale and every parent, most specific first.
    pub fn chain(&self) -> Vec<Self> {
        let mut chain = vec![self.clone()];
        while let Some(parent) = chain.last().and_then(Self::parent) {
            chain.push(parent);
        }
        chain
    }

    /// The backend identifier, for building plural rules and number formatters.
    pub(crate) fn id(&self) -> &LanguageIdentifier {
        &self.id
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

impl std::str::FromStr for Locale {
    type Err = LocaleError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalizes_case_and_separators() {
        assert_eq!(Locale::parse("pt-br").unwrap().as_str(), "pt-BR");
        assert_eq!(Locale::parse("ZH_hant_tw").unwrap().as_str(), "zh-Hant-TW");
        assert_eq!(Locale::parse("EN").unwrap(), Locale::parse("en").unwrap());
    }

    #[test]
    fn rejects_garbage_and_extensions() {
        assert_eq!(Locale::parse(""), Err(LocaleError::Empty));
        assert!(matches!(Locale::parse("en-US-"), Err(LocaleError::Invalid { .. })));
        assert!(matches!(Locale::parse("x2"), Err(LocaleError::Invalid { .. })));
        assert!(matches!(Locale::parse("3"), Err(LocaleError::Invalid { .. })));
        assert!(matches!(Locale::parse("english"), Err(LocaleError::Invalid { .. })));
        assert!(matches!(Locale::parse("en-u-nu-arab"), Err(LocaleError::Extension { .. })));
        assert!(matches!(Locale::parse("en-x-private"), Err(LocaleError::Extension { .. })));
    }

    #[test]
    fn chain_strips_variants_then_region_then_script() {
        let chain = Locale::parse("zh-Hant-TW-pinyin").unwrap().chain();
        let names: Vec<&str> = chain.iter().map(Locale::as_str).collect();
        assert_eq!(names, ["zh-Hant-TW-pinyin", "zh-Hant-TW", "zh-Hant", "zh"]);
        let chain = Locale::parse("de-DE-1996").unwrap().chain();
        let names: Vec<&str> = chain.iter().map(Locale::as_str).collect();
        assert_eq!(names, ["de-DE-1996", "de-DE", "de"]);
        assert!(Locale::parse("en").unwrap().parent().is_none());
    }

    #[test]
    fn parts() {
        let locale = Locale::parse("de-Latn-DE-1996").unwrap();
        assert_eq!(locale.language(), "de");
        assert_eq!(locale.script().as_deref(), Some("Latn"));
        assert_eq!(locale.region().as_deref(), Some("DE"));
        assert_eq!(locale.variants(), ["1996"]);
    }
}
