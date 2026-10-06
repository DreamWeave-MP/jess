---
title: 'Localization'
description: 'What jess-l10n is, the OpenMW model it keeps, the ICU MessageFormat it speaks, how bundles are laid out and loaded, how locales fall back, and why every pattern compiles exactly once.'
weight: 75
extra:
  kind: guide
---

The first UI toolkit that ships with `text = "Open File"` never recovers: every label after it is an English string, and translation becomes a retrofit. So the text service comes first. `jess-l10n` (`libs/l10n`) is that service: it turns a bundle name, a message id, and some variables into a string in the user's language, with plurals and numbers done right, and it does it from tables it built once.

The model is OpenMW's (`components/l10n`). The formatting language is ICU MessageFormat, the one OpenMW's YAML files already use. The implementation is jess's, and it is small.

## What It Is

```text
providers -> reload -> compiled bundles (one arena each) -> resolution tables -> format into a String
```

| Concept | What it is | OpenMW calls it |
| --- | --- | --- |
| Bundle | A named group of messages: `common`, `jess.editor`, `s3maphore.playlist-editor`. | context |
| Message | A stable id inside a bundle: `file_count`. Never the English text. | key |
| Pattern | ICU MessageFormat text for one message in one locale. | the YAML value |
| Provider | Something that offers `<bundle>/<locale>.toml` files: a folder, a VFS, later a package. | the VFS |
| Locale chain | The preferred locales, each followed by its parents, then the bundle's fallback locale. | `MessageBundles` |
| Generation | A counter that rises when a reload swaps a bundle in or a locale preference changes. | (none) |

Nothing in the crate knows about Morrowind, OpenMW's paths, Luau, or a GUI. The Luau end is [`jess.l10n`](@/docs/api/l10n.md); the GUI end doesn't exist yet, and when it does it will get strings from here.

## What It Is Not

- **Not a translation editor, and not machine translation.** It reads files people wrote.
- **Not gettext.** Message ids are semantic (`file_count`), not source strings, and there is no string extraction.
- **Not a text engine.** It returns semantic text. Shaping, fonts, and layout belong to the render side.
- **Not a plural library for Luau.** A script never pluralizes; it passes `count` and the pattern decides.
- **Not an application.** A `Registry` is a value. Under a host, one `RwLock<Registry>` is the domain's service, `jess.l10n` reaches it, and a locale change becomes an `l10n.changed` event ([Application Kernel](@/docs/contributing/application-kernel.md)).

## The Files

Files lay out the way OpenMW's do (`Manager::readLangData` in `components/l10n/manager.cpp` opens `l10n/<context>/<locale>.yaml`): a folder per bundle, a file per locale.

```text
l10n/
└── common/
    ├── en.toml
    ├── pl.toml
    └── pt-BR.toml
```

A bundle file is one flat TOML table of id to pattern. Comments are for translators; multi-line literal strings (`'''`) need no escaping of braces or apostrophes:

```toml
# Simple replacement.
hello = "Hello, {name}!"

# Plural, with an exact match and CLDR categories.
file_count = '''
{count, plural,
  =0 {No files}
  one {# file}
  other {# files}}'''

# Select.
mode_label = "{mode, select, edit {Editing} view {Viewing} other {Idle}}"

# Number formatting.
percent_complete = "{progress, number, percent} complete"
```

A value that isn't a string, a duplicate id, a key with whitespace in it, or a file whose name isn't a locale is a load error, not a guess. TOML was chosen over OpenMW's YAML for the parser, not the syntax: `toml`'s span-preserving `DeTable` gives every key a line, reports every error in a file at once, rejects duplicate keys itself, and pulls in no serde. Nothing public depends on the format; an adapter that reads OpenMW's YAML would be another provider.

## The Grammar

Patterns are ICU MessageFormat, parsed by the crate's own `pattern.rs` (ICU's `MessagePattern` rules, `DOUBLE_OPTIONAL` apostrophes):

- `{name}`: the argument as text. A number takes the locale's default number format, so `1234` is `1,234` in English and `1.234` in German.
- `{name, number, style}`: `integer`, `percent` (times 100, whole number, sign), or a `::` skeleton: `group-off`, `group-min2`, `percent`, `scale/100`, `precision-integer`, `.00` (two required fraction digits), `.0#` (one required, one optional). OpenMW's own files use `:: group-off` and `::.00`.
- `{name, plural, offset:n =0 {…} one {…} other {…}}` and `{name, selectordinal, …}`: `=n` matches the exact value before the offset; the CLDR categories match the value after it; `other` is required. `#` inside a case is the offset number, formatted.
- `{name, select, key {…} other {…}}`.
- `''` is one apostrophe; an apostrophe before `{`, `}`, or (inside a plural case) `#` quotes literal text up to the next single apostrophe; any other apostrophe is literal.
- `date`, `time`, `spellout`, `ordinal`, `duration`, and `choice` are compile errors with a diagnostic. There is no backend for them yet, and silently printing the raw value would be worse.

Plural categories come from `icu_plurals`, number formatting from `icu_decimal` (ICU4X, compiled CLDR data). The parser compiles every one of the 2,787 patterns OpenMW ships without a rejection; the other Rust MessageFormat crates were tried and are written up in the crate's `README.md`.

## Resolution

`Registry::set_preferred_locales(&[pt-BR, en])` expands to a chain: each locale, then its parents (`Locale::parent`: variants first, then region, then script), then the next locale. A message of `common` is looked up in `pt-BR`, `pt`, `en`, then `common`'s fallback locale (`en` unless declared otherwise, OpenMW's default too). This is `MessageBundles::setPreferredLocales` and `formatMessage` in `components/l10n/messagebundles.cpp`, with one extension: OpenMW rebuilds stripped locales from language and country, which loses a script along with a variant; jess keeps `zh-Hant` between `zh-Hant-TW` and `zh`. Locales without a script fall back identically.

The rules that format a message are those of the locale its file came from, not the user's, as in OpenMW (`loadLocaleYaml` builds each `MessageFormat` with its file's locale). A Polish pattern reached through fallback still pluralizes in Polish.

Where OpenMW formats the key itself when nothing defines it, jess returns an error naming the locales tried. `Registry::placeholder` writes `[[bundle:message]]` for a caller that must show something; the error is still the caller's to report.

## Providers and Precedence

Providers are registered in order; a later one overrides an earlier one, message by message, within a bundle and locale. Nothing depends on filesystem enumeration order: two files of one provider for the same bundle and locale (`en.toml` and `EN.toml`) are a collision, reported, and neither loads. Every message remembers its provider, path, line, and column (`Source`), so inspection can answer "which provider supplied this text?" and "is this message missing in Polish, and which file defines it in English?".

`DirectoryProvider` reads a folder on the host. `VfsProvider` reads through anything that lists paths and reads files (`VfsSource`), which is the shape of jess's VFS and of a package. `MemoryFiles` is a `VfsSource` for tests.

## Compiled Once

A reload reads every provider, parses every file, and compiles every pattern of every locale into one arena per bundle generation: four vectors (parts, cases, text, argument slots), a `Pattern` is two spans into them, and the whole bundle is a handful of allocations. Each bundle then gets a resolution table, message index to arena entry, built by walking the chain once. Formatting is: index the bundle, index the table, walk a slice, write into the caller's `String`. A locale switch rebuilds the tables and compiles nothing. A message with no arguments is borrowed straight out of the arena (`Registry::literal`).

Release numbers, from `cargo bench -p jess-l10n` on a bundle of 1,000 messages in four locales:

| Path | Time |
| --- | --- |
| Load and compile 4,000 messages | 4.7 ms (1.2 µs per message, TOML parse included) |
| `literal` (borrowed) | 3 ns |
| Cached format, plain text | 13 ns |
| Cached format, one string argument | 26 ns |
| Cached format, Polish plural | 121 ns |
| Uncached (compile and then format) | 1.1 µs |
| Locale switch, one bundle | 0.9 µs |
| Hot reload of four edited files | 4.5 ms |

## Generations and Last Known Good

A bundle with any error (a file that doesn't parse, a pattern that doesn't compile, a locale that isn't one, a provider collision) is not swapped in; its last good generation stays live, whole, so one bad edit to `pl.toml` doesn't let that reload's `en.toml` through either. A bundle with no good generation yet takes what did compile, so a first load with one bad message still shows the rest. A provider that can't list its files stops the reload and nothing changes. Every swap and every locale change raises the registry's `Generation`, the one number a consumer watches to know its cached text is stale.

## Diagnostics

Every problem is a `Diagnostic`: severity, a kebab-case kind (`pattern-parse-failure`, `duplicate-message`, `provider-collision`, `missing-message`, `fallback-used`, …), the provider, path, line, column, bundle, locale, message, and generation it belongs to, and a sentence. Its `Display` is one line a log can print:

```text
error[pattern-parse-failure] l10n/common/en.toml:6:3 (provider package): common/en 'broken': at offset 33: 'othr' isn't a plural case; use =n, zero, one, two, few, many, or other
```

The column is the bad selector's own, not the message's: the loader knows where the pattern sits in the file and adds the parser's offset.

## Where It Goes From Here

- Date and time arguments, behind the same `Value` boundary, when `icu_datetime`'s data is worth carrying.
- Localized percent sign placement (German writes `42 %`, Turkish `%42`) when ICU4X's percent formatter is stable.
- Package providers, once packages exist: a JPK is a `VfsSource`.
- A GUI that formats on `l10n.changed` instead of caching text per frame. The event, the domain-owned registry, and the `l10n.read`/`l10n.configure` checks are in place ([Application Kernel](@/docs/contributing/application-kernel.md)); the consumer is the GUI pass's.
