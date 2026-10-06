---
title: 'Saves in Rust'
description: 'Reading, checking, and stripping saves with jess-save.'
weight: 40
extra:
  kind: guide
---

A save is a TES3 file with more records, so `jess-save` works on the same `RawPlugin` everything else does. It reads OpenMW (`.omwsave`) and Morrowind (`.ess`) saves, and checks and strips OpenMW ones.

```toml
[dependencies]
jess = { version = "0.1", default-features = false, features = ["save"] }
```

## Read, check, strip

```rust
use std::path::Path;

use jess::plugin::LegacyEncoding;
use jess::save::{ConfigSource, StripMode, StripRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new("Caius_Met.omwsave");

    // Which game wrote it is detected; pass Some(SaveKind::OpenMw) to insist.
    let (save, kind) = jess::save::read(path, None, LegacyEncoding::default())?;
    let info = jess::save::info(&save, kind, "Caius_Met.omwsave")?;
    println!("{} in {}, needs {:?}", info.player.name, info.cell, info.dependencies);

    // Does it load with the current load order?
    let check = jess::save::check(path, LegacyEncoding::default(), &ConfigSource::Default)?;
    println!("verdict: {} (loads: {})", check.verdict, check.loads());

    // What would removing Tribunal do?
    let request = StripRequest {
        dependencies: vec!["Tribunal.esm".into()],
        mode: StripMode::DryRun,
        sample_limit: 5,
        config: ConfigSource::Default,
        ..StripRequest::default() // backs up what it writes over; `backup: Backup::Skip` doesn't
    };
    let report = jess::save::strip(path, &request, LegacyEncoding::default())?;
    println!("blocked: {}", report.is_blocked());
    Ok(())
}
```

## Reading: `read`, `parse`, `info`, `screenshot`

- `read(path, kind, encoding)` and `parse(bytes, name, kind, encoding)` return `(RawPlugin, SaveKind)`. `SaveKind::detect` decides: a `SAVE` record means OpenMW, a `GMDT` in the header means Morrowind. Pass a kind to refuse the other one.
- `info(&save, kind, name)` returns `SaveInfo`: format and format version, player (name, level, class), cell, in-game time, time played, dependencies, health. Morrowind saves leave out what they don't record. It's `Serialize`, and it's the same shape `jess save info` and Luau `save.header` print.
- `screenshot(&save, kind, name)` returns the embedded screenshot. OpenMW saves carry a JPEG; Morrowind's raw pixels come back as a TGA.
- `SaveValues` turns records into readable value trees, with OpenMW's serialized Lua data decoded. That's `jess save dump`.

`name` is only for error messages. Every error names the file.

## Checking: `check`, `check_plugin`

`check` answers "will this save load with my load order, and what breaks if it does". The `SaveCheck` it returns:

- `verdict`: `invalid_structure`, `will_not_load`, `broken_state`, `loads_with_warnings`, `loads_cleanly`, or `structure_valid` (no config, so only the file itself was checked). `loads()` is false for the first two.
- `missing_content` (with `strippable`: still in the data paths, so strip can remove it cleanly), `added_content`, `order_changed`.
- `findings`: missing records the engine doesn't quietly drop, with severity and where they sit.
- `dropped_on_load`: what OpenMW will silently throw away, by type.
- `structure_problems`, `warnings`, and whether the format is one strip can edit.

`ConfigSource` says which `openmw.cfg` to compare against: `Default` (OpenMW's own lookup, honouring `OPENMW_CONFIG`/`OPENMW_CONFIG_DIR`), `Path(..)`, or `Disabled`.

## Stripping: `strip`, `strip_plugin`

`strip` removes content files from an OpenMW save without breaking it: dependency lists rewritten, RefNums remapped, and every field that would break the save fixed or refused. The rules, and the OpenMW source lines behind each one, are in [`docs/save-strip.md`](https://github.com/DreamWeave-MP/jess/blob/main/docs/save-strip.md).

A `StripRequest` has:

- `dependencies`: content files to remove, in any order. Dependents go before their masters.
- `mode`: `DryRun` (plan and verify, apply nothing), `InMemory` (apply, write nothing), `InPlace` (overwrite, keeping `.bak`, `.bak1`, ...), or `Output(path)`.
- `sample_limit`: how many example records each part of the report keeps.
- `config`: where to find the content files being removed.

Everything is applied and verified in memory first. If any removal is blocked, nothing is written and `report.is_blocked()` says so, with reasons in the report.

## Saves already in memory

`check_plugin(&save, kind, name, &config)` and `strip_plugin(&save, kind, input, &request)` do the same on a `RawPlugin` you already have, edits included. `strip_plugin` returns the report and, when every removal was applied, the stripped save:

```rust
let (report, stripped) = jess::save::strip_plugin(&save, kind, None, &StripRequest {
    mode: StripMode::InMemory,
    ..request
})?;
if let Some(stripped) = stripped {
    std::fs::write("Stripped.omwsave", stripped.save_bytes())?;
}
```

That's how Luau `jess.save.strip` works on a save handle's unsaved edits.

## Morrowind saves

`.ess` files read fine: `read`, `info`, `screenshot`, and `SaveValues` all work. `check` and `strip` return `SaveError::Unsupported`, because their rules come from OpenMW's loader and a Morrowind save isn't loaded by it.

## Errors

`SaveError` is a `#[non_exhaustive]` enum: `Read`, `Parse`, `NotASave`, `Metadata`, `Unsupported`, `UnknownDependency` (you asked to strip something the save doesn't depend on), `Write`, and `Invalid` for everything else with a message. It implements `std::error::Error` with sources.

## Format versions

OpenMW bumps its save format now and then. `jess_plugin::openmw_save` has the constants: `CURRENT_FORMAT_VERSION` (the newest jess knows), `MIN_MUTATION_FORMAT_VERSION` (the oldest strip will edit), and `FORMAT_HISTORY` (what each bump changed). `SaveCheck::strippable_format` says whether a save's format falls in that range, and strip refuses to edit one that doesn't. How the constants are kept in step is in [Format Notes](@/docs/contributing/format-notes.md).

## See also

- [Stripping a mod out of a save](@/docs/guides/saves.md), the CLI side.
- [Saves from Luau](@/docs/scripting/saves.md).
- [docs.rs/jess-save](https://docs.rs/jess-save).
