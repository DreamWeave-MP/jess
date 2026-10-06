## jess

Tools and libraries for Morrowind and OpenMW content: plugins (`.esp`, `.esm`, `.omwaddon`), saves (`.omwsave`, `.ess`), and meshes (`.nif`, `.kf`). Other games' NIFs are read the way OpenMW reads them. The long-term goal is the wider Bethesda ESP family, not one game.

**Documentation: <https://DreamWeave-MP.github.io/jess>**

It's three layers, and you only need the one you use:

- **`jess`, the command line tool.** Inspects, checks, cleans, and edits plugins, saves, and NIFs; checks that every mesh, texture, and sound a plugin names exists; merges a load order.
- **A Luau runtime.** Everything the command line does is a [Luau](https://luau.org) API, and `jess run script.luau` runs a script with all of it loaded.

`jess run` grants optional capabilities only when the host operator asks for them. For example,
`jess run --capability network.transport script.luau` enables `@dream/net` client creation; without
that flag, `net.client` and `net.server` are denied. With the capability, Luau can construct and
drive both sides; the native transport keeps the signing key private inside server userdata. Jess
also exposes `jess run --profile`, `--disassemble FILE`, and `--emit-asm FILE` for script profiling,
textual bytecode disassembly, and generated native assembly without separate Luau tools.
- **Rust crates**, one per job, on crates.io.

jess grew out of [Greatness7](https://github.com/Greatness7)'s [tes3](https://github.com/Greatness7/tes3) library, whose record types, binary IO, and NIF work are still its foundation.

Leave a file alone and jess writes it back byte for byte. Change one record and only that record's bytes change. That's a hard rule, checked against the retail masters and real saves.

### Install

[Release archives](https://github.com/DreamWeave-MP/jess/releases) carry the `jess` binary for Linux, macOS, and Windows. `cargo install jess-cli` builds it from crates.io. [Install](https://DreamWeave-MP.github.io/jess/docs/getting-started/install/) covers both, plus shell completions and man pages.

### CLI

jess reads your `openmw.cfg` the way OpenMW does, so a bare name like `Morrowind.esm` finds the file your game loads, in any of your data folders. `--openmw-cfg <path>` picks a different config; `--no-openmw-config` (or `JESS_NO_OPENMW_CONFIG=1`) turns lookups off and takes every name as a path, which is how you use jess [without OpenMW](https://DreamWeave-MP.github.io/jess/docs/getting-started/without-openmw/).

Every command prints one YAML document (`--format json` or `toml` if a program is reading). Commands that edit a file back it up first, unless you pass `--output` to write elsewhere, and all of them take `--dry-run`.

```sh
# The active load order, and where each plugin was found.
jess openmw content

# Lint a plugin, or clean it into a new file without touching the original.
jess plugin lint MyMod.esp
jess plugin clean --duplicate-records --evil-gmsts --output Cleaned.esp MyMod.esp

# Dump records as YAML, or compare two plugins record by record.
jess plugin dump --type cell --instance-match fargoth Morrowind.esm
jess plugin diff Old.esp New.esp

# Check a save loads with the current load order, or strip content files out of it.
jess save check MySave.omwsave
jess save strip --output Vanilla.omwsave MySave.omwsave Bloodmoon.esm Tribunal.esm

# Check meshes the way OpenMW loads them, and preview the repairs that have one clear fix.
jess nif check "Mods/MyMod/Data Files"
jess nif fix --output "Mods/NIF Fixes" --dry-run

# Check every file a plugin refers to.
jess plugin assets MyMod.esp
```

Coming from tes3cmd: most commands have the same name under `jess plugin`, and [Coming from tes3cmd](https://DreamWeave-MP.github.io/jess/docs/getting-started/coming-from-tes3cmd/) maps all of them, including what's gone on purpose (`undelete`, which destroys cells on OpenMW). The [CLI reference](https://DreamWeave-MP.github.io/jess/docs/cli/) has every flag; the [guides](https://DreamWeave-MP.github.io/jess/docs/guides/) start from the problem.

### Luau

```luau
local tr = jess.plugin.open("Tribunal.esm")
for book in tr:select { type = "book", sub = "Dagoth" } do
  book:edit(function(value) value.name ..= " (Annotated)" end)
end
tr:save("Tribunal-annotated.esm")
```

`select` returns every record matching a filter; each `book` is a handle on one record that keeps pointing at it while the plugin changes around it. `jess init my-scripts` sets up a folder with the API's types for your editor.

Scripts can also run over the whole load order: list them in `openmw.cfg` as `jess-script=` lines, and a bare `jess` runs them in order. Three ship inside jess, ports of [TES3Merge](https://github.com/NullCascade/TES3Merge), [S3LightFixes](https://github.com/DreamWeave-MP/S3LightFixes), and [Merged Lands](https://github.com/Sophialovesmorrowind/OpenMWMergedLands), so `jess-script=@jess/tes3merge` merges your leveled lists and conflicting records with nothing else installed. The port runs [many times faster](https://DreamWeave-MP.github.io/jess/docs/scripting/performance/) than native TES3Merge. And when the game is broken and you'd like to know why, `jess run @jess/doctor` checks the load order (missing masters, files nothing provides, models OpenMW won't load, dirty plugins, changes a later mod throws away) and writes a report with a fix for each.

Start with [Scripting](https://DreamWeave-MP.github.io/jess/docs/scripting/); the [Luau API reference](https://DreamWeave-MP.github.io/jess/docs/api/) has every module and type.

### Rust

Each job is its own crate, usable on its own:

| Crate | What it does |
| --- | --- |
| `jess-plugin` | Holds a plugin as its original bytes, split into records, and writes untouched records back as those bytes. Everything else builds on it. |
| `jess-omwaddon` | Typed Morrowind/OpenMW records, read from and written back into a `jess-plugin` plugin. |
| `jess-nif` | NIF and KF reading, writing, and the OpenMW compatibility check. |
| `jess-openmw` | `openmw.cfg`, the content list, and OpenMW's virtual file system: every data folder and archive merged into one tree of paths. |
| `jess-assets` | The NIF check over files, folders, archives, or the load order, and the files plugins refer to. |
| `jess-save` | OpenMW save checking and stripping, and readable OpenMW and Morrowind save records. |
| `jess-lua` | The Luau API and the bundled scripts; `jess` re-exports it behind the `lua` feature. |
| `jess-io`, `jess-luau-types`, `jess-macros` | Binary IO and legacy text encodings, Luau type generation, and the proc macros. |

The `jess` crate re-exports the libraries behind one feature each:

`Cargo.toml`
```toml
[dependencies]
jess = { version = "0.1", default-features = false, features = ["plugin", "omwaddon"] }
```

The facade has one feature per library crate, named after its module: `plugin`, `omwaddon`, `nif`, and `save` are on by default; `openmw` and `assets` are opt-in, and `serde` turns serde on in every enabled crate that has it. The Luau API is the `jess-lua` crate, not a facade feature.

`main.rs`
```rs
use jess::omwaddon::{KnownRecord, PluginView};
use jess::plugin::RawPlugin;

fn main() -> std::io::Result<()> {
    let bytes = std::fs::read("./Morrowind.esm")?;
    let raw = RawPlugin::parse(bytes).map_err(std::io::Error::other)?;
    let plugin = PluginView::new(&raw);

    for record in plugin.records_of_kind(KnownRecord::Npc) {
        println!("{}", record.id().unwrap_or_default());
    }

    Ok(())
}
```

`jess::plugin::RawPlugin` is the starting point when bytes must survive; `jess::omwaddon::PluginView` and `PluginEditor` read and edit typed records over it. `jess::omwaddon::GeneratedPlugin` is for building a plugin from scratch, and makes no promise about anyone's original bytes.

Until 1.0, a minor version can break things: pin one. [Rust Crates](https://DreamWeave-MP.github.io/jess/docs/rust/) covers each crate in depth.

## License

The libraries (`jess-plugin`, `jess-omwaddon`, `jess-nif`, `jess-openmw`, `jess-assets`, `jess-save`, `jess-io`, `jess-luau-types`, `jess-macros`) and the `jess` facade are licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.

Every crate, `jess-lua` and the `jess` binary included, is MIT OR Apache-2.0. Releases up to and including dream_archive and dream-path 1.0.0 were GPL-3.0; jess depends on their 1.1.0 releases (dream_archive through vfstool), which aren't.
