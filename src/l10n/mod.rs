//! Localization: named bundles of ICU `MessageFormat` messages, resolved through a locale
//! fallback chain, from providers with a fixed precedence.
//!
//! The model is `OpenMW`'s (`components/l10n`): a *bundle* (`OpenMW`'s context) is a named group
//! of messages; each locale's messages live in `<bundle>/<locale>.toml`; a message is looked up
//! in the user's preferred locales, each followed by its less specific parents, then in the
//! bundle's fallback locale. Patterns are ICU `MessageFormat`: arguments, `plural` with exact
//! matches and CLDR categories, `selectordinal`, `select`, and locale-aware numbers.
//!
//! What jess adds to the model:
//!
//! - **Providers.** A [`Provider`] lists and reads bundle files: a folder
//!   ([`DirectoryProvider`]), a VFS ([`VfsProvider`] over a [`VfsSource`]), later a package.
//!   Providers registered later override earlier ones, message by message, and every message
//!   remembers the provider, file, and line it came from ([`Source`]).
//! - **Compile once.** [`Registry::reload`] parses every file and compiles every pattern of every
//!   locale, so a malformed message is a load diagnostic, not a surprise at the first render.
//!   Formatting walks a flat compiled pattern into the caller's `String` and allocates nothing.
//! - **Generations.** A bundle with any error isn't swapped in; its last good generation stays
//!   live. Locale changes and reloads raise the registry's [`Generation`], the one number a
//!   consumer watches to know its cached text is stale.
//! - **Loud failures.** A missing message is an error naming the locales tried, never an empty
//!   string and never the id dressed up as text. [`Diagnostic`] carries provider, path, line,
//!   bundle, locale, and message for a tool to render.
//!
//! ```
//! use jess::l10n::{Locale, MemoryFiles, Registry, Value, VfsProvider};
//!
//! let files = MemoryFiles::new();
//! files.insert("l10n/common/en.toml", "hello = 'Hello, {name}!'\nmode = '{mode, select, edit {Editing} other {Viewing}}'");
//! files.insert("l10n/common/de.toml", "hello = 'Hallo, {name}!'");
//!
//! let mut registry = Registry::new();
//! registry.add_provider(VfsProvider::new("base", "l10n", files));
//! let report = registry.reload();
//! assert!(report.is_clean(), "{:?}", report.diagnostics);
//!
//! registry.set_preferred_locales(&[Locale::parse("de-AT")?])?;
//! let common = registry.bundle("common")?;
//! let hello = registry.message(common, "hello");
//! let mode = registry.message(common, "mode");
//!
//! // de-AT falls back to de, which has the message.
//! assert_eq!(registry.format_to_string(hello, &[("name", Value::Str("Fargoth"))])?, "Hallo, Fargoth!");
//! // de doesn't have this one; the bundle's fallback locale, en, does.
//! assert_eq!(registry.format_to_string(mode, &[("mode", Value::Str("edit"))])?, "Editing");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! Nothing here knows about Morrowind, `OpenMW`'s file paths, Luau, or a GUI. The backend
//! (ICU4X's `icu_plurals` and `icu_decimal`, the `toml` parser) appears in no public type.

#![warn(missing_docs)]

mod diagnostic;
mod file;
mod format;
mod id;
mod locale;
mod pattern;
pub mod preferences;
mod provider;
mod registry;

pub use diagnostic::{Diagnostic, DiagnosticKind, Severity};
pub use format::{ArgumentError, Value};
pub use id::{BundleId, Generation, LocaleId, MessageId, ProviderId};
pub use locale::{Locale, LocaleError};
pub use pattern::{ArgumentKind, ArgumentSpec, PatternError, PatternErrorKind};
pub use provider::{DirectoryProvider, EXTENSION, MemoryFiles, Provider, ProviderError, Resource, VfsProvider, VfsSource};
pub use registry::{
    BundleNameError, DEFAULT_FALLBACK, FormatError, FormatErrorKind, MessageInfo, PatternFormatError, Registry, Report,
    Resolved, Source, validate_bundle_name,
};

/// The capabilities localization calls are gated on, named here so callers and grants agree.
///
/// `jess::l10n` enforces none of them: a native caller holding a [`Registry`] already has
/// everything. The script boundary does: `jess.l10n` checks `READ` on every call that resolves
/// text and `CONFIGURE` on every call that changes what everyone sees, through
/// `crate::runtime::require_capability`, whenever the script runs under a `ScriptDomain`. Outside
/// any domain (a plain `jess run`, a test) nothing holds authority and every call succeeds.
pub mod capability {
    /// Resolving messages: looking up bundles and formatting.
    pub const READ: &str = "l10n.read";
    /// Changing what everyone sees: the preferred locales, fallback locales, providers, reloads.
    pub const CONFIGURE: &str = "l10n.configure";
}

/// Compiles a pattern and reports whether it is valid `MessageFormat`, without a registry.
///
/// For tools that lint a pattern on its own. The error's offset is a byte offset into `pattern`.
///
/// # Errors
///
/// Returns the first thing wrong with the pattern and where it is.
pub fn check_pattern(pattern: &str) -> Result<Vec<ArgumentSpec>, PatternError> {
    let mut arena = pattern::Arena::new();
    let compiled = arena.compile(pattern)?;
    Ok(arena.arguments(compiled))
}
