---
title: 'Plugins in Rust'
description: 'RawPlugin, PluginView, PluginEditor, and GeneratedPlugin.'
weight: 20
extra:
  kind: guide
---

Plugins come in two layers. `jess-plugin` frames records and subrecords and keeps the original bytes. `jess-omwaddon` decodes records into structs on top of it, one at a time, and writes back only what you changed. Start from the bottom layer and borrow the top one when you need types.

The structs are convenient. The original bytes are authoritative.

```toml
[dependencies]
jess = { version = "0.1", default-features = false, features = ["plugin", "omwaddon"] }
```

## The raw spine: `RawPlugin`

```rust
use jess::plugin::{RawPlugin, Tag, tes3};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = std::fs::read("MyMod.esp")?;
    let mut plugin = RawPlugin::parse(bytes.clone())?;

    // Untouched: the saved bytes are the bytes you read.
    assert_eq!(plugin.save_bytes(), bytes);

    let header = tes3::header(&plugin)?;
    println!("{} records, author {:?}", header.record_count, header.author);

    // Drop every GMST; everything else is still copied from the original buffer.
    plugin.retain_records(|record| record.tag() != Tag::new(*b"GMST"));
    tes3::update_record_count(&mut plugin)?;
    std::fs::write("Trimmed.esp", plugin.save_bytes())?;
    Ok(())
}
```

`RawPlugin` holds the file's bytes and a list of `RawRecord`s, each a range into them plus its `RawSubrecord`s. It doesn't know what an NPC is. It knows where one starts and ends.

That's the whole trick behind byte-identical saves: `save_bytes()` copies every untouched record straight out of the original buffer. Re-encoding a whole plugin because one record changed would be ridiculous, so it doesn't. Edits (`replace_record_payload`, `replace_subrecord_payload`, `delete_subrecord`, `insert_record`, `delete_record`, `retain_records`, and friends) rebuild only the record they touch. Everything else is a `memcpy`. `record.is_dirty()` tells you which records got rebuilt.

Reading:

- `plugin.records()` is every record in order. `record.tag()`, `record.subrecords()`, `record.subrecords_by_tag(tag)`, `record.range()`.
- `plugin.record_payload(record)`, `plugin.subrecord_payload(subrecord)`, and their `_bytes` twins (header included) give you the bytes, edits applied.
- `record.cell_groups()` splits a `CELL` into its header and its references, one group per reference. A moved reference's `MVRF`/`CNDT` pair goes with the `FRMR` after it, the way OpenMW reads it (see [Format Notes](@/docs/contributing/format-notes.md)).

TES3 header chores live in `jess_plugin::tes3`, not on `RawPlugin`, because later games do headers differently: `header`, `masters`, `set_masters`, `set_master_size`, `update_record_count`, `set_file_type`, `set_author`, `set_description`, and `new_plugin` for an empty one.

### Profiles, encodings, and partial parses

`RawPlugin::parse` reads with `PluginProfile::RAW_TES3` and Windows-1252. For anything else:

```rust
use jess::plugin::{LegacyEncoding, PluginProfile, RawPlugin};

let plugin = RawPlugin::parse_with_options(bytes, PluginProfile::OPENMW_ADDON, LegacyEncoding::Win1251)?;
```

The encoding matters only when something decodes text (typed strings, ids, string-field replacement). Raw bytes are raw bytes. To pick it the way the CLI and Luau do, with the `openmw` feature, `jess::openmw::resolve_encoding(explicit, context)` gives the code page asked for, else the config's `encoding=` (`OpenMwContext::encoding`), else Windows-1252. The profile (`RAW_TES3`, `TES3_VANILLA`, `OPENMW_ADDON`, `OPENMW_SAVE`) says what the file should be; `plugin.validate_profile(profile)` reports where it isn't, without refusing to parse it.

`RawPlugin::parse_filtered` frames every record but only parses the ones you keep, so a mangled record you don't care about can't stop the parse. `Keep::Cell(CellParts { header: true, references: false })` keeps just part of each `CELL`, so a pass over cell names never parses a single reference.

### Writing it out

`std::fs::write` is fine for a file you made. For a file somebody else made, use `jess_plugin::files`, which is what the CLI, `jess-save`, and Luau all write through:

- `write_file(path, &bytes)` copies whatever is already at `path` to `<name>.bak` (then `.bak1`, `.bak2`, ...) before writing, and returns `Written { path, backup }`. A failed backup writes nothing.
- `write_update(input, output, &bytes)` writes an edit to `output`, or back over `input` when there is none, backing up whatever it replaces. `check_output(input, output)` refuses an `output` that is the input; leaving `output` out is how you ask for that, and that way you get a backup.
- `write_file_with` and `write_update_with` take `Backup::Skip` for `--no-backup`.
- `convert_file_type(input, Tes3FileType::Master, &ConvertOptions::default())` is `jess plugin esm`: checks the extension, refuses to overwrite unless `overwrite`, changes only the header's file type, and with `dry_run` writes nothing and reports `written: None`.

Want to prove a file survives? `jess_plugin::codec::check_round_trip(&bytes, &[], false)` parses, saves, and compares. It's what `jess plugin test-codec` and Luau `jess.plugin.checkRoundTrip` run.

## Linting and cleaning

`omwaddon::lint` is `jess plugin lint` and `jess plugin clean` without the printing. Checks that compare a plugin with its masters take a `MasterSource` from `jess_plugin::masters`: `PathMasters` (names are paths), `NoMasters` (those checks report nothing), or `jess_openmw::OpenMwContext` (names resolve through the load order). Wrap any of them in `BesidePlugin::new(&source, plugin_path)` to also look in the plugin's own folder, the way the CLI does.

```rust
use jess::omwaddon::lint::{self, CleanChecks, LintChecks, LintSeverity};
use jess::plugin::RawPlugin;
use jess::plugin::masters::PathMasters;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut plugin = RawPlugin::parse(std::fs::read("MyMod.esp")?)?;

    // Nothing named means the recommended checks, as in `jess plugin lint`.
    let findings = lint::lint(&plugin, LintChecks::default().or_recommended(), &PathMasters)?;
    for finding in &findings {
        println!("{}: {finding}", finding.severity.name());
    }
    if findings.iter().any(|finding| finding.severity.at_least(LintSeverity::Error)) {
        return Err("MyMod.esp is broken for players".into());
    }

    let report = lint::clean(&mut plugin, CleanChecks::all(), &PathMasters)?;
    println!("removed {} records, {} subrecords", report.removed_records, report.removed_subrecords);
    jess::plugin::files::write_file(std::path::Path::new("MyMod.esp"), &plugin.save_bytes())?;
    Ok(())
}
```

Every `LintCheck` has a severity: `Error` for a plugin broken for players (a missing master, expansion functions without the expansion), `Warning` for dirt and likely mistakes, `Info` for things plenty of mods do on purpose. `LintPreset::Recommended` is the checks that break or change the game and rarely fire on purpose; `LintPreset::All` is everything. `LintCheck::parse` and `CleanChecks::parse` take a name in any spelling the CLI or Luau would (`evil-gmsts`, `evil_gmsts`, `evilGmsts`); `CleanChecks::parse` of nothing is every cleanup, like tes3cmd's `clean`.

`clean` and `edit::delete_matching` set the header's record count when they remove records, and leave a plugin they remove nothing from byte for byte. If you want the count left exactly as it was, wrong or not, call `clean_with` or `delete_matching_with` with `jess_plugin::tes3::RecordCount::Keep`.

## Typed records: `PluginView` and `PluginEditor`

`jess-omwaddon` borrows a `RawPlugin` instead of owning a copy. A `PluginView` reads; a `PluginEditor` reads and writes.

```rust
use jess::omwaddon::{KnownRecord, PluginEditor, PluginView, TES3Object};
use jess::plugin::RawPlugin;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut raw = RawPlugin::parse(std::fs::read("MyMod.esp")?)?;

    // Read: a view decodes one record at a time, on demand.
    let view = PluginView::new(&raw);
    for record in view.records_of_kind(KnownRecord::Light) {
        if let Some(TES3Object::Light(light)) = record.decode()? {
            println!("{} radius {}", light.id, light.data.radius);
        }
    }

    // Write: decode, change, put it back.
    let mut editor = PluginEditor::new(&mut raw);
    let lights = editor
        .records_of_kind(KnownRecord::Light)
        .map(|record| record.decode_required())
        .collect::<Result<Vec<_>, _>>()?;
    for mut decoded in lights {
        if let TES3Object::Light(light) = &mut decoded.record {
            light.data.radius *= 2;
        }
        editor.replace_decoded(&decoded)?;
    }
    std::fs::write("Brighter.esp", raw.save_bytes())?;
    Ok(())
}
```

Run that on a plugin and diff it: the lights changed, and nothing else moved a byte.

What you get from a view:

- `records()` is everything, known or not; `known_records()` skips tags jess has no type for; `records_of_kind(KnownRecord::Npc)` filters to one type.
- A `RecordView` has `index()`, `tag()`, `id()`, `flags()`, and `string_fields()` without decoding the whole record. `id()` is cheap. Use it.
- `decode()` gives `Option<TES3Object>` (`None` for unknown tags). `decode_required()` gives a `DecodedRecord { index, kind, record }`, which is what the editor takes back.

`TES3Object` is one enum with a variant per record type (`Npc`, `Cell`, `Light`, `Dialogue`, `DialogueInfo`, `ScriptConfigList` for `LUAL`, ...), and each wraps a plain struct with public fields. The tag-to-variant mapping in `libs/omwaddon/src/types.rs` drives the generated load and save code.

### Minimal write-back

`PluginEditor::replace_record` doesn't re-encode your struct and slap it in. Typed encoding normalizes things that twenty years of tools spelled differently: NUL terminators on names and ids, padding, empty subrecords, reference order. (Free text, like book text and descriptions, keeps its bytes, terminator or not.) Re-encoding the whole record on every edit would rewrite all of that, and a one-field change would show up in a diff as the entire record.

So it encodes both the old and the new value and compares them subrecord by subrecord (`omwaddon::writeback::minimal_record`). Unchanged subrecords keep their original bytes; only the changed ones take the new encoding. `CELL` references are matched by `FRMR` index, so the references you didn't touch keep their bytes and their order. Then it checks its own work: if the patched record doesn't decode to exactly your new value, it falls back to the full new encoding. Being clever is only useful while it stays correct. If nothing changed, nothing is written.

For small jobs there are targeted edits that never decode anything: `replace_string_fields` (regex replace across typed string fields, what `jess plugin modify --replace` uses), `set_cell_name`, and `set_cell_region`. Need something raw mid-edit? `editor.raw_mut()`.

### Errors

`jess_plugin::ParseError` carries a message and a byte `offset()`. Typed code returns `TypedViewError`: `Raw` (a `ParseError`), `Decode` (the record didn't decode), `StringDecode`, `UnknownRecord`, `TagMismatch` (you tried to put a `Light` where a `Static` was), `MissingCellHeaderField`, and `ProfileViolation`. All of them implement `std::error::Error`, so `?` into `Box<dyn Error>` works.

## Building from scratch: `GeneratedPlugin`

For plugins you generate rather than edit, `GeneratedPlugin` is just a `Vec<TES3Object>` that saves:

```rust
use jess::omwaddon::{GeneratedPlugin, Header, Static, TES3Object};

fn main() -> std::io::Result<()> {
    let mut plugin = GeneratedPlugin::new();
    plugin.objects.push(TES3Object::Header(Header::default()));
    plugin.objects.push(TES3Object::Static(Static {
        id: "my_rock".into(),
        mesh: "x\\my_rock.nif".into(),
        ..Default::default()
    }));
    plugin.save_path("Generated.esp")
}
```

`save_path` fills in the header's record count and sets ESP or ESM from the extension. It's a normalizing writer: load a plugin into it and save, and you get jess's encoding of every record, not the original bytes. That's the right call for generated output and the wrong one for editing somebody's mod. Preservation is the default; normalization is a decision. Use the raw spine for other people's files.

## `serde`

With the `serde` feature every record type serializes, which is how `jess plugin dump` prints them. Field names are snake_case; bitflags serialize as strings (`"FEMALE | 0x100"` keeps bits jess has no name for); binary blobs are base64. `omwaddon::display` reshapes a record into the friendlier tree the dumps show.

## See also

- [Crate Overview](@/docs/rust/crates.md) for features.
- [Records and Views](@/docs/scripting/records-and-views.md) for the same model from Luau.
- [docs.rs/jess-plugin](https://docs.rs/jess-plugin) and [docs.rs/jess-omwaddon](https://docs.rs/jess-omwaddon) for everything else: `lint`, `compare`, `select`, `edit`, `records`.
