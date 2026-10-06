//! The acceptance tests of the localization handoff, over the fixture bundles in `tests/fixtures`.
//!
//! `base` is a package's own bundle (`common` in en, pl, pt, de); `layer` is a contribution over
//! it. Tests that change files mid-run use an in-memory VFS instead, so nothing writes to the
//! fixtures.

use std::path::PathBuf;

use jess::l10n::{
    ArgumentError, BundleId, DiagnosticKind, DirectoryProvider, FormatErrorKind, Locale, MemoryFiles, PatternFormatError,
    Provider, ProviderError, Registry, Resource, Severity, Value, VfsProvider,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/l10n/fixtures").join(name)
}

fn locale(tag: &str) -> Locale {
    Locale::parse(tag).unwrap()
}

/// A registry over the base fixture, loaded, and its `common` bundle.
fn base() -> (Registry, BundleId) {
    let mut registry = Registry::new();
    registry.add_provider(DirectoryProvider::new("base", fixture("base")));
    let report = registry.reload();
    assert!(report.is_clean(), "{:#?}", report.diagnostics);
    let common = registry.bundle("common").unwrap();
    (registry, common)
}

fn text(registry: &mut Registry, bundle: BundleId, message: &str, args: &[(&str, Value<'_>)]) -> String {
    let id = registry.message(bundle, message);
    registry.format_to_string(id, args).unwrap()
}

/// The fixture's files as an in-memory VFS under `l10n/`.
fn base_in_memory() -> MemoryFiles {
    let files = MemoryFiles::new();
    for locale in ["en", "pl", "pt", "de"] {
        let text = std::fs::read_to_string(fixture("base").join(format!("common/{locale}.toml"))).unwrap();
        files.insert(format!("l10n/common/{locale}.toml"), text);
    }
    files
}

// 1. The default locale resolves.
#[test]
fn default_locale_resolves() {
    let (mut registry, common) = base();
    assert_eq!(registry.preferred_locales(), []);
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Hello, Fargoth!"
    );
    let id = registry.message(common, "hello");
    let resolved = registry.format(id, &[("name", "x".into())], &mut String::new()).unwrap();
    assert_eq!(registry.locale_name(resolved.locale), "en");
    assert!(
        !resolved.fallback,
        "with no preference the bundle's own locale isn't a fallback"
    );
}

// 2. An exact locale resolves.
#[test]
fn exact_locale_resolves() {
    let (mut registry, common) = base();
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Cześć, Fargoth!"
    );
    let id = registry.message(common, "hello");
    let resolved = registry.format(id, &[("name", "x".into())], &mut String::new()).unwrap();
    assert_eq!(registry.locale_name(resolved.locale), "pl");
    assert!(!resolved.fallback);
}

// 3. A regional locale falls back to its language.
#[test]
fn regional_locale_falls_back_to_language() {
    let (mut registry, common) = base();
    registry.set_preferred_locales(&[locale("pt-BR")]).unwrap();
    let chain: Vec<&str> = registry
        .locale_chain(common)
        .into_iter()
        .map(|id| registry.locale_name(id))
        .collect();
    assert_eq!(chain, ["pt-BR", "pt", "en"]);
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Olá, Fargoth!"
    );
    let id = registry.message(common, "hello");
    let resolved = registry.format(id, &[("name", "x".into())], &mut String::new()).unwrap();
    assert_eq!(registry.locale_name(resolved.locale), "pt");
    assert!(resolved.fallback);
    let diagnostic = registry.fallback_diagnostic(id, resolved).unwrap();
    assert_eq!(diagnostic.kind, DiagnosticKind::FallbackUsed);
    assert_eq!(diagnostic.severity, Severity::Info);
    assert_eq!(diagnostic.locale.as_deref(), Some("pt"));

    // Every preferred locale's parents come before the next preferred locale.
    registry
        .set_preferred_locales(&[locale("zh-Hant-TW"), locale("de-AT"), locale("de")])
        .unwrap();
    let chain: Vec<&str> = registry
        .locale_chain(common)
        .into_iter()
        .map(|id| registry.locale_name(id))
        .collect();
    assert_eq!(chain, ["zh-Hant-TW", "zh-Hant", "zh", "de-AT", "de", "en"]);
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Hallo, Fargoth!"
    );
}

// 4. The bundle's fallback locale works, and it needn't be English.
#[test]
fn bundle_fallback_locale() {
    let (mut registry, common) = base();
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(text(&mut registry, common, "fallback_only", &[]), "Only English says this");

    // pt has no file_count; with de as the bundle's fallback, German supplies it.
    registry.set_preferred_locales(&[locale("pt-BR")]).unwrap();
    assert_eq!(text(&mut registry, common, "file_count", &[("count", 2.into())]), "2 files");
    let german = registry.locale("de").unwrap();
    let before = registry.generation();
    assert!(registry.set_bundle_fallback(common, german).unwrap() > before);
    assert_eq!(registry.bundle_fallback(common), Some(german));
    assert_eq!(text(&mut registry, common, "file_count", &[("count", 2.into())]), "2 Dateien");

    // The fallback is one locale, not a chain: nothing reaches English any more.
    let id = registry.message(common, "fallback_only");
    let error = registry.format_to_string(id, &[]).unwrap_err();
    assert_eq!(
        error.kind,
        FormatErrorKind::MissingMessage {
            tried: vec!["pt-BR".into(), "pt".into(), "de".into()]
        }
    );
}

// 5. Plural categories differ correctly for a non-English locale.
#[test]
fn plural_categories_differ_by_locale() {
    let (mut registry, common) = base();
    let count = |registry: &mut Registry, count: f64| text(registry, common, "file_count", &[("count", count.into())]);
    assert_eq!(count(&mut registry, 0.0), "No files");
    assert_eq!(count(&mut registry, 1.0), "1 file");
    assert_eq!(count(&mut registry, 2.0), "2 files");
    assert_eq!(count(&mut registry, 5.0), "5 files");
    assert_eq!(count(&mut registry, 22.0), "22 files");

    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(count(&mut registry, 0.0), "Brak plików");
    assert_eq!(count(&mut registry, 1.0), "1 plik");
    assert_eq!(count(&mut registry, 2.0), "2 pliki");
    assert_eq!(count(&mut registry, 5.0), "5 plików");
    assert_eq!(count(&mut registry, 22.0), "22 pliki");
    assert_eq!(count(&mut registry, 1.5), "1,5 pliku");
    assert_eq!(count(&mut registry, 1234.0), "1234 pliki");
    assert_eq!(count(&mut registry, 12345.0), "12\u{a0}345 plików");
}

// 6. Select works.
#[test]
fn select() {
    let (mut registry, common) = base();
    let mode = |registry: &mut Registry, mode: &str| text(registry, common, "mode_label", &[("mode", mode.into())]);
    assert_eq!(mode(&mut registry, "edit"), "Editing");
    assert_eq!(mode(&mut registry, "view"), "Viewing");
    assert_eq!(mode(&mut registry, "asleep"), "Idle");
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(mode(&mut registry, "edit"), "Edycja");
    assert_eq!(mode(&mut registry, "asleep"), "Bezczynny");
}

// Number formatting, by the locale the message came from.
#[test]
fn number_formatting() {
    let (mut registry, common) = base();
    let progress = [("progress", Value::Float(0.42))];
    assert_eq!(text(&mut registry, common, "percent_complete", &progress), "42% complete");
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(text(&mut registry, common, "percent_complete", &progress), "Ukończono 42%");
    registry.set_preferred_locales(&[locale("de")]).unwrap();
    assert_eq!(
        text(
            &mut registry,
            common,
            "percent_complete",
            &[("progress", Value::Float(0.425))]
        ),
        "42,5% abgeschlossen"
    );
}

// 7. A missing parameter is a clear error.
#[test]
fn missing_parameter() {
    let (mut registry, common) = base();
    let id = registry.message(common, "hello");
    let mut out = String::from("kept");
    let error = registry.format(id, &[], &mut out).unwrap_err();
    assert_eq!(out, "kept", "a failed format leaves the output as it was");
    assert_eq!(&*error.bundle, "common");
    assert_eq!(&*error.message, "hello");
    assert_eq!(
        error.kind,
        FormatErrorKind::Argument {
            locale: "en".into(),
            error: ArgumentError::Missing { argument: "name".into() }
        }
    );
    assert_eq!(
        error.to_string(),
        "bundle 'common', message 'hello' (en): argument 'name' is missing"
    );
    let diagnostic = error.diagnostic(registry.generation());
    assert_eq!(diagnostic.kind, DiagnosticKind::MissingArgument);
    assert_eq!(diagnostic.bundle.as_deref(), Some("common"));
    assert_eq!(diagnostic.message.as_deref(), Some("hello"));
    assert_eq!(diagnostic.locale.as_deref(), Some("en"));
}

// 8. An unsupported parameter type is a clear error.
#[test]
fn unsupported_parameter_type() {
    let (mut registry, common) = base();
    let id = registry.message(common, "file_count");
    let error = registry.format_to_string(id, &[("count", "many".into())]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "bundle 'common', message 'file_count' (en): argument 'count' must be a number, not a string"
    );
    assert_eq!(
        error.diagnostic(registry.generation()).kind,
        DiagnosticKind::UnsupportedArgumentType
    );
    let id = registry.message(common, "mode_label");
    let error = registry.format_to_string(id, &[("mode", 3.into())]).unwrap_err();
    assert_eq!(
        error.to_string(),
        "bundle 'common', message 'mode_label' (en): argument 'mode' must be a string or boolean, not a number"
    );
    // What a pattern needs is inspectable before anything is formatted.
    let arguments = registry.lookup(id).unwrap().arguments;
    assert_eq!(arguments[0].name, "mode");
    assert_eq!(arguments[0].kind, jess::l10n::ArgumentKind::Select);
}

// A missing message is an error naming what was tried, and the placeholder is loud.
#[test]
fn missing_message() {
    let (mut registry, common) = base();
    registry.set_preferred_locales(&[locale("pl-PL")]).unwrap();
    let id = registry.message(common, "no_such_message");
    let mut out = String::new();
    let error = registry.format(id, &[], &mut out).unwrap_err();
    assert!(out.is_empty(), "a missing message is never an empty success");
    assert_eq!(
        error.to_string(),
        "bundle 'common' has no message 'no_such_message' in pl-PL, pl, en"
    );
    assert_eq!(error.diagnostic(registry.generation()).kind, DiagnosticKind::MissingMessage);
    registry.placeholder(id, &mut out);
    assert_eq!(out, "[[common:no_such_message]]");
    assert!(registry.lookup(id).is_none());
}

// 9. A malformed pattern is caught at load and at reload, with its place in the file.
#[test]
fn malformed_pattern_is_caught_at_load() {
    let files = MemoryFiles::new();
    files.insert(
        "l10n/common/en.toml",
        "good = 'Fine'\n\nbroken = '''\n{count, plural,\n  one {# file}\n  othr {# files}}'''\nunclosed = \"{name\"\n",
    );
    let mut registry = Registry::new();
    registry.add_provider(VfsProvider::new("package", "l10n", files));
    let report = registry.reload();
    assert!(!report.is_clean());
    let errors: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == DiagnosticKind::PatternParseFailure)
        .collect();
    assert_eq!(errors.len(), 2, "{:#?}", report.diagnostics);
    let broken = errors[0];
    assert_eq!(broken.severity, Severity::Error);
    assert_eq!(broken.provider.as_deref(), Some("package"));
    assert_eq!(broken.path.as_deref(), Some("l10n/common/en.toml"));
    assert_eq!(broken.bundle.as_deref(), Some("common"));
    assert_eq!(broken.locale.as_deref(), Some("en"));
    assert_eq!(broken.message.as_deref(), Some("broken"));
    assert_eq!(
        (broken.line, broken.column),
        (Some(6), Some(3)),
        "the bad selector's own line and column"
    );
    assert!(broken.detail.contains("'othr' isn't a plural case"), "{}", broken.detail);
    assert_eq!(broken.generation, report.generation);
    assert_eq!(
        broken.to_string(),
        "error[pattern-parse-failure] l10n/common/en.toml:6:3 (provider package): common/en 'broken': at offset 33: 'othr' isn't a plural case; use =n, zero, one, two, few, many, or other"
    );
    assert_eq!(errors[1].message.as_deref(), Some("unclosed"));
    assert_eq!(errors[1].line, Some(7));

    // With no good generation to keep, the first load takes what compiled.
    let common = registry.bundle_id("common").unwrap();
    assert_eq!(report.updated, [common]);
    assert_eq!(text(&mut registry, common, "good", &[]), "Fine");
    let broken = registry.message(common, "broken");
    assert!(matches!(
        registry.format_to_string(broken, &[("count", 1.into())]).unwrap_err().kind,
        FormatErrorKind::MissingMessage { .. }
    ));
}

// 10. A duplicate message is a deterministic error: neither definition wins.
#[test]
fn duplicate_messages_are_errors() {
    let files = MemoryFiles::new();
    files.insert("l10n/common/en.toml", "hello = 'first'\nhello = 'second'\n");
    let mut registry = Registry::new();
    registry.add_provider(VfsProvider::new("package", "l10n", files.clone()));
    let report = registry.reload();
    let duplicate = &report.diagnostics[0];
    assert_eq!(duplicate.kind, DiagnosticKind::DuplicateMessage);
    assert_eq!((duplicate.line, duplicate.column), (Some(2), Some(1)));
    assert_eq!(duplicate.path.as_deref(), Some("l10n/common/en.toml"));

    // Two files of one provider for one locale sit at the same precedence: a collision.
    files.insert("l10n/common/en.toml", "hello = 'lowercase file'\n");
    files.insert("l10n/common/EN.toml", "hello = 'uppercase file'\n");
    let first = registry.reload();
    let collisions: Vec<_> = first
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == DiagnosticKind::ProviderCollision)
        .map(|diagnostic| diagnostic.path.clone().unwrap())
        .collect();
    assert_eq!(collisions, ["l10n/common/EN.toml", "l10n/common/en.toml"]);
    let common = registry.bundle_id("common").unwrap();
    assert_eq!(first.kept, [common]);
    assert_eq!(
        registry.reload().diagnostics,
        first.diagnostics,
        "the same files report the same way every time"
    );
}

// 11. Provider precedence is the registration order, whatever order files are listed in.
#[test]
fn provider_precedence_is_registration_order() {
    let layered = |first: &str, second: &str| {
        let mut registry = Registry::new();
        registry.add_provider(DirectoryProvider::new(first, fixture(first)));
        registry.add_provider(DirectoryProvider::new(second, fixture(second)));
        assert!(registry.reload().is_clean());
        let common = registry.bundle("common").unwrap();
        (registry, common)
    };
    let (mut registry, common) = layered("base", "layer");
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Howdy, Fargoth!"
    );
    // Messages merge by id: the layer adds one and leaves the rest of the base alone.
    assert_eq!(text(&mut registry, common, "layer_only", &[]), "From the layer");
    assert_eq!(text(&mut registry, common, "fallback_only", &[]), "Only English says this");
    // The override is per locale: Polish still comes from the base.
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Cześć, Fargoth!"
    );

    let (mut registry, common) = layered("layer", "base");
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Hello, Fargoth!"
    );
    assert_eq!(text(&mut registry, common, "layer_only", &[]), "From the layer");
}

// 12. Source provenance is preserved.
#[test]
fn provenance() {
    let mut registry = Registry::new();
    let base = registry.add_provider(DirectoryProvider::new("base", fixture("base")));
    let layer = registry.add_provider(DirectoryProvider::new("layer", fixture("layer")));
    assert!(registry.reload().is_clean());
    let common = registry.bundle("common").unwrap();

    let hello = registry.message(common, "hello");
    let info = registry.lookup(hello).unwrap();
    assert_eq!(info.source.provider, layer);
    assert_eq!(registry.provider_name(info.source.provider), Some("layer"));
    assert!(info.source.path.ends_with("layer/common/en.toml"), "{}", info.source.path);
    assert_eq!((info.source.line, info.source.column), (2, 1));

    let file_count = registry.message(common, "file_count");
    let info = registry.lookup(file_count).unwrap();
    assert_eq!(info.source.provider, base);
    assert!(info.source.path.ends_with("base/common/en.toml"), "{}", info.source.path);
    assert_eq!(info.source.line, 7);

    // Is this message missing in that locale? Which file defines it there?
    let polish = registry.locale("pl").unwrap();
    let german = registry.locale("de").unwrap();
    let fallback_only = registry.message(common, "fallback_only");
    assert!(registry.lookup_in(fallback_only, polish).is_none());
    let info = registry.lookup_in(hello, polish).unwrap();
    assert_eq!(info.source.provider, base);
    assert!(info.source.path.ends_with("base/common/pl.toml"));
    let mode_label = registry.message(common, "mode_label");
    assert!(registry.lookup_in(mode_label, german).is_none());
}

// Inspection: bundles, providers, locales, and messages can all be enumerated.
#[test]
fn inspection() {
    let mut registry = Registry::new();
    registry.add_provider(DirectoryProvider::new("base", fixture("base")));
    registry.add_provider(DirectoryProvider::new("layer", fixture("layer")));
    registry.reload();
    let providers: Vec<&str> = registry.providers().map(|(_, name)| name).collect();
    assert_eq!(providers, ["base", "layer"]);
    let bundles: Vec<&str> = registry.bundles().map(|(_, name)| name).collect();
    assert_eq!(bundles, ["common"]);
    let common = registry.bundle_id("common").unwrap();
    let locales: Vec<&str> = registry
        .bundle_locales(common)
        .into_iter()
        .map(|id| registry.locale_name(id))
        .collect();
    assert_eq!(locales, ["de", "en", "pl", "pt"]);
    let mut messages: Vec<&str> = registry.messages(common).map(|(_, name)| name).collect();
    messages.sort_unstable();
    assert_eq!(
        messages,
        [
            "fallback_only",
            "file_count",
            "hello",
            "layer_only",
            "mode_label",
            "percent_complete"
        ]
    );
    for (id, name) in registry.messages(common) {
        assert_eq!(registry.message_name(id), Some(name));
        assert_eq!(registry.message_id(common, name), Some(id));
        assert_eq!(id.bundle(), common);
    }
    assert_eq!(registry.bundle_name(common), Some("common"));
    assert_eq!(registry.bundle_generation(common), Some(registry.generation()));
}

// 13. A runtime locale switch changes the output, with no reload.
#[test]
fn runtime_locale_switch() {
    let (mut registry, common) = base();
    let hello = registry.message(common, "hello");
    let args = [("name", Value::Str("Fargoth"))];
    let loaded = registry.generation();
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Hello, Fargoth!");
    let polish = registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert!(polish > loaded, "a locale change is a new generation");
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Cześć, Fargoth!");
    let german = registry.set_preferred_locales(&[locale("de"), locale("pl")]).unwrap();
    assert!(german > polish);
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Hallo, Fargoth!");
    registry.set_preferred_locales(&[]).unwrap();
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Hello, Fargoth!");
    assert_eq!(
        registry.bundle_generation(common),
        Some(loaded),
        "the bundle itself didn't change; only what the chain picks from it"
    );
}

// 14. The compiled cache doesn't change semantics.
#[test]
fn cache_does_not_change_semantics() {
    let (mut cached, common) = base();
    cached.set_preferred_locales(&[locale("pl")]).unwrap();
    let cases: [(&str, Vec<(&str, Value<'_>)>); 5] = [
        ("hello", vec![("name", "Fargoth".into())]),
        ("file_count", vec![("count", 5.into())]),
        ("mode_label", vec![("mode", "view".into())]),
        ("percent_complete", vec![("progress", Value::Float(0.5))]),
        ("fallback_only", vec![]),
    ];
    for (message, args) in &cases {
        let id = cached.message(common, message);
        let first = cached.format_to_string(id, args).unwrap();
        // The same registry, asked again and again.
        for _ in 0..3 {
            assert_eq!(cached.format_to_string(id, args).unwrap(), first);
        }
        // A registry that has formatted nothing yet.
        let (mut fresh, fresh_common) = base();
        fresh.set_preferred_locales(&[locale("pl")]).unwrap();
        assert_eq!(text(&mut fresh, fresh_common, message, args), first);
        // The same registry after a reload and after switching away and back.
        assert!(cached.reload().is_clean());
        assert_eq!(cached.format_to_string(id, args).unwrap(), first);
        cached.set_preferred_locales(&[locale("de")]).unwrap();
        cached.set_preferred_locales(&[locale("pl")]).unwrap();
        assert_eq!(cached.format_to_string(id, args).unwrap(), first);
    }
}

// 15. A failed hot reload preserves the previous generation.
#[test]
fn failed_hot_reload_keeps_last_good() {
    let files = base_in_memory();
    files.insert("l10n/other/en.toml", "title = 'Other'");
    let mut registry = Registry::new();
    registry.add_provider(VfsProvider::new("package", "l10n", files.clone()));
    assert!(registry.reload().is_clean());
    let common = registry.bundle("common").unwrap();
    let other = registry.bundle("other").unwrap();
    let hello = registry.message(common, "hello");
    let args = [("name", Value::Str("Fargoth"))];
    let good = registry.generation();

    // A good edit is live after one reload.
    files.insert("l10n/common/en.toml", "hello = 'Hi, {name}!'");
    let report = registry.reload();
    assert!(report.is_clean());
    assert!(report.generation > good);
    assert_eq!(
        registry.format_to_string(hello, &args).unwrap(),
        "Hi, {name}!".replace("{name}", "Fargoth")
    );
    let good = registry.generation();

    // A bad edit to one locale of `common`, and a good edit to `other`, in the same reload.
    files.insert("l10n/common/pl.toml", "hello = 'Cześć, {name'");
    files.insert("l10n/common/en.toml", "hello = 'Hey, {name}!'");
    files.insert("l10n/other/en.toml", "title = 'Another'");
    let report = registry.reload();
    assert!(!report.is_clean());
    assert_eq!(report.kept, [common]);
    assert_eq!(report.updated, [other]);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].kind, DiagnosticKind::PatternParseFailure);
    assert_eq!(report.diagnostics[0].locale.as_deref(), Some("pl"));
    // The whole bundle keeps its last good generation: English didn't take the new text either.
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Hi, Fargoth!");
    assert_eq!(registry.bundle_generation(common), Some(good));
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Cześć, Fargoth!");
    // The bundle that did load is live.
    assert_eq!(text(&mut registry, other, "title", &[]), "Another");
    assert!(registry.bundle_generation(other).unwrap() > good);

    // A file that isn't even TOML keeps the last good generation too.
    files.insert("l10n/common/pl.toml", "hello = ");
    let report = registry.reload();
    assert_eq!(report.kept, [common]);
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::BundleParseFailure)
    );
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Cześć, Fargoth!");

    // Fixing the file makes the next reload take everything, including the English edit.
    files.insert("l10n/common/pl.toml", "hello = 'Hej, {name}!'");
    let report = registry.reload();
    assert!(report.is_clean());
    assert_eq!(report.updated, [common, other]);
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Hej, Fargoth!");
    registry.set_preferred_locales(&[]).unwrap();
    assert_eq!(registry.format_to_string(hello, &args).unwrap(), "Hey, Fargoth!");
}

/// A provider whose listing fails on demand.
struct Flaky {
    files: VfsProvider<MemoryFiles>,
    broken: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Provider for Flaky {
    fn name(&self) -> &'static str {
        "flaky"
    }

    fn resources(&self) -> Result<Vec<Resource>, ProviderError> {
        if self.broken.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ProviderError {
                path: Some("l10n".into()),
                reason: "the archive went away".into(),
            });
        }
        self.files.resources()
    }

    fn read(&self, resource: &Resource) -> Result<String, ProviderError> {
        self.files.read(resource)
    }
}

// A provider that can't list its files changes nothing.
#[test]
fn provider_failure_keeps_everything() {
    let broken = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut registry = Registry::new();
    registry.add_provider(Flaky {
        files: VfsProvider::new("inner", "l10n", base_in_memory()),
        broken: broken.clone(),
    });
    assert!(registry.reload().is_clean());
    let common = registry.bundle("common").unwrap();
    let generation = registry.generation();

    broken.store(true, std::sync::atomic::Ordering::Relaxed);
    let report = registry.reload();
    assert_eq!(report.generation, generation);
    assert_eq!(report.kept, [common]);
    assert_eq!(report.updated, []);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].kind, DiagnosticKind::ProviderFailure);
    assert_eq!(report.diagnostics[0].provider.as_deref(), Some("flaky"));
    assert!(report.diagnostics[0].detail.contains("the archive went away"));
    assert_eq!(
        text(&mut registry, common, "hello", &[("name", "Fargoth".into())]),
        "Hello, Fargoth!"
    );
}

// A folder and a VFS holding the same files behave the same.
#[test]
fn directory_and_vfs_providers_agree() {
    let (mut from_folder, folder_common) = base();
    let mut from_vfs = Registry::new();
    from_vfs.add_provider(VfsProvider::new("base", "l10n", base_in_memory()));
    assert!(from_vfs.reload().is_clean());
    let vfs_common = from_vfs.bundle("common").unwrap();
    for tag in ["en", "pl", "pt-BR", "de-AT"] {
        from_folder.set_preferred_locales(&[locale(tag)]).unwrap();
        from_vfs.set_preferred_locales(&[locale(tag)]).unwrap();
        let cases: [(&str, Vec<(&str, Value<'_>)>); 5] = [
            ("hello", vec![("name", "Fargoth".into())]),
            ("file_count", vec![("count", 3.into())]),
            ("mode_label", vec![("mode", "edit".into())]),
            ("percent_complete", vec![("progress", Value::Float(0.25))]),
            ("fallback_only", vec![]),
        ];
        for (message, args) in &cases {
            assert_eq!(
                text(&mut from_folder, folder_common, message, args),
                text(&mut from_vfs, vfs_common, message, args),
                "{message} in {tag}"
            );
        }
    }
}

// Files and folders that can't be what they're named for are reported, not skipped.
#[test]
fn bad_names_are_diagnostics() {
    let files = MemoryFiles::new();
    files.insert("l10n/common/en.toml", "hello = 'Hello'");
    files.insert("l10n/common/english.toml", "hello = 'Hello'");
    files.insert("l10n/bad name/en.toml", "hello = 'Hello'");
    let mut registry = Registry::new();
    registry.add_provider(VfsProvider::new("package", "l10n", files));
    let report = registry.reload();
    let kinds: Vec<_> = report.diagnostics.iter().map(|diagnostic| diagnostic.kind).collect();
    assert_eq!(kinds, [DiagnosticKind::InvalidBundleName, DiagnosticKind::InvalidLocale]);
    assert_eq!(report.diagnostics[0].path.as_deref(), Some("l10n/bad name/en.toml"));
    assert_eq!(report.diagnostics[1].path.as_deref(), Some("l10n/common/english.toml"));
    assert_eq!(report.diagnostics[1].bundle.as_deref(), Some("common"));
    assert!(registry.bundle("bad name").is_err());
    assert!(registry.locale("english").is_err());
}

// A bundle nobody has files for is a warning, and every message in it is missing.
#[test]
fn empty_bundle() {
    let mut registry = Registry::new();
    let ghost = registry.bundle("ghost").unwrap();
    let report = registry.reload();
    assert!(report.is_clean(), "a warning isn't an error");
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].kind, DiagnosticKind::EmptyBundle);
    assert_eq!(report.diagnostics[0].severity, Severity::Warning);
    let id = registry.message(ghost, "anything");
    assert_eq!(
        registry.format_to_string(id, &[]).unwrap_err().to_string(),
        "bundle 'ghost' has no message 'anything' in en"
    );
}

// An id from another registry is an error, not a panic or another message's text.
#[test]
fn foreign_ids_are_errors() {
    let (mut registry, common) = base();
    let hello = registry.message(common, "hello");
    let empty = Registry::new();
    let error = empty.format_to_string(hello, &[]).unwrap_err();
    assert_eq!(error.kind, FormatErrorKind::UnknownMessage);
    let mut out = String::new();
    empty.placeholder(hello, &mut out);
    assert_eq!(out, format!("[[{hello}]]"));
}

// A message with no arguments is borrowed, not copied.
#[test]
fn literals_are_borrowed() {
    let (mut registry, common) = base();
    let fallback_only = registry.message(common, "fallback_only");
    assert_eq!(registry.literal(fallback_only), Some("Only English says this"));
    let hello = registry.message(common, "hello");
    assert_eq!(registry.literal(hello), None, "a message with arguments has to be formatted");
    let missing = registry.message(common, "missing");
    assert_eq!(registry.literal(missing), None);
    registry.set_preferred_locales(&[locale("pl")]).unwrap();
    assert_eq!(registry.literal(fallback_only), Some("Only English says this"));
}

// A pattern that isn't in a bundle can still be formatted, and checked on its own.
#[test]
fn one_shot_patterns() {
    let mut registry = Registry::new();
    let polish = registry.locale("pl").unwrap();
    let mut out = String::new();
    let pattern = "{n, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}";
    registry
        .format_pattern(polish, pattern, &[("n", 5.into())], &mut out)
        .unwrap();
    assert_eq!(out, "5 plików");
    out.clear();
    let error = registry
        .format_pattern(polish, "{n, plural, one {# plik}}", &[], &mut out)
        .unwrap_err();
    assert!(matches!(error, PatternFormatError::Pattern(_)), "{error}");
    let error = registry.format_pattern(polish, "{n}", &[], &mut out).unwrap_err();
    assert!(matches!(error, PatternFormatError::Argument(_)), "{error}");
    assert_eq!(out, "");
    let arguments = jess::l10n::check_pattern(pattern).unwrap();
    assert_eq!(
        (arguments[0].name.as_str(), arguments[0].kind),
        ("n", jess::l10n::ArgumentKind::Number)
    );
    assert!(jess::l10n::check_pattern("{n").is_err());
}
