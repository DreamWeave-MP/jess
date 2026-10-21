---
title: 'Localization'
description: 'What @jess/l10n is, the ICU MessageFormat it speaks, how bundles are laid out and loaded, how locales fall back, and why every pattern compiles exactly once.'
weight: 75
extra:
  kind: guide
---

The first UI toolkit that ships with `text = "Open File"` never recovers: every label after it is
an English string, and translation becomes a retrofit. So the text service comes first.
[`@jess/l10n`](@/docs/api/l10n.md) is that service: it turns a bundle name, a message id, and some
values into a string in the user's language, with plurals and numbers done right, and it does it
from tables it built once.

The model is bundles of messages, one file per locale, resolved through a fallback chain. The
formatting language is ICU MessageFormat. The implementation is Luau in the built-in package
(`jpk/builtin/src/l10n`), and it is small: the native side contributes nothing but
[`@dream/intl`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-intl)'s CLDR
plural rules and decimal formatting, which are reality; what a message means is Luau's.

## What It Is

```text
providers -> reload -> compiled patterns per bundle and locale -> resolution tables -> format
```

| Concept | What it is |
| --- | --- |
| Bundle | A named group of messages: `common`, `jess.editor`, `example.inventory`. |
| Message | A stable id inside a bundle: `file_count`. Never the English text. |
| Pattern | ICU MessageFormat text for one message in one locale. |
| Provider | Something that offers `<bundle>/<locale>.toml` files: a folder, a table of files, a package's payload. |
| Locale chain | The preferred locales, each followed by its parents, then the bundle's fallback locale. |
| Generation | A counter that rises when a reload swaps a bundle in or a locale preference changes. |

| Module | What it owns |
| --- | --- |
| `l10n/locale.jsl` | Locale identity through `@dream/intl`, and the parents a locale falls back through. |
| `l10n/file.jsl` | The bundle file: `@jess/data/toml`'s decoder with key positions, ids, and the errors. |
| `l10n/pattern.jsl` | The MessageFormat compiler: a pattern into parts and argument slots. |
| `l10n/format.jsl` | Formatting compiled parts with one locale's plural rules and number formatters. |
| `l10n/init.jsl` | Providers, the registry, reload, resolution, diagnostics, and bundle handles. |

Nothing in it knows about an application's paths or a GUI; the GUI doesn't exist yet, and when
it does it will get strings from here.

## What It Is Not

- **Not a translation editor, and not machine translation.** It reads files people wrote.
- **Not gettext.** Message ids are semantic (`file_count`), not source strings, and there is no
  string extraction.
- **Not a text engine.** It returns semantic text. Shaping, fonts, and layout belong to the render
  side.
- **Not a plural library for scripts.** A script never pluralizes; it passes `count` and the
  pattern decides.
- **Not a host service.** A registry is a value in one runtime: the module's own, or one
  `l10n.new()` made. Nothing is shared between runtimes and nothing broadcasts a locale change; an
  application that switches locales tells its workers itself.

## The Files

A folder per bundle, a file per locale:

```text
l10n/
└── common/
    ├── en.toml
    ├── pl.toml
    └── pt-BR.toml
```

A bundle file is one flat TOML table of id to pattern. Comments are for translators; multi-line
literal strings (`'''`) need no escaping of braces or apostrophes:

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

A value that isn't a string, a duplicate id, a key with whitespace in it, or a file whose name
isn't a locale is a load error, not a guess. TOML was chosen for what jess's decoder gives back:
`@jess/data/toml` with `positions = true` records where every key and value sits, so an error
lands on its line and column, and it refuses duplicate keys itself. Nothing public depends on the
format; an adapter that reads another layout would be another provider.

## The Grammar

Patterns are ICU MessageFormat, compiled by `pattern.jsl` (ICU's `MessagePattern` rules,
`DOUBLE_OPTIONAL` apostrophes):

- `{name}`: the argument as text. A number takes the locale's default number format, so `1234` is
  `1,234` in English and `1.234` in German.
- `{name, number, style}`: `integer`, `percent` (times 100, whole number, sign), or a `::`
  skeleton: `group-off`, `group-min2`, `percent`, `scale/100`, `precision-integer`, `.00` (two
  required fraction digits), `.0#` (one required, one optional).
- `{name, plural, offset:n =0 {…} one {…} other {…}}` and `{name, selectordinal, …}`: `=n` matches
  the exact value before the offset; the CLDR categories match the value after it; `other` is
  required. `#` inside a case is the offset number, formatted.
- `{name, select, key {…} other {…}}`.
- `''` is one apostrophe; an apostrophe before `{`, `}`, or (inside a plural case) `#` quotes
  literal text up to the next single apostrophe; any other apostrophe is literal.
- `date`, `time`, `spellout`, `ordinal`, `duration`, and `choice` are compile errors with a
  diagnostic. There is no backend for them yet, and silently printing the raw value would be
  worse.

Plural categories and number formatting are `@dream/intl`'s (ICU4X, compiled CLDR data), each
locale's rules and formatters built the first time a pattern needs them and kept. Rounding is half
to even, as ICU's. A scaled number (`percent`, `scale/100`) moves the decimal point in the shortest
digits that read back as the double (`bytes.shortest`), so scaling adds no binary rounding.

## Resolution

`setLocales { 'pt-BR', 'en' }` expands to a chain: each locale, then its parents (variants first,
then region, then script), then the next locale. A message of `common` is looked up in `pt-BR`,
`pt`, `en`, then `common`'s fallback locale (`en` unless declared otherwise). Parents come from the
locale's own tags, so nothing is lost on the way down: `zh-Hant` stays between `zh-Hant-TW` and
`zh`. Locales without a script fall back the same way.

The rules that format a message are those of the locale its file came from, not the user's. A
Polish pattern reached through fallback still pluralizes in Polish.

When nothing defines a message in any locale of the chain, jess raises an error naming the locales
tried rather than printing the id and hoping. `bundle:placeholder(id)` gives `[[bundle:message]]`
for a caller that must show something; the error is still the caller's to report.

## Providers and Precedence

Providers are added in order; a later one overrides an earlier one, message by message, within a
bundle and locale. Nothing depends on filesystem enumeration order: a provider's files are sorted
by path, and two files of one provider for the same bundle and locale (`en.toml` and `EN.toml`)
are a collision, reported, and neither loads. Every message remembers its provider, path, line,
and column, so `bundle:lookup` can answer "which provider supplied this text?" and "is this
message missing in Polish, and which file defines it in English?".

A provider is a table: a name, `resources()` that lists what it offers, and `read(resource)`.
`l10n.directory` reads a folder through `@dream/fs`, so the folder is behind `filesystem.read`
like any other read; `l10n.files` reads a table of paths to text, the shape of a package's
payload ([`jess.payload`](@/docs/api/jess.md#jess-payload)) or of bundles assembled in memory.

## Compiled Once

A reload reads every provider, parses every file, and compiles every pattern of every locale.
Each bundle then gets a resolution table, message id to compiled entry, built by walking the chain
once. Formatting is: look up the entry, walk its parts, concatenate. A locale switch rebuilds the
tables and compiles nothing. A message with no arguments has its text ready (`bundle:literal`).

## Generations and Last Known Good

A bundle with any error (a file that doesn't parse, a pattern that doesn't compile, a locale that
isn't one, a provider collision) is not swapped in; its last good generation stays live, whole,
so one bad edit to `pl.toml` doesn't let that reload's `en.toml` through either. A bundle with no
good generation yet takes what did compile, so a first load with one bad message still shows the
rest. A provider that can't list its files stops the reload and nothing changes. Every swap and
every locale change raises the registry's generation, the one number a consumer watches to know
its cached text is stale.

## Diagnostics

Every problem is a diagnostic table: severity, a kebab-case kind (`pattern-parse-failure`,
`duplicate-message`, `provider-collision`, `missing-message`, …), the provider, path, line,
column, bundle, locale, message, and generation it belongs to, and a sentence. `tostring` gives
one line a log can print:

```text
error[pattern-parse-failure] l10n/common/en.toml:6:3 (provider package): common/en 'broken': at offset 33: 'othr' isn't a plural case; use =n, zero, one, two, few, many, or other
```

The column is the bad selector's own, not the message's: the loader knows where the pattern sits
in the file and adds the compiler's offset.

## Tests

The acceptance cases are `jpk/builtin/tests/l10n/run.jsl`, over the fixtures in `fixtures.jsl`;
`jess package test jpk/builtin` runs them from the source and from the packed JPK.

## Where It Goes From Here

- Date and time arguments, when `@dream/intl` formats them.
- Localized percent sign placement (German writes `42 %`, Turkish `%42`) when ICU4X's percent
  formatter is stable.
- A GUI that formats when the generation moves instead of caching text per frame.
