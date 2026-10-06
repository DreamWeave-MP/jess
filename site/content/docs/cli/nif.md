---
title: 'jess nif'
description: 'check, info, dump, modify, fix.'
weight: 30
extra:
  kind: reference
---

{{ api_signature(value="jess nif <COMMAND>") }}

Check, describe, dump, script, and repair NIF models and KF animations, the way OpenMW loads them. NIFs from every game OpenMW reads can be checked, described, and dumped; only Morrowind's can be modified and fixed.

The group has no options of its own beyond the [global options](@/docs/cli/global-options.md).

## Inputs and lookups

Every `nif` command takes inputs the same way. An input is any of:

- a `.nif` or `.kf` file
- a folder, searched recursively (a mod's `Data Files`, or a `meshes` folder in one)
- an archive: `.bsa` (Morrowind, Oblivion, Skyrim) or `.ba2` (Fallout 4, Starfield), read in place
- a [VFS](@/docs/reference/glossary.md#vfs) path like `meshes/f/furn_de_firepit_01.nif`, found in the `--data` folders and archives

Textures resolve the way OpenMW resolves them: through your inputs first, then the `--data` folders and archives, later ones winning. So checking a mod's folder finds the mod's own textures, and a `--data` pointing at your `Data Files` and `Morrowind.bsa` fills in the rest.

There's no load order behind any of this: jess looks where you tell it to, and nowhere else. A VFS path with no `--data` to find it in is `error: 'meshes/f/furn_de_firepit_01.nif' doesn't exist`. To check everything a load order loads, a script hands [`jess.nif.check`](@/docs/api/nif.md) a loaded config.

## Severities

Findings from `check` (and from [`plugin assets`](@/docs/cli/plugin.md#assets)) have one of six severities, worst first. `--show` takes the kebab-case name; reports print the snake_case one.

| `--show` value | In reports | Means |
| --- | --- | --- |
| `wont-load` | `wont_load` | OpenMW fails to load the file and shows the error marker instead. |
| `broken` | `broken` | Loads, but something in it is dropped or drawn differently than authored. |
| `missing-asset` | `missing_asset` | A texture or other file it names can't be found. |
| `collision` | `collision` | Collision is wrong: things you walk through, or invisible things you bump into. |
| `performance` | `performance` | Costs frame time or memory for no visible benefit. |
| `info` | `info` | Worth knowing; not a problem. |

Every rule, what triggers it, and the OpenMW code it follows: [Checking Rules](@/docs/nif/checking-rules.md).

## check

{{ api_signature(value="jess nif check [--show <LEVEL>] [-v] [--data <PATH>]... <INPUTS>...") }}

| Argument / option | Default | Does |
| --- | --- | --- |
| `<INPUTS>...` | | NIF/KF files, folders, archives, or VFS paths in the `--data` folders to check. At least one. |
| `--show <LEVEL>` | `collision` | The least severe findings to include. Files with nothing at that level or worse are left out. |
| `-v`, `--verbose` | off | Every file and every finding, down to `info`. Overrides `--show`. |
| `--data <PATH>` | | A folder or archive to look textures up in, without checking its NIFs. Repeatable; later ones win, and the inputs win over all of them. |

```sh
jess nif check "Mods/MyMod/Data Files"
jess nif check --show wont-load ~/Games/Morrowind/Data\ Files/Morrowind.bsa
jess nif check --format json MyMod.bsa
jess nif check --data ~/Games/Morrowind/Data\ Files/Morrowind.bsa meshes/f/furn_de_firepit_01.nif
```

```yaml
summary:
  textures_checked: true
  load_order_checked: false
  files: 7506
  deeply_checked: 7506
  wont_load: 2
  by_worst_severity:
    broken: 11
    collision: 46
    info: 220
    missing_asset: 58
    performance: 48
    wont_load: 2
  by_code:
    invisible_shape_collides: 47
    missing_texture: 58
    no_root_nodes: 2
    # ...
files:
- path: meshes/i/tx_crystal_02.nif
  source: /home/you/Games/Morrowind/Data Files/Morrowind.bsa
  version: 4.0.0.2
  game: Morrowind
  deeply_checked: true
  worst: wont_load
  findings:
  - severity: wont_load
    code: no_root_nodes
    message: Found no root nodes
    record: null
  textures: []
  stats:
    records: 1
    # ...
```

`summary` counts everything checked, whatever `--show` hides. `textures_checked` says whether textures were looked up at all (in the inputs' data folders or `--data`). `load_order_checked` is always `false` here: the CLI has no load order, so a texture installed anywhere you didn't name shows as missing. ([`jess.nif.check`](@/docs/api/nif.md) sets it when a script ran the check over a load order.) Each file in `files` has:

| Field | Meaning |
| --- | --- |
| `path`, `source` | The VFS path, and the file, folder, or archive it came from. |
| `version`, `game` | The NIF version and the game it identifies. |
| `deeply_checked` | Whether the whole scene graph was checked. `false` when the file couldn't be fully read (a `wont_load` finding says why). |
| `worst` | The worst severity among its findings. |
| `findings` | `severity`, `code`, `message`, and the `record` index it's about (or `null`): the record a fix changes, when `nif fix` has one. |
| `textures` | Each texture it names: `path` as written, `resolved` VFS path, and the `source` it was found in. |
| `stats` | Record, node, shape, vertex, triangle, collision triangle, controller, and embedded texture counts, and whether it's skinned. |

**Exit code:** `1` when any file won't load (`error: 2 files won't load`), after printing the report. `0` otherwise, whatever else it found. `error: no NIF or KF files to check` when the inputs hold none.

## info

{{ api_signature(value="jess nif info [--no-tree] [--data <PATH>]... <INPUTS>...") }}

| Argument / option | Does |
| --- | --- |
| `<INPUTS>...` | NIF/KF files, folders, archives, or VFS paths to describe. At least one. |
| `--no-tree` | Leave out the scene graph. |
| `--data <PATH>` | A folder or archive to look textures and VFS-path inputs up in. Repeatable. |

Describes each NIF: header, record types, scene graph, collision setup, statistics, and textures with where OpenMW finds them.

```sh
jess nif info meshes/f/furn_de_firepit_01.nif
```

```yaml
files:
- path: meshes/f/furn_de_firepit_01.nif
  source: /home/you/Games/Morrowind/Data Files/Morrowind.bsa
  version: 4.0.0.2
  game: Morrowind
  user_version: 0
  beth_version: null
  author: null
  record_count: 14
  record_types:
    NiMaterialProperty: 2
    NiNode: 1
    NiSourceTexture: 2
    NiTexturingProperty: 2
    NiTriShape: 3
    NiTriShapeData: 3
    RootCollisionNode: 1
  error: null
  collision:
    source: root_collision_node
    node: 11
    hides_markers: false
  stats: { ... }
  textures:
  - path: Tx_stone_darkgrey_01.tga
    resolved: textures/tx_stone_darkgrey_01.dds
    source: /home/you/Games/Morrowind/Data Files/Morrowind.bsa
  # ...
  tree:
  - index: 0
    type: NiNode
    name: Furn_De_Firepit_01
    children:
    - index: 1
      type: NiTriShape
      name: Tri Furn_De_Firepit_01 0
      properties:
      - index: 2
        type: NiTexturingProperty
      # ...
      shape:
        data:
          index: 5
          type: NiTriShapeData
        vertices: 374
        triangles: 556
        skin: null
    # ...
    - index: 11
      type: RootCollisionNode
      name: ''
      hidden: true
      children: [ ... ]
```

`user_version`, `beth_version`, and `author` are filled in for later games' NIFs. `error` is set when the file couldn't be read, with everything else as far as it got. `collision.source` says where collision comes from: a `RootCollisionNode` (with its `node` index), or the render geometry itself. The tree lists a record that appears twice, through a shared or looping link, without its children the second time, and stops descending 256 levels down. Morrowind's deepest NIFs have about 20; a chain of thousands is a broken or hostile file, and printing it would overflow the stack.

## dump

{{ api_signature(value="jess nif dump [-t <TYPE>]... [--index <N>]... [--name <REGEX>] [--list] [--data <PATH>]... <INPUT>") }}

| Argument / option | Does |
| --- | --- |
| `<INPUT>` | One NIF or KF: a file, or a VFS path in the `--data` folders. |
| `-t`, `--type <TYPE>` | Only records of this type, like `NiTriShape`. Repeatable; any may match. |
| `--index <N>` | Only the record at this index. Repeatable. |
| `--name <REGEX>` | Only records whose name matches this case-insensitive regex. |
| `--list` | Only each matching record's `index`, `type`, and `name`. |
| `--data <PATH>` | A folder or archive to find the input's VFS path in. Repeatable; later ones win. |

Every field of every matching record. Links between records are record indices, `null` for an empty link.

```sh
jess nif dump meshes/f/furn_de_firepit_01.nif --type NiSourceTexture
```

```yaml
path: meshes/f/furn_de_firepit_01.nif
source: /home/you/Games/Morrowind/Data Files/Morrowind.bsa
roots:
- 0
records:
- index: 3
  type: NiSourceTexture
  name: ''
  extra_data: null
  controller: null
  source:
    external: Tx_stone_darkgrey_01.tga
  pixel_layout: Palettized4
  use_mipmaps: 'Yes'
  alpha_format: Default
  is_static: true
- index: 8
  # ...
```

Other games' NIFs dump the way OpenMW reads them:

```sh
jess nif dump --data "Oblivion - Meshes.bsa" meshes/clutter/apple01.nif
```

`error: nif dump takes one NIF or KF file` if the input is a folder or archive holding more than one.

## modify

{{ api_signature(value="jess nif modify --script <SCRIPT> [--output <OUTPUT>] [--dry-run] [--no-backup] [--data <PATH>]... <INPUT>") }}

| Argument / option | Does |
| --- | --- |
| `<INPUT>` | One Morrowind NIF or KF: a file, or a VFS path in the `--data` folders. |
| `--script <SCRIPT>` | The Luau script to run. Required. |
| `--output <OUTPUT>` | Write the result here instead of over the input. Required when the input is inside an archive. Missing folders on the way are created. |
| `--dry-run` | Run the script and report whether it changed the NIF; write nothing (`dry_run: true`). |
| `--no-backup` | Don't back up the file written over. |
| `--data <PATH>` | A folder or archive to find the input's VFS path in. Repeatable; later ones win. |

Runs the script with `(nif, path)` as `...`: a NIF handle to edit, and the file's path. If the NIF's bytes changed afterwards, jess writes it, in place with a backup, or to `--output`.

```luau
-- add-ncc.luau: actors walk through it, the camera still collides.
-- Lowercase "ncc" would turn the camera's collision off too; nif check flags that.
local nif = ...
local root = nif:roots()[1]
root.extraData = nif:insert { type = 'NiStringExtraData', value = 'NCC' }
```

```sh
jess nif modify --script add-ncc.luau --output mod/meshes/f/furn_de_firepit_01.nif meshes/f/furn_de_firepit_01.nif
```

```yaml
input: meshes/f/furn_de_firepit_01.nif
path: meshes/f/furn_de_firepit_01.nif
script: add-ncc.luau
changed: true
written:
  path: mod/meshes/f/furn_de_firepit_01.nif
```

`changed: false` and `written: null` when the script left the NIF as it was. Errors: `<path> is a <game> <version> NIF; jess only edits Morrowind (4.0.0.2) NIFs so far`, and `<path> is in an archive; pass --output to write the modified NIF`. The NIF handle API: [Editing NIFs](@/docs/nif/editing.md).

## fix

{{ api_signature(value="jess nif fix [--only <FIX>,...] [--dry-run] [--no-backup] [--output <DIR>] [--data <PATH>]... <INPUTS>...") }}

| Argument / option | Does |
| --- | --- |
| `<INPUTS>...` | NIF/KF files, folders, or archives. At least one. |
| `--only <FIX>` | Only these fixes; comma-separated or repeated. Without it, all of them. |
| `--dry-run` | Report what would change; write nothing. |
| `--no-backup` | Don't back up the files written over. |
| `--output <DIR>` | Write fixed files into this data folder at their VFS paths (`<DIR>/meshes/...`), instead of over the originals. Required for files inside archives. |
| `--data <PATH>` | A folder or archive to look textures up in, without fixing its NIFs. Repeatable; later ones win, and the inputs win over all of them. |

The fixes, each for a problem `check` reports that has exactly one right answer:

| `--only` value | Fixes |
| --- | --- |
| `rcn` | A `RootCollisionNode` nested under a child instead of the root: adds `RCN` string data so OpenMW searches for it instead of colliding with the render geometry. |
| `ncc` | Lowercase `ncc` string data, which turns off *all* collision; uppercases it to `NCC`, which keeps camera collision. |
| `markers` | `Tri EditorMarker` shapes that would draw in game: adds `MRK` string data to hide them. |
| `skin-weights` | Skin weights for vertices the shape doesn't have, which make OpenMW refuse the file: drops them. |
| `unreachable` | Records nothing links to: drops them. |

Without `--output`, each fixed file is written over the original after a backup (`.bak`, `.bak1`, ...); with it, a file already in the folder is backed up the same way. `--no-backup` skips the backups. Only files that needed a fix are touched or reported. Other games' NIFs are skipped silently.

```sh
jess nif fix --dry-run "Mods/MyMod/Data Files"
jess nif fix --output "Mods/NIF Fixes" --dry-run Mods/MyMod/MyMod.bsa   # an archive, into an override folder
jess nif fix --only ncc,markers "Mods/MyMod/Data Files"
```

```yaml
summary:
  files: 1
  fixed: 1
  written: 1
  skipped: 0
  by_fix:
    ncc: 1
files:
- path: meshes/f/furn_de_firepit_01.nif
  source: mod
  fixes:
  - fix: ncc
    message: '"ncc" is now "NCC", so the camera still collides'
    record: 14
  written:
    path: mod/meshes/f/furn_de_firepit_01.nif
    backup: mod/meshes/f/furn_de_firepit_01.nif.bak
  skipped: null
```

| Field | Meaning |
| --- | --- |
| `summary.files` | NIFs looked at. |
| `summary.fixed` | Files with fixes (that would be, on a dry run) applied. |
| `summary.written` | Files written. `0` on a dry run. |
| `summary.skipped` | Files that needed fixing but couldn't be: unreadable, or in an archive without `--output`. |
| `files[].skipped` | Why, for a skipped file. |

**Exit code:** `1` when any file was skipped (`error: 1 file needing fixes wasn't fixed`).

The guide: [Checking and Fixing NIFs](@/docs/guides/nif-checking-and-fixing.md).
