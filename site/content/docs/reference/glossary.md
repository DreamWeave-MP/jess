---
title: 'Glossary'
description: 'The words these docs use: VFS, content files, records and subrecords, handles, views, selections, load-order scripts, and the format tags that keep coming up.'
weight: 10
extra:
  kind: reference
---

Morrowind modding has thirty years of vocabulary, some of it Bethesda's, some OpenMW's, some jess's. These are the terms the docs use without stopping. Each entry says which of the three it comes from when that matters.

## Your setup

### openmw.cfg {#openmw-cfg}

OpenMW's config file, where your setup lives: `data=` folders, `content=` files, `fallback-archive=` archives, `data-local=`, `encoding=`, and any [`jess-script=`](#jess-script) lines. Configs chain: a root config's `config=` line pulls in your user config, and later ones add to or override earlier ones. The `jess` CLI doesn't read it; Luau scripts load it with `jess.openmw.load`, and get the whole chain's result. [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md#in-scripts) covers how.

### Data folders {#data-folders}

The `data=` folders in `openmw.cfg`, in order. Each is a folder laid out like Morrowind's `Data Files`: plugins at the top, `meshes/`, `textures/`, `sound/` and so on under it. When two folders have the same file, the later folder wins. Mod managers usually give each mod its own data folder, which is why "where is this file actually coming from" is a real question.

### data-local {#data-local}

One extra data folder, from the `data-local=` line, that OpenMW loads after every `data=` folder. It's the folder for files *you* generate: merged patches, light fixes, anything a tool writes for your load order. Writing there means no mod's own folder gets changed, so updating a mod never clobbers your patch and your patch never corrupts a mod. The load-order tools write there.

### VFS {#vfs}

OpenMW's virtual file system: every data folder and every `fallback-archive=` archive (`.bsa`, `.ba2`) merged into one tree. A VFS path looks like `meshes/f/furn_de_firepit_01.nif`: forward slashes, starting at `meshes/`, `textures/` and friends, looked up ignoring case. Loose files in data folders win over archives, and later data folders win over earlier ones. jess builds the same tree, so a VFS path you give jess finds the file OpenMW would load. Plugins are the exception: they're looked up as loose files only, since OpenMW doesn't load them from archives either.

### Content files {#content-files}

The files OpenMW loads as game data: the `content=` lines in `openmw.cfg`. Mostly [masters](#master) and [plugins](#plugin) (`.esm`, `.esp`, `.omwaddon`, `.omwgame`), plus `.omwscripts`, OpenMW's lists of Lua scripts, which aren't plugins at all. "Active content" is the ones listed; a file in a data folder that isn't listed is installed but inactive.

### Load order {#load-order}

The order content files load in, which decides which one wins when two change the same record: the later one. In OpenMW it's the order of the `content=` lines. In Morrowind.exe it's masters first, then plugins, each by file date. The `main.jsl` that `jess init` writes prints yours.

## Files

### Master {#master}

A content file other files depend on: `Morrowind.esm`, `Tribunal.esm`, `Bloodmoon.esm`, and any `.esm` a mod ships. A plugin names its masters in its header, with each one's size as it was when the plugin was saved. The difference from a plugin is one field in the header (the file type) plus the extension; `jess plugin esm` and `esp` flip it.

### Plugin {#plugin}

A `.esp`: content that depends on masters. Also used loosely, in these docs and the `jess plugin` command group, for any TES3-format file, masters and `.omwaddon`s included, since the format is the same.

### omwaddon {#omwaddon}

OpenMW's extension for a plugin (`.omwaddon`). Same TES3 format, same records, plus a few record types only OpenMW reads. Morrowind.exe won't load one.

### omwgame {#omwgame}

OpenMW's extension for a master that is a whole game rather than an add-on (`.omwgame`). Same format again.

### omwsave, ess {#saves}

Saves: `.omwsave` is OpenMW's, `.ess` is Morrowind.exe's. Both are TES3-format files with records of their own. jess reads both; `save check` and `save strip` work on OpenMW saves only.

## Inside a plugin

### Record {#record}

The unit a plugin is made of: one book, one NPC, one cell, one dialogue line. Each starts with a four-letter [tag](#tag), a size, and flags, followed by its [subrecords](#subrecord). A plugin is a `TES3` header record followed by every other record in order. jess numbers records by position, starting from the header at `0`, and that number is what `record:` means in dumps.

A record is also the unit jess uses to preserve, identify, decode, and rewrite plugin data. Records nobody changed are written back as their original bytes.

### Subrecord {#subrecord}

A tagged field inside a record: `NAME` for the ID, `MODL` for a mesh path, `DATA` for whatever that record type crams into `DATA`. Four-letter tag, size, bytes. The same tag means different things in different record types, and some records repeat a tag, rely on subrecord order, or both. Bethesda did not provide a schema. Bethesda provided evidence.

### Tag {#tag}

A record or subrecord type's four-letter code: `BOOK`, `NPC_`, `CELL`, `LIGH`; `NAME`, `MODL`, `FRMR`. jess takes tags anywhere it takes a record type, in any case, and also takes type names (`Book`, `Light`, `LeveledItem`).

### HEDR {#hedr}

The subrecord at the start of a plugin's `TES3` header: format version, file type (master or plugin), author (32 bytes), description (256 bytes), and the [record count](#record-count). The master list follows it as `MAST`/`DATA` pairs, a master's name and its size in bytes. `jess plugin header` reads and edits all of it.

### Record count {#record-count}

The number in `HEDR` that claims how many records follow the header. Nothing in the game refuses a plugin over a wrong one, but tools trust it. jess keeps it right when it adds or removes records; `plugin header --update-record-count` fixes one that was already wrong.

### FRMR {#frmr}

The subrecord that starts a reference inside a `CELL`: the reference's number, followed by the reference's own subrecords (what object it is, where it stands, whether it's deleted). Everything from one `FRMR` up to the next is one reference. jess matches references by their `FRMR` number, which is how it edits one reference and leaves the others' bytes alone.

### MVRF {#mvrf}

"Moved reference": an `MVRF`/`CNDT` pair in a `CELL` saying the reference after it has moved to another cell. The pair belongs to the `FRMR` that follows it, as OpenMW reads it, so deleting a moved reference in jess deletes its pair too.

### NAM0 {#nam0}

A `CELL` subrecord written before the cell's temporary references, holding how many there are. Persistent references come before it. Removing references means this number should change too; jess handles it when it removes them.

## jess's model

### Byte-identical round trip {#byte-identical-round-trip}

Read a file and write it back without changes, and you get the same bytes. For jess this is a hard rule, not an aspiration: an unedited plugin saves byte for byte, and an edited one differs only in the records the edit touched. Plugins have been through decades of tools with incompatible ideas of "correct", and the odd bytes some of them left behind may mean something the typed model doesn't understand. Preservation is the default. Normalization is a decision.

### Raw-preserving spine {#raw-preserving-spine}

The part of jess (the `jess-plugin` crate) that holds a plugin as its original bytes, split into records and subrecords, and writes untouched records by copying their byte ranges. Everything typed sits on top of it. It's what makes the round trip above possible.

### Record handle {#record-handle}

A Luau (and Rust) reference to one record that survives edits elsewhere in the plugin. A record's number is a position, and positions shift every time something is inserted or deleted in front of it; keep a number as identity and the next insert quietly points it at a different record. A handle is tied to its record instead. When the record is deleted, the handle fails on the next use rather than drifting onto whatever record now sits at that number. Every name on a Luau handle is the record's field or one of its methods; the handle's own record number, tag, and plugin are `jess.meta(record)`. [Records and Views](@/docs/scripting/records-and-views.md) has the details.

### View {#view}

A live window onto part of a record: a handle's fields, a nested struct like `light.data`, or a list. Reading a view reads the plugin; writing one changes it. Nothing is copied until you ask (`toTable()`). The typed view is convenient; the original bytes are authoritative, and a change is encoded back into bytes, minimally, when something next reads them.

### Selection {#selection}

What `plugin:select{...}` returns: "these records in this plugin", described by a spec (type, ID pattern, flags, subrecord and reference filters, the same filters as the CLI's `--type`/`--id`/`--instance-match`). A selection is live. Every use re-evaluates it against the plugin as it is right then.

### Load-order script {#load-order-script}

A Luau patcher that reads a whole load order: a module returning a function that takes `(state, openmw)`, a table shared by everything in the run and a loaded config. A driver script loads the config and calls it, under `jess run`. Patchers (merged leveled lists, light fixes) are load-order scripts, and so are Dreamwind's [load-order tools](@/docs/bundled-scripts/_index.md). [Load-Order Scripts](@/docs/scripting/load-order-scripts.md).

### jess-script= {#jess-script}

An `openmw.cfg` line naming a [load-order script](#load-order-script): a VFS path (`jess-script=scripts/myMod/patch.jsl`) or a [bundled script](#bundled-scripts) (`jess-script=@jess/tes3merge`). OpenMW ignores the lines, so they're safe to keep in your real config. The `jess` CLI doesn't run them; a loaded config lists them as `jessScripts`, and a Rust host built on `jess-lua` can run them in order.

### Bundled scripts (@jess/) {#bundled-scripts}

Load-order scripts compiled into `jess-lua`, named with `@jess/`: `@jess/tes3merge`, `@jess/s3lightfixes`, `@jess/mergedlands`, and `@jess/doctor` (a load-order checkup). The `jess` CLI doesn't run them. They're the same sources as Dreamwind's `@dreamwind/tools/...` modules, which is how you run them from `jess run`. [Bundled Scripts](@/docs/bundled-scripts/_index.md).

### jess-load-scripts {#jess-load-scripts}

The folder a Rust host running `jess-lua`'s [bundled scripts](#bundled-scripts) copies them into and runs them from, so they can be edited: `jess-load-scripts` in [`data-local`](#data-local), or in the first `data=` folder when the config has no `data-local=` line. Each copy starts with a header line recording the hash of what was written; a copy whose hash still matches is replaced when a newer script is bundled, and one you've edited is never touched. The `jess` CLI doesn't use it.

## NIF check terms

### RCN {#rcn}

Root string data (an `NiStringExtraData` on the root node) that tells OpenMW to search the whole tree for the `RootCollisionNode` instead of only the root's children. Without it, a collision node nested one level too deep isn't found, and the render geometry collides instead. `nif fix` adds it.

### NCC {#ncc}

Root string data meaning "no collision, except the camera". The exact case matters: `NCC` keeps camera collision, and `ncc`, `Ncc`, or any other spelling turns off collision entirely. `nif check` reports the wrong case as `lowercase_ncc`; `nif fix` uppercases it.

### MRK {#mrk}

Root string data that hides Morrowind's `Tri EditorMarker` shapes in game. Without it, the markers the Construction Set shows you also draw for the player. `nif fix` adds it.

More on every NIF check and what OpenMW code it follows: [Checking Rules](@/docs/nif/checking-rules.md).
