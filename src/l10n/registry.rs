//! The registry: providers, bundles, the locale chain, and generations.
//!
//! A reload reads every provider, compiles every pattern once, and swaps a bundle in only if all
//! of it compiled. Between reloads formatting is table lookups: a [`MessageId`] indexes its
//! bundle's resolution table, which names the compiled pattern the current locale chain picks.
//! A locale change rebuilds those tables and compiles nothing.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use crate::l10n::diagnostic::{Diagnostic, DiagnosticKind, Severity};
use crate::l10n::file;
use crate::l10n::format::{ArgumentError, LocaleRules, Value};
use crate::l10n::id::{BundleId, Generation, LocaleId, MessageId, ProviderId};
use crate::l10n::locale::{Locale, LocaleError};
use crate::l10n::pattern::{Arena, ArgumentSpec, Pattern, PatternError};
use crate::l10n::provider::{Provider, Resource};

/// The fallback locale of a bundle nobody declared one for. `OpenMW`'s default, too:
/// `Manager::getContext(contextName, fallbackLocale = "en")` in `components/l10n/manager.hpp`.
pub const DEFAULT_FALLBACK: &str = "en";

/// A resolution table slot no locale fills.
const NONE: u32 = u32::MAX;

/// Where a message's text came from.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Source {
    /// The provider that supplied the winning definition.
    pub provider: ProviderId,
    /// The file, as the provider names it.
    pub path: Arc<str>,
    /// The 1-based line of the message id in the file.
    pub line: u32,
    /// The 1-based column of the message id in the line.
    pub column: u32,
}

/// What a lookup found: the message as one locale defines it.
#[derive(Clone, Debug)]
pub struct MessageInfo<'a> {
    /// The locale whose file defines the text.
    pub locale: LocaleId,
    /// The file and provider the text came from.
    pub source: &'a Source,
    /// The arguments the pattern names and how it uses each.
    pub arguments: Vec<ArgumentSpec>,
}

/// How a formatted message was resolved.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Resolved {
    /// The locale that supplied the pattern, and whose plural and number rules formatted it.
    pub locale: LocaleId,
    /// Whether that is a locale other than the first preferred one.
    pub fallback: bool,
}

/// What a reload did.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Report {
    /// The registry's generation after the reload: raised if any bundle was swapped in.
    pub generation: Generation,
    /// Everything found wrong, in a deterministic order: by bundle, locale, provider, line.
    pub diagnostics: Vec<Diagnostic>,
    /// The bundles whose new generation is now live.
    pub updated: Vec<BundleId>,
    /// The bundles that failed to load and kept their last good generation.
    pub kept: Vec<BundleId>,
}

impl Report {
    /// Whether the reload found no errors. Warnings don't count.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }
}

/// A name that can't be a bundle's.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BundleNameError {
    /// The name as given.
    pub name: String,
}

impl fmt::Display for BundleNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "'{}' isn't a bundle name: use letters, digits, '_', and '-', in segments separated by '.'",
            self.name.escape_debug()
        )
    }
}

impl std::error::Error for BundleNameError {}

/// Checks a bundle name: non-empty segments of ASCII letters, digits, `_`, and `-`, joined by
/// `.` (`common`, `jess.common`, `s3maphore.playlist-editor`).
///
/// `OpenMW` allows letters, digits, and `_` (`Manager::getContext` in
/// `components/l10n/manager.cpp`); jess adds `.` and `-` so a package can namespace its bundles.
/// Either way a name is one safe path segment.
///
/// # Errors
///
/// Returns the name when it has an empty segment or a character outside the set.
pub fn validate_bundle_name(name: &str) -> Result<(), BundleNameError> {
    let segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    };
    if name.split('.').all(segment) {
        Ok(())
    } else {
        Err(BundleNameError { name: name.to_owned() })
    }
}

/// Why a message didn't format.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FormatError {
    /// The bundle's name, or the id's `Display` when the id isn't this registry's.
    pub bundle: Box<str>,
    /// The message's name, or the id's `Display` when the id isn't this registry's.
    pub message: Box<str>,
    /// What went wrong.
    pub kind: FormatErrorKind,
}

/// What went wrong formatting a message.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum FormatErrorKind {
    /// The id names a bundle or message this registry never allocated.
    UnknownMessage,
    /// No locale in the chain defines the message.
    MissingMessage {
        /// The locales tried, in order: the preferred chain, then the bundle's fallback.
        tried: Vec<String>,
    },
    /// The message exists and an argument is missing or of the wrong type.
    Argument {
        /// The locale whose pattern was being formatted.
        locale: String,
        /// What's wrong with the argument.
        error: ArgumentError,
    },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            FormatErrorKind::UnknownMessage => {
                write!(f, "{} isn't a message of this registry", self.message)
            }
            FormatErrorKind::MissingMessage { tried } => write!(
                f,
                "bundle '{}' has no message '{}' in {}",
                self.bundle,
                self.message,
                if tried.is_empty() {
                    "any locale".to_owned()
                } else {
                    tried.join(", ")
                }
            ),
            FormatErrorKind::Argument { locale, error } => {
                write!(f, "bundle '{}', message '{}' ({locale}): {error}", self.bundle, self.message)
            }
        }
    }
}

impl std::error::Error for FormatError {}

/// Why [`Registry::format_pattern`] didn't format.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PatternFormatError {
    /// The locale id isn't this registry's.
    UnknownLocale,
    /// The pattern doesn't compile.
    Pattern(PatternError),
    /// An argument is missing or of the wrong type.
    Argument(ArgumentError),
}

impl fmt::Display for PatternFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownLocale => f.write_str("the locale isn't this registry's"),
            Self::Pattern(error) => write!(f, "the pattern doesn't compile: {error}"),
            Self::Argument(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for PatternFormatError {}

impl FormatError {
    /// The error as a structured diagnostic.
    #[must_use]
    pub fn diagnostic(&self, generation: Generation) -> Diagnostic {
        let (kind, locale) = match &self.kind {
            FormatErrorKind::UnknownMessage | FormatErrorKind::MissingMessage { .. } => {
                (DiagnosticKind::MissingMessage, None)
            }
            FormatErrorKind::Argument { locale, error } => (
                match error {
                    ArgumentError::Missing { .. } => DiagnosticKind::MissingArgument,
                    ArgumentError::WrongType { .. } | ArgumentError::NotFinite { .. } => {
                        DiagnosticKind::UnsupportedArgumentType
                    }
                },
                Some(locale.as_str()),
            ),
        };
        let mut diagnostic = Diagnostic::error(kind, generation, self.to_string())
            .bundle(&self.bundle)
            .message(&self.message);
        if let Some(locale) = locale {
            diagnostic = diagnostic.locale(locale);
        }
        diagnostic
    }
}

struct LocaleEntry {
    locale: Locale,
    rules: LocaleRules,
}

/// One compiled message in one locale.
struct Entry {
    locale: LocaleId,
    pattern: Pattern,
    source: Source,
}

/// A bundle's compiled messages: one generation of it.
#[derive(Default)]
struct Loaded {
    /// Per locale the bundle has, a table from message index to entry index.
    locales: Vec<(LocaleId, Vec<u32>)>,
    entries: Vec<Entry>,
    /// Every entry's compiled pattern.
    arena: Arena,
}

struct Bundle {
    name: Box<str>,
    fallback: LocaleId,
    names: Vec<Box<str>>,
    name_ids: HashMap<Box<str>, u32>,
    /// The last generation that loaded without an error, or the first one, errors and all.
    loaded: Loaded,
    /// Whether `loaded` came from a load at all.
    has_loaded: bool,
    /// Message index to the entry the locale chain picks.
    resolved: Vec<u32>,
    generation: Generation,
}

impl Bundle {
    fn intern(&mut self, name: &str) -> u32 {
        if let Some(index) = self.name_ids.get(name) {
            return *index;
        }
        let index = u32::try_from(self.names.len()).expect("fewer than 2^32 messages in a bundle");
        self.names.push(name.into());
        self.name_ids.insert(name.into(), index);
        index
    }

    /// Rebuilds the resolution table: for each message, the first locale of the chain, then the
    /// fallback, that defines it. This is the lookup order of `MessageBundles::formatMessage` in
    /// `OpenMW`'s `components/l10n/messagebundles.cpp`, done once per generation instead of once
    /// per call.
    fn resolve(&mut self, chain: &[LocaleId]) {
        self.resolved.clear();
        self.resolved.resize(self.names.len(), NONE);
        for locale in chain.iter().chain(std::iter::once(&self.fallback)) {
            let Some((_, table)) = self.loaded.locales.iter().find(|(known, _)| known == locale) else {
                continue;
            };
            for (slot, entry) in self.resolved.iter_mut().zip(table) {
                if *slot == NONE {
                    *slot = *entry;
                }
            }
        }
    }
}

/// The localization service: providers in, formatted text out.
///
/// ```
/// use jess::l10n::{Locale, MemoryFiles, Registry, Value, VfsProvider};
///
/// let files = MemoryFiles::new();
/// files.insert("l10n/common/en.toml", "file_count = '{count, plural, one {# file} other {# files}}'");
/// files.insert("l10n/common/pl.toml", "file_count = '{count, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}'");
///
/// let mut registry = Registry::new();
/// registry.add_provider(VfsProvider::new("base", "l10n", files));
/// assert!(registry.reload().is_clean());
///
/// let common = registry.bundle("common")?;
/// let file_count = registry.message(common, "file_count");
/// let mut text = String::new();
/// registry.format(file_count, &[("count", Value::Int(5))], &mut text)?;
/// assert_eq!(text, "5 files");
///
/// registry.set_preferred_locales(&[Locale::parse("pl-PL")?])?;
/// text.clear();
/// registry.format(file_count, &[("count", Value::Int(5))], &mut text)?;
/// assert_eq!(text, "5 plików");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Formatting takes `&self` and the registry is `Send + Sync`: share it behind an `RwLock` (or
/// an `Arc` swapped whole) and any number of threads format at once. Changing it takes `&mut
/// self`. Nothing here is global; a process may hold as many registries as it has applications.
pub struct Registry {
    providers: Vec<Box<dyn Provider>>,
    locales: Vec<LocaleEntry>,
    locale_ids: HashMap<Box<str>, LocaleId>,
    bundles: Vec<Bundle>,
    bundle_ids: HashMap<Box<str>, BundleId>,
    /// The preferred locales as they were set.
    preferred: Vec<LocaleId>,
    /// The preferred locales, each followed by its parents, without repeats.
    chain: Vec<LocaleId>,
    default_fallback: LocaleId,
    generation: Generation,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field(
                "providers",
                &self.providers.iter().map(|provider| provider.name()).collect::<Vec<_>>(),
            )
            .field(
                "bundles",
                &self.bundles.iter().map(|bundle| &*bundle.name).collect::<Vec<_>>(),
            )
            .field(
                "preferred",
                &self
                    .preferred
                    .iter()
                    .map(|locale| self.locale_name(*locale))
                    .collect::<Vec<_>>(),
            )
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// An empty registry: no providers, no preferred locales, and [`DEFAULT_FALLBACK`] as the
    /// fallback locale of bundles that don't declare their own.
    ///
    /// # Panics
    ///
    /// If the backend's compiled CLDR data has no rules for English, which it always has.
    #[must_use]
    pub fn new() -> Self {
        let mut registry = Self {
            providers: Vec::new(),
            locales: Vec::new(),
            locale_ids: HashMap::new(),
            bundles: Vec::new(),
            bundle_ids: HashMap::new(),
            preferred: Vec::new(),
            chain: Vec::new(),
            default_fallback: LocaleId(0),
            generation: Generation::default(),
        };
        registry.default_fallback = registry.locale(DEFAULT_FALLBACK).expect("the compiled CLDR data has English");
        registry
    }

    /// The registry's generation: it rises whenever what formatting returns may have changed.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    // Providers.

    /// Registers a provider. Providers registered later take precedence: where two define the
    /// same message of the same bundle and locale, the later one's text wins. Call
    /// [`reload`](Self::reload) to read it.
    ///
    /// # Panics
    ///
    /// After 2^32 providers.
    pub fn add_provider(&mut self, provider: impl Provider + 'static) -> ProviderId {
        let id = ProviderId(u32::try_from(self.providers.len()).expect("fewer than 2^32 providers"));
        self.providers.push(Box::new(provider));
        id
    }

    /// The providers, lowest precedence first.
    pub fn providers(&self) -> impl Iterator<Item = (ProviderId, &str)> {
        (0..)
            .map(ProviderId)
            .zip(self.providers.iter().map(|provider| provider.name()))
    }

    /// A provider's name.
    #[must_use]
    pub fn provider_name(&self, provider: ProviderId) -> Option<&str> {
        self.providers.get(provider.0 as usize).map(|provider| provider.name())
    }

    // Locales.

    /// Parses a locale tag and interns it.
    ///
    /// # Errors
    ///
    /// Returns the parse error for a tag that isn't a locale, or the backend's when no rules can be built for it.
    pub fn locale(&mut self, tag: &str) -> Result<LocaleId, LocaleError> {
        self.intern_locale(&Locale::parse(tag)?)
    }

    /// Interns a locale, building its plural rules and number formatters the first time.
    ///
    /// # Panics
    ///
    /// After 2^32 locales.
    ///
    /// # Errors
    ///
    /// Returns the backend's error when no plural or number rules can be built for the locale.
    pub fn intern_locale(&mut self, locale: &Locale) -> Result<LocaleId, LocaleError> {
        if let Some(id) = self.locale_ids.get(locale.as_str()) {
            return Ok(*id);
        }
        let rules = LocaleRules::new(locale).map_err(|reason| LocaleError::Unsupported {
            input: locale.as_str().to_owned(),
            reason,
        })?;
        let id = LocaleId(u32::try_from(self.locales.len()).expect("fewer than 2^32 locales"));
        self.locales.push(LocaleEntry {
            locale: locale.clone(),
            rules,
        });
        self.locale_ids.insert(locale.as_str().into(), id);
        Ok(id)
    }

    /// The id of a locale, if the registry has seen it.
    #[must_use]
    pub fn locale_id(&self, locale: &Locale) -> Option<LocaleId> {
        self.locale_ids.get(locale.as_str()).copied()
    }

    /// The locale an id names.
    ///
    /// # Panics
    ///
    /// If the id is another registry's.
    #[must_use]
    pub fn locale_of(&self, id: LocaleId) -> &Locale {
        &self.locales[id.0 as usize].locale
    }

    /// The canonical tag of the locale an id names.
    ///
    /// # Panics
    ///
    /// If the id is another registry's.
    #[must_use]
    pub fn locale_name(&self, id: LocaleId) -> &str {
        self.locale_of(id).as_str()
    }

    /// Sets the locales the user prefers, most wanted first, and returns the new generation.
    ///
    /// Where the list comes from (an explicit override, a profile setting, the system) is the
    /// caller's policy; [`preferences`](crate::l10n::preferences) composes the usual sources. Each
    /// locale is tried, then its parents ([`Locale::parent`]), before the next one; a bundle's
    /// fallback locale comes after all of them. No pattern is recompiled.
    ///
    /// # Errors
    ///
    /// Returns the backend's error when a locale, or one of its parents, has no plural or number rules.
    pub fn set_preferred_locales(&mut self, locales: &[Locale]) -> Result<Generation, LocaleError> {
        let mut preferred = Vec::with_capacity(locales.len());
        let mut chain = Vec::new();
        for locale in locales {
            let id = self.intern_locale(locale)?;
            if !preferred.contains(&id) {
                preferred.push(id);
            }
            for link in locale.chain() {
                let id = self.intern_locale(&link)?;
                if !chain.contains(&id) {
                    chain.push(id);
                }
            }
        }
        self.preferred = preferred;
        self.chain = chain;
        for bundle in &mut self.bundles {
            bundle.resolve(&self.chain);
        }
        self.generation = self.generation.next();
        Ok(self.generation)
    }

    /// The preferred locales, as they were set.
    #[must_use]
    pub fn preferred_locales(&self) -> &[LocaleId] {
        &self.preferred
    }

    /// The locales a message of `bundle` is looked up in, in order: every preferred locale
    /// followed by its parents, then the bundle's fallback locale.
    #[must_use]
    pub fn locale_chain(&self, bundle: BundleId) -> Vec<LocaleId> {
        let mut chain = self.chain.clone();
        if let Some(bundle) = self.bundles.get(bundle.0 as usize)
            && !chain.contains(&bundle.fallback)
        {
            chain.push(bundle.fallback);
        }
        chain
    }

    // Bundles.

    /// The bundle named `name`, declared now if it wasn't: it gets the default fallback locale
    /// and whatever the providers had for it at the last reload.
    ///
    /// # Errors
    ///
    /// Returns the name when it isn't a bundle name ([`validate_bundle_name`]).
    pub fn bundle(&mut self, name: &str) -> Result<BundleId, BundleNameError> {
        validate_bundle_name(name)?;
        Ok(self.declare(name))
    }

    fn declare(&mut self, name: &str) -> BundleId {
        if let Some(id) = self.bundle_ids.get(name) {
            return *id;
        }
        let id = BundleId(u32::try_from(self.bundles.len()).expect("fewer than 2^32 bundles"));
        self.bundles.push(Bundle {
            name: name.into(),
            fallback: self.default_fallback,
            names: Vec::new(),
            name_ids: HashMap::new(),
            loaded: Loaded::default(),
            has_loaded: false,
            resolved: Vec::new(),
            generation: self.generation,
        });
        self.bundle_ids.insert(name.into(), id);
        id
    }

    /// The id of a bundle, if it exists.
    #[must_use]
    pub fn bundle_id(&self, name: &str) -> Option<BundleId> {
        self.bundle_ids.get(name).copied()
    }

    /// Every bundle: the ones providers offered at the last reload and the ones declared.
    pub fn bundles(&self) -> impl Iterator<Item = (BundleId, &str)> {
        (0..).map(BundleId).zip(self.bundles.iter().map(|bundle| &*bundle.name))
    }

    /// A bundle's name.
    #[must_use]
    pub fn bundle_name(&self, bundle: BundleId) -> Option<&str> {
        self.bundles.get(bundle.0 as usize).map(|bundle| &*bundle.name)
    }

    /// The generation a bundle's messages were last swapped in at.
    #[must_use]
    pub fn bundle_generation(&self, bundle: BundleId) -> Option<Generation> {
        self.bundles.get(bundle.0 as usize).map(|bundle| bundle.generation)
    }

    /// A bundle's fallback locale: the one tried after every preferred locale.
    #[must_use]
    pub fn bundle_fallback(&self, bundle: BundleId) -> Option<LocaleId> {
        self.bundles.get(bundle.0 as usize).map(|bundle| bundle.fallback)
    }

    /// Sets a bundle's fallback locale and returns the new generation, or `None` for a bundle
    /// that isn't this registry's.
    pub fn set_bundle_fallback(&mut self, bundle: BundleId, fallback: LocaleId) -> Option<Generation> {
        if fallback.0 as usize >= self.locales.len() {
            return None;
        }
        let bundle = self.bundles.get_mut(bundle.0 as usize)?;
        if bundle.fallback != fallback {
            bundle.fallback = fallback;
            bundle.resolve(&self.chain);
            self.generation = self.generation.next();
        }
        Some(self.generation)
    }

    /// The locales a bundle has files for, in tag order.
    #[must_use]
    pub fn bundle_locales(&self, bundle: BundleId) -> Vec<LocaleId> {
        self.bundles
            .get(bundle.0 as usize)
            .map(|bundle| bundle.loaded.locales.iter().map(|(locale, _)| *locale).collect())
            .unwrap_or_default()
    }

    // Messages.

    /// The id of a message name in a bundle, allocated now if the name is new. The id is good
    /// whether or not any locale defines the message yet; a later reload may.
    ///
    /// # Panics
    ///
    /// If the bundle is another registry's.
    pub fn message(&mut self, bundle: BundleId, name: &str) -> MessageId {
        MessageId::new(bundle, self.bundles[bundle.0 as usize].intern(name))
    }

    /// The id of a message name in a bundle, if the name has been seen.
    #[must_use]
    pub fn message_id(&self, bundle: BundleId, name: &str) -> Option<MessageId> {
        let index = self.bundles.get(bundle.0 as usize)?.name_ids.get(name)?;
        Some(MessageId::new(bundle, *index))
    }

    /// A message's name.
    #[must_use]
    pub fn message_name(&self, id: MessageId) -> Option<&str> {
        let bundle = self.bundles.get(id.bundle().0 as usize)?;
        bundle.names.get(id.index()).map(|name| &**name)
    }

    /// Every message name a bundle knows: the ones its files define and the ones asked for.
    pub fn messages(&self, bundle: BundleId) -> impl Iterator<Item = (MessageId, &str)> {
        let names = self
            .bundles
            .get(bundle.0 as usize)
            .map_or(&[][..], |bundle| &bundle.names[..]);
        (0..)
            .map(move |index| MessageId::new(bundle, index))
            .zip(names.iter().map(|name| &**name))
    }

    /// What the current locale chain resolves a message to, or `None` if it's missing.
    #[must_use]
    pub fn lookup(&self, id: MessageId) -> Option<MessageInfo<'_>> {
        let bundle = self.bundles.get(id.bundle().0 as usize)?;
        let entry = *bundle.resolved.get(id.index())?;
        Self::info(bundle, entry)
    }

    /// A message as one locale's file defines it, or `None` if that locale doesn't. No fallback.
    #[must_use]
    pub fn lookup_in(&self, id: MessageId, locale: LocaleId) -> Option<MessageInfo<'_>> {
        let bundle = self.bundles.get(id.bundle().0 as usize)?;
        let (_, table) = bundle.loaded.locales.iter().find(|(known, _)| *known == locale)?;
        Self::info(bundle, *table.get(id.index())?)
    }

    fn info(bundle: &Bundle, entry: u32) -> Option<MessageInfo<'_>> {
        let entry = bundle.loaded.entries.get(entry as usize)?;
        Some(MessageInfo {
            locale: entry.locale,
            source: &entry.source,
            arguments: bundle.loaded.arena.arguments(entry.pattern),
        })
    }

    /// The text of a message that takes no arguments, borrowed from the registry, or `None` if
    /// the message is missing or has arguments. A label that never changes needn't be copied.
    #[must_use]
    pub fn literal(&self, id: MessageId) -> Option<&str> {
        let bundle = self.bundles.get(id.bundle().0 as usize)?;
        let entry = *bundle.resolved.get(id.index())?;
        let entry = bundle.loaded.entries.get(entry as usize)?;
        bundle.loaded.arena.literal(entry.pattern)
    }

    // Formatting.

    /// Formats a message into `out`, appending.
    ///
    /// The pattern is the one the locale chain resolves to, and the plural and number rules are
    /// those of the locale the pattern came from, as in `OpenMW` (`loadLocaleYaml` in
    /// `components/l10n/messagebundles.cpp` builds each `MessageFormat` with its file's locale).
    /// On an error `out` is left as it was.
    ///
    /// # Errors
    ///
    /// Returns why the message has no text (unknown id, no locale defines it) or why the pattern rejected the arguments.
    pub fn format(&self, id: MessageId, args: &[(&str, Value<'_>)], out: &mut String) -> Result<Resolved, FormatError> {
        let Some(bundle) = self.bundles.get(id.bundle().0 as usize) else {
            return Err(Self::unknown(id));
        };
        let entry = match bundle.resolved.get(id.index()) {
            Some(&entry) if entry != NONE => &bundle.loaded.entries[entry as usize],
            Some(_) => return Err(self.missing(bundle, id)),
            None if id.index() < bundle.names.len() => return Err(self.missing(bundle, id)),
            None => return Err(Self::unknown(id)),
        };
        let start = out.len();
        let rules = &self.locales[entry.locale.0 as usize].rules;
        if let Err(error) = bundle.loaded.arena.format(entry.pattern, rules, args, out) {
            out.truncate(start);
            return Err(FormatError {
                bundle: bundle.name.clone(),
                message: bundle.names[id.index()].clone(),
                kind: FormatErrorKind::Argument {
                    locale: self.locale_name(entry.locale).to_owned(),
                    error,
                },
            });
        }
        Ok(Resolved {
            locale: entry.locale,
            fallback: self.preferred.first().is_some_and(|first| *first != entry.locale),
        })
    }

    /// Formats a message into a new `String`.
    ///
    /// # Errors
    ///
    /// As [`format`](Self::format).
    pub fn format_to_string(&self, id: MessageId, args: &[(&str, Value<'_>)]) -> Result<String, FormatError> {
        let mut out = String::new();
        self.format(id, args, &mut out)?;
        Ok(out)
    }

    /// Compiles and formats a pattern that isn't in any bundle, with a locale's rules.
    ///
    /// For tools, tests, and text that arrives at runtime. It compiles on every call; a bundle's
    /// messages compile once at load, which is where user-facing text belongs.
    ///
    /// # Errors
    ///
    /// Returns an error when the locale isn't this registry's, the pattern doesn't compile, or an argument is missing or of the wrong type.
    pub fn format_pattern(
        &self,
        locale: LocaleId,
        pattern: &str,
        args: &[(&str, Value<'_>)],
        out: &mut String,
    ) -> Result<(), PatternFormatError> {
        let rules = &self
            .locales
            .get(locale.0 as usize)
            .ok_or(PatternFormatError::UnknownLocale)?
            .rules;
        let mut arena = Arena::new();
        let compiled = arena.compile(pattern).map_err(PatternFormatError::Pattern)?;
        let start = out.len();
        arena.format(compiled, rules, args, out).map_err(|error| {
            out.truncate(start);
            PatternFormatError::Argument(error)
        })
    }

    /// Appends the placeholder for a message that couldn't be formatted: `[[bundle:message]]`.
    ///
    /// For a caller that must show something. It is deliberately loud and never a translation;
    /// the [`FormatError`] is still the caller's to report.
    pub fn placeholder(&self, id: MessageId, out: &mut String) {
        out.push_str("[[");
        match (self.bundle_name(id.bundle()), self.message_name(id)) {
            (Some(bundle), Some(message)) => {
                out.push_str(bundle);
                out.push(':');
                out.push_str(message);
            }
            _ => out.push_str(&id.to_string()),
        }
        out.push_str("]]");
    }

    /// The diagnostic for a message a fallback locale supplied, or `None` if the first preferred
    /// locale did.
    #[must_use]
    pub fn fallback_diagnostic(&self, id: MessageId, resolved: Resolved) -> Option<Diagnostic> {
        if !resolved.fallback {
            return None;
        }
        let wanted = self.preferred.first().map(|locale| self.locale_name(*locale))?;
        let used = self.locale_name(resolved.locale);
        let mut diagnostic = Diagnostic::new(
            Severity::Info,
            DiagnosticKind::FallbackUsed,
            self.generation,
            format!("no text in {wanted}; {used} supplied it"),
        )
        .locale(used);
        if let (Some(bundle), Some(message)) = (self.bundle_name(id.bundle()), self.message_name(id)) {
            diagnostic = diagnostic.bundle(bundle).message(message);
        }
        if let Some(info) = self.lookup(id) {
            diagnostic = diagnostic.path(&info.source.path).at(info.source.line, info.source.column);
            if let Some(provider) = self.provider_name(info.source.provider) {
                diagnostic = diagnostic.provider(provider);
            }
        }
        Some(diagnostic)
    }

    fn unknown(id: MessageId) -> FormatError {
        FormatError {
            bundle: id.bundle().to_string().into(),
            message: id.to_string().into(),
            kind: FormatErrorKind::UnknownMessage,
        }
    }

    fn missing(&self, bundle: &Bundle, id: MessageId) -> FormatError {
        let mut tried: Vec<String> = self.chain.iter().map(|locale| self.locale_name(*locale).to_owned()).collect();
        let fallback = self.locale_name(bundle.fallback);
        if !tried.iter().any(|locale| locale == fallback) {
            tried.push(fallback.to_owned());
        }
        FormatError {
            bundle: bundle.name.clone(),
            message: bundle.names[id.index()].clone(),
            kind: FormatErrorKind::MissingMessage { tried },
        }
    }

    // Loading.

    /// Reads every provider again and swaps in each bundle that loads without an error.
    ///
    /// Every file of every locale is parsed and every pattern compiled here, so a malformed
    /// message is reported now, not when a rare code path first formats it. The rules:
    ///
    /// - Within a bundle and locale, messages merge by id; a provider registered later replaces
    ///   an earlier provider's text.
    /// - A bundle with any error (a file that doesn't parse, a pattern that doesn't compile, a
    ///   file named for something that isn't a locale, one provider offering a locale twice)
    ///   is not swapped in: its last good generation stays live. A bundle with no good
    ///   generation yet takes what did load, so a first load with one bad message still shows
    ///   the rest.
    /// - A provider that can't list its files stops the reload; nothing changes.
    ///
    /// The generation rises if any bundle was swapped in.
    pub fn reload(&mut self) -> Report {
        let next = self.generation.next();
        let mut diagnostics = Vec::new();
        let mut listed: Vec<(usize, Resource)> = Vec::new();
        let mut listing_failed = false;
        for (index, provider) in self.providers.iter().enumerate() {
            match provider.resources() {
                Ok(resources) => listed.extend(resources.into_iter().map(|resource| (index, resource))),
                Err(error) => {
                    let mut diagnostic = Diagnostic::error(
                        DiagnosticKind::ProviderFailure,
                        self.generation,
                        format!("couldn't list bundles, so nothing was reloaded: {}", error.reason),
                    )
                    .provider(provider.name());
                    if let Some(path) = &error.path {
                        diagnostic = diagnostic.path(path);
                    }
                    diagnostics.push(diagnostic);
                    listing_failed = true;
                }
            }
        }
        if listing_failed {
            return Report {
                generation: self.generation,
                diagnostics,
                updated: Vec::new(),
                kept: (0..).map(BundleId).take(self.bundles.len()).collect(),
            };
        }

        // Bundle name, then canonical locale tag, then the files in provider order.
        let mut groups: BTreeMap<String, BundleFiles> = BTreeMap::new();
        for bundle in &self.bundles {
            groups.entry(bundle.name.to_string()).or_default();
        }
        for (provider, resource) in listed {
            let provider_name = self.providers[provider].name();
            if let Err(error) = validate_bundle_name(&resource.bundle) {
                diagnostics.push(
                    Diagnostic::error(DiagnosticKind::InvalidBundleName, next, error.to_string())
                        .provider(provider_name)
                        .path(&resource.path),
                );
                continue;
            }
            let files = groups.entry(resource.bundle.clone()).or_default();
            match Locale::parse(&resource.locale) {
                Ok(locale) => files
                    .locales
                    .entry(locale.as_str().to_owned())
                    .or_insert_with(|| (locale, Vec::new()))
                    .1
                    .push((provider, resource)),
                Err(error) => {
                    files.failed = true;
                    diagnostics.push(
                        Diagnostic::error(DiagnosticKind::InvalidLocale, next, error.to_string())
                            .provider(provider_name)
                            .path(&resource.path)
                            .bundle(&resource.bundle),
                    );
                }
            }
        }

        let mut updated = Vec::new();
        let mut kept = Vec::new();
        for (name, files) in groups {
            let id = self.declare(&name);
            let (loaded, failed) = self.load_bundle(id, files, next, &mut diagnostics);
            let bundle = &mut self.bundles[id.0 as usize];
            if failed && bundle.has_loaded {
                kept.push(id);
                continue;
            }
            bundle.loaded = loaded;
            bundle.has_loaded = true;
            bundle.generation = next;
            bundle.resolve(&self.chain);
            updated.push(id);
        }
        if !updated.is_empty() {
            self.generation = next;
        }
        Report {
            generation: self.generation,
            diagnostics,
            updated,
            kept,
        }
    }

    /// Reads and compiles one bundle's files. Returns what loaded and whether anything failed.
    fn load_bundle(
        &mut self,
        id: BundleId,
        files: BundleFiles,
        generation: Generation,
        diagnostics: &mut Vec<Diagnostic>,
    ) -> (Loaded, bool) {
        let mut failed = files.failed;
        let mut loaded = Loaded::default();
        let bundle_name = self.bundles[id.0 as usize].name.to_string();
        if files.locales.is_empty() && !failed {
            diagnostics.push(
                Diagnostic::new(
                    Severity::Warning,
                    DiagnosticKind::EmptyBundle,
                    generation,
                    "no provider has a file for this bundle; every message in it is missing",
                )
                .bundle(&bundle_name),
            );
        }
        for (locale_name, (locale, sources)) in files.locales {
            let locale_id = match self.intern_locale(&locale) {
                Ok(locale_id) => locale_id,
                Err(error) => {
                    failed = true;
                    diagnostics.push(
                        Diagnostic::error(DiagnosticKind::InvalidLocale, generation, error.to_string())
                            .bundle(&bundle_name)
                            .locale(&locale_name),
                    );
                    continue;
                }
            };
            // Message index to its winning definition; a later provider replaces an earlier one.
            let mut merged: BTreeMap<u32, (Pattern, Source)> = BTreeMap::new();
            for (position, (provider, resource)) in sources.iter().enumerate() {
                let provider_name = self.providers[*provider].name().to_owned();
                let context = |diagnostic: Diagnostic| {
                    diagnostic
                        .provider(&provider_name)
                        .path(&resource.path)
                        .bundle(&bundle_name)
                        .locale(&locale_name)
                };
                // Two files of one provider for one locale (`en.toml` and `EN.toml`, say) sit at
                // the same precedence; neither may win.
                let collision = sources
                    .iter()
                    .enumerate()
                    .find(|(index, (other, _))| *index != position && other == provider);
                if let Some((_, (_, other))) = collision {
                    failed = true;
                    diagnostics.push(context(Diagnostic::error(
                        DiagnosticKind::ProviderCollision,
                        generation,
                        format!(
                            "the provider also offers {} for the same bundle and locale; neither was loaded",
                            other.path
                        ),
                    )));
                    continue;
                }
                let text = match self.providers[*provider].read(resource) {
                    Ok(text) => text,
                    Err(error) => {
                        failed = true;
                        diagnostics.push(context(Diagnostic::error(
                            DiagnosticKind::ProviderFailure,
                            generation,
                            format!("couldn't read the file: {}", error.reason),
                        )));
                        continue;
                    }
                };
                #[allow(clippy::cast_possible_truncation)] // A provider index is a `ProviderId`'s.
                let source = FileSource {
                    provider: ProviderId(*provider as u32),
                    path: resource.path.as_str().into(),
                    text: &text,
                };
                let file_failed =
                    self.compile_file(id, &source, &context, generation, diagnostics, &mut loaded.arena, &mut merged);
                failed |= file_failed;
            }
            let mut table = vec![NONE; self.bundles[id.0 as usize].names.len()];
            for (index, (pattern, source)) in merged {
                table[index as usize] = u32::try_from(loaded.entries.len()).expect("fewer than 2^32 messages in a bundle");
                loaded.entries.push(Entry {
                    locale: locale_id,
                    pattern,
                    source,
                });
            }
            loaded.locales.push((locale_id, table));
        }
        loaded.arena.shrink_to_fit();
        (loaded, failed)
    }

    /// Parses one file and compiles its messages into `merged`, replacing what lower-precedence
    /// files defined. Returns whether anything in it failed.
    #[allow(clippy::too_many_arguments)] // One file's worth of loading state, passed once.
    fn compile_file(
        &mut self,
        id: BundleId,
        file: &FileSource<'_>,
        context: &dyn Fn(Diagnostic) -> Diagnostic,
        generation: Generation,
        diagnostics: &mut Vec<Diagnostic>,
        arena: &mut Arena,
        merged: &mut BTreeMap<u32, (Pattern, Source)>,
    ) -> bool {
        let mut failed = false;
        let (messages, errors) = file::parse(file.text);
        let lines = file::LineIndex::new(file.text);
        for error in errors {
            failed = true;
            let kind = if error.duplicate {
                DiagnosticKind::DuplicateMessage
            } else {
                DiagnosticKind::BundleParseFailure
            };
            let mut diagnostic = context(Diagnostic::error(kind, generation, error.reason)).at(error.line, error.column);
            if let Some(message) = &error.message {
                diagnostic = diagnostic.message(message);
            }
            diagnostics.push(diagnostic);
        }
        for message in messages {
            match arena.compile(&message.pattern) {
                Ok(pattern) => {
                    let index = self.bundles[id.0 as usize].intern(&message.id);
                    let source = Source {
                        provider: file.provider,
                        path: file.path.clone(),
                        line: message.line,
                        column: message.column,
                    };
                    merged.insert(index, (pattern, source));
                }
                Err(error) => {
                    failed = true;
                    let (line, column) = message
                        .pattern_offset
                        .map_or((message.line, message.column), |offset| lines.position(offset + error.offset));
                    diagnostics.push(
                        context(Diagnostic::error(
                            DiagnosticKind::PatternParseFailure,
                            generation,
                            error.to_string(),
                        ))
                        .at(line, column)
                        .message(&message.id),
                    );
                }
            }
        }
        failed
    }
}

/// One bundle file as read: who supplied it, where, and its text.
struct FileSource<'t> {
    provider: ProviderId,
    path: Arc<str>,
    text: &'t str,
}

/// The files of one bundle found by a reload.
#[derive(Default)]
struct BundleFiles {
    /// Something about the bundle's files is already wrong.
    failed: bool,
    /// Canonical locale tag to the locale and its files, lowest-precedence provider first.
    locales: BTreeMap<String, (Locale, Vec<(usize, Resource)>)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_names() {
        for name in [
            "common",
            "jess.common",
            "s3maphore.playlist-editor",
            "OMWEngine",
            "a_b.c-d.e9",
        ] {
            assert!(validate_bundle_name(name).is_ok(), "{name}");
        }
        for name in ["", ".", "a..b", ".a", "a.", "a/b", "a b", "../etc", "ünï"] {
            assert!(validate_bundle_name(name).is_err(), "{name}");
        }
    }

    #[test]
    fn a_registry_is_shareable() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Registry>();
    }
}
