//! Structured diagnostics: what went wrong, in jess's terms, and where it came from.

use std::fmt;

use crate::l10n::id::Generation;

/// How bad a diagnostic is.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum Severity {
    /// Worth knowing: a fallback locale supplied the text.
    Info,
    /// Probably a mistake that loses nothing: a bundle no provider has files for.
    Warning,
    /// A source or caller error: a file that doesn't parse, a pattern that doesn't compile, a
    /// message nothing defines.
    Error,
}

impl Severity {
    /// The severity as a lowercase word.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// What a diagnostic is about.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DiagnosticKind {
    /// A file is named for something that isn't a locale.
    InvalidLocale,
    /// A folder is named for something that can't be a bundle.
    InvalidBundleName,
    /// A provider couldn't list its files or read one.
    ProviderFailure,
    /// A bundle file isn't valid, or holds something other than id-to-pattern strings.
    BundleParseFailure,
    /// A pattern isn't valid `MessageFormat`.
    PatternParseFailure,
    /// One file defines the same message id twice.
    DuplicateMessage,
    /// One provider offers the same bundle and locale twice.
    ProviderCollision,
    /// A bundle was asked for and no provider has a file for it.
    EmptyBundle,
    /// No locale in the fallback chain defines the message.
    MissingMessage,
    /// A pattern names an argument the caller didn't pass.
    MissingArgument,
    /// An argument was passed with a type the pattern can't use.
    UnsupportedArgumentType,
    /// A locale other than the first preferred one supplied the text.
    FallbackUsed,
}

impl DiagnosticKind {
    /// The kind as a stable kebab-case code, for tools and for the Luau side.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidLocale => "invalid-locale",
            Self::InvalidBundleName => "invalid-bundle-name",
            Self::ProviderFailure => "provider-failure",
            Self::BundleParseFailure => "bundle-parse-failure",
            Self::PatternParseFailure => "pattern-parse-failure",
            Self::DuplicateMessage => "duplicate-message",
            Self::ProviderCollision => "provider-collision",
            Self::EmptyBundle => "empty-bundle",
            Self::MissingMessage => "missing-message",
            Self::MissingArgument => "missing-argument",
            Self::UnsupportedArgumentType => "unsupported-argument-type",
            Self::FallbackUsed => "fallback-used",
        }
    }
}

/// One structured diagnostic.
///
/// Every field that applies is filled; a GUI renders the fields, a log prints the `Display`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    /// How bad it is.
    pub severity: Severity,
    /// What it's about.
    pub kind: DiagnosticKind,
    /// The explanation, in a sentence.
    pub detail: String,
    /// The provider the source came from.
    pub provider: Option<String>,
    /// The file, as its provider names it.
    pub path: Option<String>,
    /// The 1-based line in the file.
    pub line: Option<u32>,
    /// The 1-based column in the line, in characters.
    pub column: Option<u32>,
    /// The bundle.
    pub bundle: Option<String>,
    /// The locale.
    pub locale: Option<String>,
    /// The message id.
    pub message: Option<String>,
    /// The registry's generation when the diagnostic was made.
    pub generation: Generation,
}

impl Diagnostic {
    pub(crate) fn new(severity: Severity, kind: DiagnosticKind, generation: Generation, detail: impl Into<String>) -> Self {
        Self {
            severity,
            kind,
            detail: detail.into(),
            provider: None,
            path: None,
            line: None,
            column: None,
            bundle: None,
            locale: None,
            message: None,
            generation,
        }
    }

    pub(crate) fn error(kind: DiagnosticKind, generation: Generation, detail: impl Into<String>) -> Self {
        Self::new(Severity::Error, kind, generation, detail)
    }

    pub(crate) fn provider(mut self, provider: &str) -> Self {
        self.provider = Some(provider.to_owned());
        self
    }

    pub(crate) fn path(mut self, path: &str) -> Self {
        self.path = Some(path.to_owned());
        self
    }

    pub(crate) fn at(mut self, line: u32, column: u32) -> Self {
        self.line = Some(line);
        self.column = Some(column);
        self
    }

    pub(crate) fn bundle(mut self, bundle: &str) -> Self {
        self.bundle = Some(bundle.to_owned());
        self
    }

    pub(crate) fn locale(mut self, locale: &str) -> Self {
        self.locale = Some(locale.to_owned());
        self
    }

    pub(crate) fn message(mut self, message: &str) -> Self {
        self.message = Some(message.to_owned());
        self
    }
}

impl fmt::Display for Diagnostic {
    /// `error[pattern-parse-failure] l10n/common/en.toml:3:9 (provider base): common/en
    /// 'file_count': at offset 8: …`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}[{}]", self.severity.as_str(), self.kind.code())?;
        if let Some(path) = &self.path {
            write!(f, " {path}")?;
            if let (Some(line), Some(column)) = (self.line, self.column) {
                write!(f, ":{line}:{column}")?;
            }
        }
        if let Some(provider) = &self.provider {
            write!(f, " (provider {provider})")?;
        }
        f.write_str(":")?;
        if let Some(bundle) = &self.bundle {
            write!(f, " {bundle}")?;
            if let Some(locale) = &self.locale {
                write!(f, "/{locale}")?;
            }
        }
        if let Some(message) = &self.message {
            write!(f, " '{message}':")?;
        } else if self.bundle.is_some() {
            f.write_str(":")?;
        }
        write!(f, " {}", self.detail)
    }
}
