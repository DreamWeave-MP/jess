---
title: 'Architecture'
description: 'The crate map: what lives where and how the pieces fit.'
weight: 20
extra:
  kind: guide
---

jess is layers. Bytes at the bottom, Luau at the top, and each crate only knows about the ones under it:

| Crate | Directory | Depends on (jess crates) |
| --- | --- | --- |
| `jess-macros` | `libs/macros` | none |
| `jess-io` | `libs/bytes_io` | `jess-macros` |
| `jess-luau-types` | `libs/luau_types` | none |
| `jess-plugin` | `libs/plugin` | `jess-io` |
| `jess-openmw` | `libs/openmw` | `jess-plugin` |
| `jess-nif` | `libs/nif` | `jess-io`, `jess-macros` (`nif`), `jess-luau-types` |
| `jess-omwaddon` | `libs/omwaddon` | `jess-plugin`, `jess-io`, `jess-macros` (`tes3`), `jess-luau-types` |
| `jess-save` | `libs/save` | `jess-plugin`, `jess-omwaddon`, `jess-openmw` |
| `jess-assets` | `libs/assets` | `jess-plugin`, `jess-omwaddon`, `jess-openmw`, `jess-nif` |
| `jess-render` | `libs/render` | none (render graph IR) |
| `jess-shader` | `libs/shader` | none (shader IR and WGSL lowering; `jess-lua` binds it as `jess.shader`) |
| `jess-runtime` | `libs/runtime` | none (script domains, contexts, capabilities, and workers over l3i; `jess-lua` binds it as `jess.context` and `jess.worker`) |
| `jess-l10n` | `libs/l10n` | none (localization: bundles, ICU MessageFormat, locale fallback; `jess-lua` binds it as `jess.l10n`) |
| `jess-input` | `libs/input` | none (device truth, action graph, intents; `jess-lua` binds it as `jess.input`) |
| `jess-lua` | `libs/lua` | all of the above except `jess-render` |
| `jess-cli` | `libs/cli` | all of the above |
| `jess` | repository root | re-exports the library crates |

Data moves up through them:

```text
bytes -> framed records -> typed values (on demand) -> views -> Luau tables
```

Nothing gets decoded until someone asks for it, and nothing gets re-encoded unless someone changed it. Decoding every record in a plugin up front would be stupid, and re-encoding every record on save would be worse: it would quietly destroy whatever the typed model doesn't understand.

## The layers

### `jess-io` (`libs/bytes_io`)

Endian-aware `Load` and `Save` traits for binary data, a `Reader`/`Writer` pair, byte-range helpers, and the three legacy code pages Morrowind content is written in. A derive (`jess-macros`, `libs/macros`) generates the boring impls. It knows nothing about TES3 or NIFs, on purpose.

### `jess-plugin` (`libs/plugin`): the raw spine

Frames a plugin into records and subrecords, keeps the original buffer, and remembers byte ranges. An untouched record saves by copying its original range; an edited one is rebuilt alone. That's the whole byte-identical guarantee, and it lives here so nothing above has to think about it.

Also here: TES3 header helpers (`tes3`, including `RecordCount` for edits that remove records), OpenMW save helpers and the save format constants (`openmw_save`), profiles (what a file should be, checked without refusing to parse), the round-trip checker (`codec`), and how master files are found by name (`masters`). `files` is how everything writes: backups, the output-isn't-the-input check, `--no-backup` (`Backup::Skip`), and `.esp`/`.esm` conversion. The CLI, `jess-save`, and Luau all go through it, so a backup is named the same way whoever made it.

### `jess-openmw` (`libs/openmw`)

`OpenMwContext` reads the `openmw.cfg` chain through `openmw-config`, lists and edits the content list (`activate_content` is the check behind Luau's `context:activate`: the file has to be in the data folders, read afresh, unless forced), and builds the VFS through `vfstool_lib`: loose files for content lookups, loose files over the fallback archives for everything else. `OpenMwResolver` loads it the first time a name doesn't exist on disk. It knows nothing about record types, so crates that only need config resolution don't pull them in.

### `jess-omwaddon` (`libs/omwaddon`): typed records

`libs/omwaddon/src/types.rs` is the registry: the `TES3Object` enum, one variant per record tag. `jess-macros` (`libs/macros/src/tes3.rs`, feature `tes3`) reads those tags and the `#[esp_meta]` structs and generates load/save dispatch, field metadata, and the Luau type descriptions. Adding a record type means adding its module and a line in `types.rs`.

On top of the types:

- `view`: `PluginView` and `PluginEditor`, which borrow a `RawPlugin` and decode one record at a time.
- `writeback`: minimal write-back, re-encoding only the subrecords an edit changed.
- `select`, `edit`, `lint`, `compare`, `records`: selectors, deletes and replacements, lint and clean, diffs, and cheap record queries, all shared by the CLI and Luau.
- `symbol`, `display`, `luau` (with `serde`): report codes and their camelCase spellings, readable record trees, record Luau types.

### `jess-nif` (`libs/nif`)

Two readers. `NiStream` is the editable, writable model of Morrowind NIFs; its type universe is in `libs/nif/src/types.rs`, with inheritance and dispatch derived by `jess-macros` (`libs/macros/src/nif.rs`, feature `nif`). `gamebryo` is a read-only port of OpenMW's `components/nif`, for every version OpenMW reads, with its record factory generated from OpenMW's own sources.

`check` (`libs/nif/src/check`) predicts what OpenMW does with a file. Every rule follows OpenMW's `nifosg` and `nifbullet` loaders and cites the function it comes from. `fix` repairs what has one clear fix.

`scene` translates an `NiStream` into what a renderer draws: nodes with world transforms, packed geometry, materials with OpenMW's state, texture references, and skins, each traceable to its source record. It's a view; the stream stays authoritative. [NIF Render Data](@/docs/contributing/nif-render-data.md) covers what it translates and why.

### `jess-assets` (`libs/assets`)

Runs the NIF check over files, folders, archives, or the load order's VFS (`Sources`), and finds every file a plugin refers to. `paths` ports OpenMW's resource lookups (`components/misc/resourcehelpers.cpp`), so `textures\foo.tga` resolves the way the engine resolves it.

### `jess-save` (`libs/save`)

Save check, strip, info, and readable save records, over the raw spine. Stripping plans every edit, applies them in memory, verifies the result, and only then writes. The rules and their engine citations are in `docs/save-strip.md`.

### `jess-runtime` (`libs/runtime`)

`ScriptDomain`: one isolated Luau authority over an l3i runtime, with the contexts that say when its code runs, the capability grant that says what it may do, isolated workers that talk to it in messages, a generation that moves on every restart, and an ownership registry teardown releases. Generic on purpose: no Morrowind, no OpenMW, no GUI. Its `@jess/context` and `@jess/worker` modules are what `jess.context` and `jess.worker` are. [Script Domains](@/docs/contributing/script-domains.md) has the model; `libs/runtime/README.md` the integration contract.

### `jess-input` (`libs/input`)

Input for applications, before there is a GUI to want it. `Control` is Jess's device vocabulary (positional keys, positional gamepad buttons, normalized axes); `InputQueue` and `InputState` are events and current state; `GraphSpec` is the serializable binding document (sources, chords, gates, tap and hold, deadzones, composite vectors, sets, layers) that `ActionGraph::compile` validates and an allocation-free `Evaluator` ticks into `ActionState` and `ActionEvent`s; `IntentRouter` turns action edges into intents with provenance; `presentations` and `conflicts` are data for a future settings screen. Backends (evdev on Linux, gilrs on macOS and Windows) sit behind a trait and features; the default build is headless. [Input Runtime](@/docs/contributing/input-runtime.md) has the model.

### `jess-lua` (`libs/lua`)

The Luau API, an [l3i](https://DreamWeave-MP.github.io/l3i/) extension. `plan.rs` composes it with the downstream libraries' extensions and `jess-runtime`'s into one runtime plan, which assigns every tag, atom, and direct-access slot; `runtime` makes a VM from it with `jess`, the libraries, `require`, and `print` as globals, then sandboxes the globals so builtins run on Luau's fast path.

- `convert.rs` moves values between Rust and Luau through a `serde_value` tree, renaming snake_case to camelCase, keeping NaN and `-0`, and using `jess.null` to hold array slots.
- `plugin.rs` gives plugins handles with stable record handles (slot ids that survive inserts and deletes); `view.rs` gives live field views that read and write in place.
- `nif_edit.rs` is `jess.nif.new/open/openBytes`: Morrowind NIFs as handles.
- `input.rs` is `jess.input`: node authoring functions, compiled graphs, and a headless runtime over `jess-input`.
- `scripts.rs` runs `jess-script=` load-order scripts for Rust hosts (`load_order_scripts`, `run_scripts`, `list_load_order_scripts`; the CLI doesn't use them), script files the way `jess run` does (`run_file`, which also takes a bundled script's name when no file has it), and modify scripts; `BUNDLED` holds the `@jess/...` scripts, all embedded from the Dreamwind package's tool sources (`jpk/dreamwind/src/tools`). A load-order script that returns a function is called as a module.
- `package.rs` is the host side of packages. The format and the model are Luau (`stdlib/jpk.luau`, `stdlib/package.luau`); Rust hands `@jess/package` its file reads, runs `stdlib/bootstrap.luau` for `inspect`, `pack`, `graph`, `test`, and `activate`, and owns the mount: the accepted plan `require` resolves package modules through, under chunk names `@<id>@<version>/<path>` and a cache keyed by id, digest, and path.
- `init.rs` is `jess init`: the script folder, its `.luaurc`, and the editor settings.
- `definitions()` builds `jess.d.luau`: hand-written API types plus generated record and NIF types, every alias an `export type` (a definitions file shares only those). `types.rs` is the folder around it (`jess luau-types`, `jess init`): a stub per name `require` resolves natively, from the runtime's own namespace, `.luaurc` aliases per scope, and stale stubs removed.

### `jess-cli` (`libs/cli`)

The `jess` binary. It parses flags, calls a library function, and prints the result through `libs/cli/src/output.rs`: one YAML, JSON, or TOML document per command, no free text. Commands that write report `written: {path, backup}`.

## Rules the layout enforces

**The CLI is a thin frontend.** If a command needs logic, the logic goes in a library crate and the CLI calls it. That's how `jess plugin lint` and Luau `plugin:lint()` run the same checks, and how `jess nif check` and Luau `nif.check` share one source setup and one severity filter. If you catch yourself parsing records in `libs/cli`, stop and move it down.

**Everything is in Luau.** Whatever the CLI can do, a script can do, with the same reports as tables. A feature that only exists as a flag is half done.

**Policy lives in scripts.** Plugin merging isn't a Rust feature: it's `@dreamwind/tools/tes3merge`, a Luau module over the API. Same for light fixes. The Rust side provides fast, correct primitives (selections, typed edits, record comparison); the scripts decide what to do with them. That keeps the crates small and lets users fork the policy without forking jess.

**Raw first, typed on demand.** Code that doesn't need a decoded record shouldn't decode one. `RecordView::id()` reads one subrecord; `parse_filtered` skips records nobody asked for.

**Licenses follow the links.** Every crate is MIT OR Apache-2.0, and every DreamWeave library it links is too (dream-path, and dream_archive through vfstool, from 1.1.0 on). A new dependency is checked before it goes in; one that isn't permissively licensed doesn't go in.

**Follow the engine, and cite it.** When jess predicts OpenMW's behavior (NIF checks, save strip, asset paths, reference grouping), it follows OpenMW's code and says where. The local checkout at `./openmw` (gitignored) is the reference. See [Format Notes](@/docs/contributing/format-notes.md).
