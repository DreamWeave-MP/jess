# `jess::l10n`

Localization for jess: named bundles of ICU MessageFormat messages, resolved through a locale
fallback chain, from providers with a fixed precedence. The model is OpenMW's `components/l10n`;
the formatting semantics are ICU's; the implementation is jess's.

```rust
use jess::l10n::{DirectoryProvider, Locale, Registry, Value};

let mut registry = Registry::new();
registry.add_provider(DirectoryProvider::new("base", "my-package/l10n"));
let report = registry.reload();
for diagnostic in &report.diagnostics {
    eprintln!("{diagnostic}");
}

registry.set_preferred_locales(&[Locale::parse("pt-BR")?])?;
let common = registry.bundle("common")?;
let file_count = registry.message(common, "file_count");
let text = registry.format_to_string(file_count, &[("count", Value::Int(3))])?;
```

## The model

| Concept | What it is | OpenMW |
| --- | --- | --- |
| Bundle | A named group of messages: `common`, `jess.editor`, `s3maphore.playlist-editor`. | context |
| Message | A stable id inside a bundle: `file_count`, never the English text. | key |
| Pattern | ICU MessageFormat text for one message in one locale. | the YAML value |
| Provider | Something that offers `<bundle>/<locale>.toml` files: a folder, a VFS, later a package. | the VFS |
| Locale chain | The preferred locales, each followed by its parents, then the bundle's fallback. | `MessageBundles` |
| Generation | A counter that rises when a reload swaps a bundle in or a locale changes. | none |

Files lay out the way OpenMW's do (`Manager::readLangData` opens `l10n/<context>/<locale>.yaml`):

```text
l10n/
└── common/
    ├── en.toml
    ├── pl.toml
    └── pt-BR.toml
```

A bundle file is one flat TOML table of id to pattern:

```toml
# Comments are for translators.
hello = "Hello, {name}!"
file_count = '''
{count, plural,
  =0 {No files}
  one {# file}
  other {# files}}'''
mode_label = "{mode, select, edit {Editing} view {Viewing} other {Idle}}"
percent_complete = "{progress, number, percent} complete"
```

Anything but a string value is a load error, and so is a duplicate id.

## Decisions

### Backend: ICU4X for the rules, a parser of our own for the grammar

The semantic contract is ICU MessageFormat. Nothing in the public API names the backend, so it can
change; this is what it is today and why.

**Chosen:**

- `icu_plurals` 2.3 (Unicode-3.0, MSRV 1.88): CLDR cardinal and ordinal plural rules, compiled
  data for every locale, `PluralRules::category_for` over an integer or a `Decimal`. The proof
  is `format::tests`: Polish `one/few/many/other`, Russian, Arabic's six categories, English
  ordinals, and offsets all match ICU.
- `icu_decimal` 2.3 + `fixed_decimal` 0.7 (Unicode-3.0): locale digits, grouping, decimal
  separator, and sign; the `Decimal` type does half-even rounding and fraction padding the way
  `DecimalFormat` does. Arabic-Egyptian digits, French narrow no-break spaces, and Polish
  minimum-2 grouping are tested.
- `icu_locale_core` 2.3: BCP 47 parsing and canonical casing. Not `icu_locale`: its likely-subtags
  data is large and the fallback OpenMW specifies needs none of it.
- `icu_provider` with `sync`, so the backend's data payloads are `Send + Sync` and the registry
  can be shared.
- `toml` 1.1 with only the `parse` feature: `DeTable::parse_recoverable` gives every key and value
  a span, reports every error in a file at once, and rejects duplicate keys itself. No serde.
- The MessageFormat parser is `pattern.rs`. The grammar is small (arguments, `number` styles,
  `plural`/`selectordinal` with `offset:` and `=n`, `select`, `#`, and ICU's `DOUBLE_OPTIONAL`
  apostrophe rules) and writing it means the compiled form is exactly the flat arena formatting
  wants, errors carry a byte offset the loader turns into a file line and column, and there is no
  intermediate AST to walk. It compiles all 2787 patterns OpenMW ships (53 files, every locale)
  with zero rejections.

Twenty-three crates join the lockfile in all, ICU4X's `zerovec`/`yoke` family included. The
plural and decimal data for every locale is a few hundred kilobytes, baked in; the linker keeps
what's referenced.

**Rejected, after trying them:**

- `mf1-parser` 0.1.7 (MIT/Apache): panics with `todo!()` on `=0` exact cases, on number styles,
  and on a malformed plural. It also depends on ICU4X 1.5, a second copy of the whole stack.
- `formatjs_icu_messageformat_parser` 0.3.1 (MIT, MSRV 1.90): parses everything, with line and
  column errors. It is the FormatJS dialect (`<b>` tags are syntax unless turned off), depends on
  the full `icu` metacrate plus `regex`, `serde_json`, and `indexmap` (112 crates in a lockfile
  of its own), and produces an owned AST of maps and strings we'd still compile into our own
  form. The matching runtime, `formatjs_icu_messageformat` 0.1.3, is BSD-3 and days old.
- `message_format` 0.0.1: a 2016 stub.
- `intl_pluralrules`: plural rules only, its own CLDR snapshot; ICU4X does the same and the
  numbers too.
- `fluent`: not ICU MessageFormat, so it never qualified.

### Format: TOML

By the handoff's criteria: human editable, comments, multi-line literal strings (`'''`) that
need no escaping of `{`, `}`, or apostrophes, duplicate keys rejected by the parser, and a span
for every key so diagnostics point at the line. YAML (OpenMW's choice) makes `{` a flow mapping
unless quoted and has no maintained Rust parser with spans and duplicate-key detection
(`serde_yaml` is deprecated). JSON has no comments and no multi-line strings. Nothing public
depends on the choice; an OpenMW adapter that reads YAML would be another provider.

### Fallback: OpenMW's, with scripts kept

`MessageBundles::setPreferredLocales` (`components/l10n/messagebundles.cpp`) tries each preferred
locale, then the locale without its variant, then without its country, before moving to the
next preferred locale; `MessageBundles::formatMessage` then tries the fallback locale. jess does
the same, with one difference: OpenMW rebuilds the stripped locales from language and country
alone, which drops a script along with the variant; jess strips variants, then region, then
script, so `zh-Hant-TW` reaches `zh-Hant` before `zh`. Locales without a script fall back
identically. The fallback locale is one locale, not a chain, as in OpenMW.

The rules that format a message are those of the locale its file came from, not the user's;
OpenMW builds each `MessageFormat` with its file's locale (`loadLocaleYaml`).

Where OpenMW formats the key as a pattern when nothing defines it, jess returns an error naming
the locales tried; `Registry::placeholder` writes `[[bundle:message]]` for callers that must show
something.

### Compiled once, formatted from tables

Every reload parses every file and compiles every pattern of every locale into one arena per
bundle generation: four vectors (parts, cases, text, argument slots) and a `Pattern` is two
spans into them. A bundle gets one resolution table, message index to arena entry, rebuilt when
the chain or the bundle changes. Formatting is: index the bundle, index the table, walk a slice,
write into the caller's `String`. Argument-free messages are borrowed straight from the arena
(`Registry::literal`).

Release numbers on the bench corpus (1000 messages in four locales; `cargo bench --bench l10n`):

| Path | Time |
| --- | --- |
| Load and compile 4000 messages | 4.7 ms (1.2 µs per message, TOML included) |
| `literal` (borrowed) | 3 ns |
| Cached format, plain text | 13 ns |
| Cached format, one string argument | 26 ns |
| Cached format, Polish plural, integer | 121 ns |
| Cached format, Polish plural, float | 293 ns |
| Cached format, select with two numbers | 175 ns |
| Lookup by name, then format | 48 ns |
| Uncached (compile and format the plural) | 1.08 µs |
| Locale switch, one bundle of 1000 messages | 0.9 µs |
| Hot reload of four edited files | 4.5 ms |
| Rejected hot reload (last good kept) | 3.3 ms |

### Generations and last known good

A reload compiles everything first and swaps a bundle in only if none of its files had an error.
A bundle with an error keeps its last good generation, whole: a bad edit to `pl.toml` doesn't let
that reload's `en.toml` through either, so the two never disagree. A bundle with no good
generation yet takes what did compile, so a first load with one bad message shows the rest. A
provider that can't list its files stops the reload; nothing changes. The registry's generation
rises on every swap and every locale change.

## Not here

- Date and time arguments: `{x, date}` and `{x, time}` are compile errors with a diagnostic, not
  silently text. ICU4X's `icu_datetime` is the obvious backend; its data is large and the
  handoff put it behind the first string path. The boundary (a `Value` variant, an argument
  type) is where it will go.
- Percent sign placement: `percent` writes the sign after the number for every locale. CLDR
  puts a space before it in German and French and the sign first in Turkish; ICU4X's stable API
  has no percent formatter yet.
- Custom `DecimalFormat` patterns (`#,##0.00`) and currency: compile errors pointing at the
  skeleton subset that is supported (`group-off`, `group-min2`, `percent`, `scale/100`,
  `precision-integer`, `.00`/`.0#`).
- A Luau API, a GUI, OpenMW's YAML files, GMST lookups: other crates, other passes.

## License

MIT OR Apache-2.0, like the other jess library crates. ICU4X is Unicode-3.0.
