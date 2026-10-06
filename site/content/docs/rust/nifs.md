---
title: 'NIFs in Rust'
description: 'Reading, writing, and checking NIFs with jess-nif and jess-assets.'
weight: 30
extra:
  kind: guide
---

`jess-nif` has two readers, because there are two jobs:

- **`NiStream`** is the editable model of **Morrowind** NIFs (NetImmerse 4.0.0.2). Load, walk, change, save.
- **`jess_nif::gamebryo`** reads **every version OpenMW reads**, Morrowind through Starfield, the way OpenMW reads it. It's for inspecting and checking, not writing.

On top of those, `jess_nif::check` tells you what OpenMW will do with a file, and `jess-assets` runs that check over folders, archives, or a whole load order.

```toml
[dependencies]
jess = { version = "0.1", default-features = false, features = ["nif", "assets"] }
```

## Editing a Morrowind NIF

```rust
use jess::nif::{NiNode, NiSourceTexture, NiStream, TextureSource, check};

fn main() -> std::io::Result<()> {
    let mut stream = NiStream::from_path("meshes/x/rock.nif")?;

    for node in stream.objects_of_type::<NiNode>() {
        println!("node {}", node.name);
    }

    // Retexture: point every external .tga at a .dds.
    for texture in stream.objects_of_type_mut::<NiSourceTexture>() {
        if let TextureSource::External(file) = &mut texture.source {
            if let Some(stem) = file.strip_suffix(".tga") {
                *file = format!("{stem}.dds");
            }
        }
    }
    let bytes = stream.save_bytes()?;

    // Check the result the way OpenMW would load it.
    let report = check::check("rock.nif", &bytes);
    for finding in &report.findings {
        println!("{finding}");
    }
    println!("won't load: {}", report.wont_load());
    Ok(())
}
```

The model:

- `stream.objects` is a slot map of `NiType`, an enum with a variant per record type. `stream.roots` are the root links.
- Records point at each other with typed `NiLink<T>`s, which are slot-map keys, not indices. Insert and remove all you like; links to surviving records stay valid. A link to a removed record saves as an empty link.
- Record structs nest their base class as `base`, and deref down the chain, so `node.name` on an `NiNode` reaches `NiObjectNET::name`.

Getting at things:

- `objects_of_type::<T>()` / `objects_of_type_mut::<T>()` iterate one type and its subclasses: `objects_of_type::<NiNode>()` includes every `RootCollisionNode` and `NiBSAnimationNode` too.
- `objects_of_type_with_link::<T>()` gives you the links too, so you can hang new records off them.
- `get(link)`, `get_mut(link)`, `get_as::<T, U>(link)` follow a link. `get_all(&links)` follows a list.
- `objects_with_name::<T>("Bip01")` matches names case-insensitively, like the engine.
- `insert(record)` returns the new record's link. `remove(link)` takes it out. `retain_reachable()` drops everything the roots can't reach.
- `geometries::<NiTriShape>()` yields each shape with its world transform; `bounding_box()` and `bounding_sphere()` do the math for you.

Load with `from_path`, `from_bytes`, or `from_path_offset` (a NIF inside an archive, by offset and size). Save with `save_path` or `save_bytes`. A Morrowind NIF you load and save unchanged comes back byte for byte; the real-data tests hold every record of real Morrowind meshes to that.

`jess_nif::glam` is re-exported, since transforms and vertices are `glam` types.

Feed it a NIF from Oblivion and `NiStream` returns an error. That's by design. Use `gamebryo` for those.

## Reading any version: `gamebryo`

`jess_nif::gamebryo` is a port of OpenMW's `components/nif`: records come out as structs named after OpenMW's classes, links are checked after loading like OpenMW's `post`, and it errors where OpenMW would. It's what `jess nif dump` uses for non-Morrowind files and what `check` reads everything with. Its record types are generated from OpenMW's sources, so they're faithful and not much fun to edit. That's fine: it's read-only.

`jess_nif::header::NifHeader::parse(&bytes)` reads just the header of any version, if all you want is "what game is this from".

## Checking: `jess_nif::check`

`check::check(name, &bytes)` returns a `NifReport`. It never fails: a file that won't parse is a finding, not an `Err`.

- `findings`: each a `Finding { severity, code, message, record }`. Codes are stable snake_case identifiers (`unknown_record_type`). Severities, worst first: `WontLoad`, `Broken`, `MissingAsset`, `Collision`, `Performance`, `Info`.
- `worst()`, `wont_load()`: the summary you usually want.
- `textures`: external texture paths as the file spells them. Resolving them is your job (or `jess-assets`'s).
- `stats` (`NifStats`) and `collision` (`CollisionSetup`: where OpenMW builds the collision shape from).

`name` matters: only the file name is used, but a `.kf` is checked as an animation and an `x` prefix means an animated model, same as the engine. Every rule cites the OpenMW loader function it follows; the full list is in [Checking Rules](@/docs/nif/checking-rules.md).

`jess_nif::fix` repairs the findings with one clear fix (nested collision nodes, lowercase `ncc`, visible editor markers, bad skin weights, unreachable records) on a `NiStream`. `jess_nif::info` builds the scene-graph summary `jess nif info` prints.

## Many files: `jess-assets`

`jess_assets::Sources` is "what to check, and where to look things up". Give it files, folders, or archives, and optionally a base VFS (normally the load order) to layer them over:

```rust
use std::path::PathBuf;

use jess::assets::{Sources, check_nifs, is_nif_path};
use jess::nif::check::Severity;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let inputs = [PathBuf::from("Mods/MyMod/Data Files")];
    // No base VFS: just this folder.
    let sources = Sources::with_inputs(None, &[], &inputs, is_nif_path)?;
    let report = check_nifs(&sources);
    println!("{} files, {} won't load", report.summary.files, report.summary.wont_load);
    for file in report.shown(Severity::Broken, false).files {
        println!("{}: {:?}", file.path, file.worst);
    }
    Ok(())
}
```

`with_inputs(base, data, inputs, wanted)`:

- `inputs` are what gets checked. A file or folder inside a `meshes` folder brings its whole data folder along, so textures next to it resolve. Paths that don't exist on disk are treated as VFS paths (`meshes/x/rock.nif`) and looked up in `base`.
- `data` are extra folders or archives to look files up in, without checking them.
- `wanted` filters by VFS path; `is_nif_path` takes `.nif` and `.kf`.
- **No inputs means everything in `base`**: the whole load order.

Later sources win over earlier ones, and every input wins over the base, as if you'd installed it last. `add_plugin_folders` adds the folders your plugins sit in, so a plugin finds the files shipped beside it.

`check_nifs` runs the check in parallel (rayon) and looks up each NIF's textures the way OpenMW does. `report.shown(severity, every_file)` trims it to what you care about; the summary still counts everything.

### The load order as the base

With the `openmw` feature, `OpenMwContext` reads `openmw.cfg` and builds the same VFS the engine does, loose files over archives:

```rust
use jess::assets::{Sources, check_nifs, is_nif_path};
use jess::openmw::{ConfigSource, OpenMwContext};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(context) = OpenMwContext::load(&ConfigSource::Default).map_err(|error| format!("{error:?}"))? else {
        return Ok(()); // no config found
    };
    let sources = Sources::with_inputs(Some(context.full_vfs()), &[], &[], is_nif_path)?;
    println!("{} files checked", check_nifs(&sources).summary.files);
    Ok(())
}
```

That's `jess nif check` with no arguments, give or take the output formatting.

Beyond NIFs, `jess_assets::plugin` finds every file a plugin's records name (models, icons, sounds, voice, book art, Lua scripts, and what MWScript `Say`s and `StreamMusic`s) and checks them, and `jess_assets::paths` is a port of OpenMW's `resourcehelpers.cpp`: how `textures\foo.tga` becomes `textures/foo.dds`, how icons and book art get their prefixes. Every lookup takes an `exists` callback, so the rules work against any VFS you have.

## See also

- [The NIF Model](@/docs/nif/nif-model.md) for the record hierarchy in depth.
- [NIF checking and fixing](@/docs/guides/nif-checking-and-fixing.md) for the CLI side.
- [docs.rs/jess-nif](https://docs.rs/jess-nif) and [docs.rs/jess-assets](https://docs.rs/jess-assets).
