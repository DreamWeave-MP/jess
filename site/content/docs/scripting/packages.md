---
title: 'Packages'
description: 'Package Sources, JPKs, layers, and activation: how Luau software ships for jess, and Dreamwind, the first real package.'
weight: 96
extra:
  kind: guide
---

Most of what jess does to Morrowind content is Luau now: the importer, TES3Merge, Merged Lands, S3LightFixes. A package is how that Luau ships. It has two forms and one meaning:

- a **Package Source** is a folder with a `jess.toml` at its top: what you edit, test, and keep in git;
- a **JPK** is that folder packed into one file by `jess pack`: what you ship. It is a ZIP, and it is immutable. A changed package is a new JPK;
- a **package** is what jess sees after opening either. The two are the same package, down to their [content digest](@/docs/api/package.md#the-content-digest), and nothing a package's code does may tell them apart.

A package has **layers**, named semantic systems like `dream.morrowind-tools` that a host asks for. A layer **exports** modules, names like `@dreamwind/tools/tes3merge` that `require` resolves: inert API that does nothing until something calls it. A layer can also have **entrypoints**, modules jess runs because the layer was activated: registering a command, starting a system. Asking for layers and getting an **activation plan** back is resolution, and it runs no code. Mounting the plan, so its exports resolve, and running its entrypoints is **activation**, and that is the host's decision, never a side effect of opening something.

```text
Package Source ──jess pack──▶ JPK
        │                      │
        └──── both open as ────┘
                   ▼
                package
         (modules, layers, payload)
                   │  resolve the requested layers: no code runs
                   ▼
             activation plan
                   │  the host activates it
                   ▼
   require('@dreamwind/tools/tes3merge') works
```

The whole package layer is Luau: [`@jess/jpk`](@/docs/api/jpk.md) reads and writes the container, and [`@jess/package`](@/docs/api/package.md) reads manifests, computes digests, packs, and resolves. Rust's part is one seam: the mount that hands an accepted plan to `require`.

## The manifest

`jess.toml` is read and checked without running anything. An unknown key is an error, so a typo can't become a silently ignored setting.

```toml
manifest_version = 1

[package]
id = "com.example.lightbooks"
version = "1.0.0"
jess_api = "1"
include = ["src", "tests"]
tests = ["tests/run.jsl"]

[dependencies]
"com.example.common" = "^1.2"

[modules]
"@lightbooks" = "src/init.jsl"
"@lightbooks/patch" = "src/patch.jsl"
"@lightbooks/register" = "src/register.jsl"

[[layer]]
id = "lightbooks.core"
exports = ["@lightbooks"]

[[layer]]
id = "lightbooks.patcher"
requires = ["lightbooks.core"]
exports = ["@lightbooks/patch"]

[[layer.entrypoint]]
module = "@lightbooks/register"
capabilities = ["vfs.read", "filesystem.write"]
```

Dreamwind's own is [`jpk/dreamwind/jess.toml`](https://github.com/DreamWeave-MP/jess/blob/main/jpk/dreamwind/jess.toml).

| Key | Is |
| --- | --- |
| `manifest_version` | `1`. |
| `package.id` | The package's identity: two or more lowercase words joined by dots, like a reversed domain. Never inferred from a file name. |
| `package.version` | `major.minor.patch`. |
| `package.jess_api` | `"1"`, the package API generation this jess provides. It moves when the package contract breaks, not when jess's version does. |
| `package.include` | The payload: folders and files, relative to the manifest, that ship. `jess.toml` always does, and the tests the manifest names must be among them. Everything else (benchmarks, `.git`, `target/`) doesn't, without an ignore list, and neither do an editor's or a merge tool's leftovers (`*.bak*`, `*~`, `*.orig`) under an included folder. |
| `package.tests` | Test modules for [`jess package test`](@/docs/cli/package.md#jess-package-test). Each must be in the payload, under a folder `include` names: tests and their fixtures ship in the JPK. |
| `dependencies` | Other packages this one's modules use, by id, each with a [version requirement](@/docs/api/package.md#package-satisfies). A dependency makes that package's exports resolvable; it activates none of its layers. |
| `modules` | The package's named modules, each mapped to a `.jsl` file in the payload. Every name is some layer's export or entrypoint; a name nothing uses is an error. Everything else in the payload is private, reachable only by relative `require`. |
| `[[layer]]` | Each layer: its `id`, the layers it `requires`, and the module names it `exports`. |
| `[[layer.entrypoint]]` | Each of the layer's entrypoints: its `module`, and the `capabilities` it needs. |
| `[managed.<id>]` | A managed module an [application](@/docs/scripting/applications.md) loads into its domain: its private `module` file, and the `interface` id it provides, if any. |
| `[targets.<id>]` | Something runnable: its `kind`, `command` (a script run to its end) or `application` (a host of its own, see [Applications](@/docs/scripting/applications.md)), its private `entry`, the `capabilities` it declares, and for a command an optional `command` name. `package.default_target` picks one. |

Paths are canonical: forward slashes, relative, no `..`. A Package Source can't hold symlinks, or two files whose names differ only in case.

## Modules and require

A package's modules are reachable two ways, and only two.

**By exported name.** `require('@dreamwind/tools/tes3merge')` loads the file the manifest maps that name to. A plan's namespace is every export of every layer of every package it selected, activated or not: an export is inert, so making it resolvable starts nothing. Export names are `@scope/...`, lowercase. `@jess/` is jess's, and the only package that may export into it is jess's own [built-in package](#the-built-in-package). A name the host already has (`@dream/vfs`, `@dream/bytes`) is a resolution error, and so is a name two packages export: nothing is chosen by load order, because "whichever loaded last" is not a design. What a package module returns is shared by every module that requires it, so a table it returns is frozen as it returns (shallowly, like `table.freeze`): no package patches another's API for the whole plan, and state a module keeps belongs in a table of its own, not on its export.

**Relatively, from inside the package.** `./`, `../`, and `@self/` work exactly as they do for files on disk: relative to the requiring module, an `init.jsl` standing for its folder. The result has to stay inside the package. A path that climbs out is an error, not a lookup somewhere else:

```text
error: module not found: ../../outside (required by @com.example.greeter@1.0.0/src/escape.jsl): it leaves the package root of com.example.greeter
```

That chunk name, `@<id>@<version>/<path>`, is what every error and traceback shows for package code, so `init.jsl:42` never has to mean "one of eleven `init.jsl`s." Underneath, a loaded module is cached by its package's id and digest and its path, so a different build of the same version is never served from a stale cache.

A package can't list other packages or open their files by path. If it wants another package's code, it declares the dependency and requires an export.

## Layers

A layer names a system, not a file or a package: `dream.tes3`, `dream.netimmerse`, `dream.morrowind-tools`. One package can provide several, and the package providing a layer can change without anyone who asks for it noticing. That's why layer ids aren't package ids: Dreamwind provides `dream.morrowind-tools`; it isn't `dreamwind.tools`.

A layer's `requires` are other layers, resolved recursively, in a deterministic order, with unknown layers and cycles as errors. Package `dependencies` are a different graph: what has to be available so modules resolve. Keeping the two apart is deliberate.

Exports and entrypoints are different things on purpose. `@dreamwind/nif` is a service: open bytes, get a NIF, save it. Requiring it registers nothing, opens nothing, and needs no authority beyond the caller's, because the caller handed it the bytes. A `register-tools` entrypoint is behavior: it runs because `dream.netimmerse-tools` was activated, and an editor that only requires `dream.netimmerse` never sees a NIF command line appear.

So capabilities belong to entrypoints, never to exports. A library module runs with the authority of whatever calls it; an entrypoint is code jess itself runs, and says what it needs. A package can't grant itself anything; the host grants, and when it won't, activation fails before any of the package's code runs, naming the entrypoint, layer, and package that asked. `jess package graph` shows every requirement, and every entrypoint, before anything executes. Activation runs the entrypoints after mounting the plan, each once, in layer order.

**Packages are trusted, for now.** jess activates them under the trusted policy: whatever capabilities an entrypoint requires, it gets. Most of jess's API isn't checked against grants yet (see [script domains](@/docs/contributing/script-domains.md)), so a package can reach anything a script can. A JPK is a package format, not a sandbox; don't run one you wouldn't run as a script.

## Packing, testing, running

```sh
jess pack jpk/dreamwind -o dreamwind.jpk     # the canonical JPK
jess package inspect dreamwind.jpk           # what it is, without running it
jess package graph jpk/dreamwind             # its layers resolved into a plan
jess package test jpk/dreamwind              # its tests, from the source and packed
jess run --package dreamwind.jpk script.jsl # a script with its modules activated
```

[`jess pack` and `jess package`](@/docs/cli/package.md) has every option and what each prints. `jess pack` writes the same bytes from the same source on every machine. `jess package test` is the parity check: the package's tests against the source, then against it packed. If the two disagree, the package is broken, whichever side passed.

## Workspaces

Packages developed together, a tool and the library it uses, are a **workspace**: a `jess-workspace.toml` naming its members, folders that each hold a Package Source. From anywhere inside one, `jess workspace run <target>` resolves the target's package over the other members, with no `--package-path` to assemble, and `jess workspace test` runs the members' tests the same way. What the members depend on beyond each other is pinned in `jess.lock`, by version and digest, and kept as JPKs in `.jess/packages`, so the workspace runs the same packages on every machine and offline. A workspace changes nothing about a package: its digest, its JPK, and what its modules may require are its manifest's alone.

```toml
manifest_version = 1

[workspace]
members = [".", "tools/greeter"]
```

```sh
jess workspace sync --package-path ../vendor   # pin and keep what the members need
jess workspace run greet --trusted -- you      # from anywhere in the workspace
jess workspace test                            # every member's tests
```

[`jess workspace`](@/docs/cli/workspace.md) has the rules: which workspace a folder is in, how a target is named, and how `sync` chooses.

## The built-in package

`jpk/builtin` is jess's own package, `org.dreamweave.jess`: the Luau libraries and tools every runtime can require, shipped the way any Luau ships. Its layers are `jess.style`, exporting [`@jess/style`](@/docs/api/style.md), the [style checker](@/docs/contributing/luau-style.md#the-checker), and `@jess/analysis`; `jess.data`, the codecs (`@jess/json`, [`@jess/data/toml`](@/docs/api/toml.md), `@jess/data/value`, `@jess/yaml`); and `jess.l10n`, [`@jess/l10n`](@/docs/api/l10n.md). Its tests ship in it.

jess carries it. The binary embeds the package's manifest and payload, taken from `jpk/builtin` when jess is built, and its exports resolve in every runtime with nothing activated: `require '@jess/style'` just works, in `jess run`, in a package, in a Rust host's VM.

It's the one package whose modules are `@jess/` names, and the privilege belongs to the copy jess carries, not to the id. A package on disk that calls itself `org.dreamweave.jess` opens only if its manifest and payload are byte for byte the embedded ones; anything else is refused, naming the first file that differs:

```text
error: package: 'mine' says it is jess's built-in package (org.dreamweave.jess), but its 'src/style/rules.jsl' isn't jess's; rebuild jess from this source, or give the package an id of its own
```

A manifest from any other package that names a `@jess/` module is refused as before, and names jess provides itself (`@jess/test`, `@jess/plugin`) stay reserved for jess. The alternative was a scope of its own, `@builtin/style`, which every other package could squat on until it too was reserved.

For working on it, the source is still an ordinary Package Source:

```sh
jess package test jpk/builtin
jess pack jpk/builtin -o builtin.jpk
jess run --package builtin.jpk check-style.jsl libs jpk
```

Testing or activating `jpk/builtin` works while it's the copy the binary embeds. Edit a file and the jess you have refuses it until you rebuild: the embedded copy is made from the source at build time (`include_str!`, so cargo rebuilds when a file changes), and `cargo test -p jess-lua --test package` fails if a payload file isn't in the embedded list, `jess_lua::builtin::FILES`.

## Dreamwind

Dreamwind (`org.dreamweave.dreamwind`, `jpk/dreamwind` in the jess repository) is the first real package: jess's Morrowind and OpenMW tooling, as Luau, in one package with several layers. Everything it has today is exports: libraries and tools that run when something calls them, with that caller's authority. No layer has an entrypoint yet, so none requires a capability.

| Layer | Exports |
| --- | --- |
| `dream.openmw-import` | `@dreamwind/openmw-import-morrowind`: the Morrowind.ini importer, below. |
| `dream.morrowind-tools` | `@dreamwind/tools/tes3merge`, `@dreamwind/tools/merged-lands`, `@dreamwind/tools/s3lightfixes`: the [load-order tools](@/docs/bundled-scripts/_index.md). `@dreamwind/tools/tes3conv`, `@dreamwind/tools/merge-to-master`, `@dreamwind/tools/add-vanilla-refs`: tes3conv's JSON, merge_to_master, and motherJungle's addVanillaRefs, over record lists. |
| `dream.bethesda-archives` | `@dreamwind/archive` and its `/ba2`, `/bsa`, `/bsa/tes3` and `/bsa/tes4` submodules: Bethesda BSAs and BA2s read, looked up, extracted and built in Luau, on [their own page](@/docs/api/dreamwind-archive.md); and `@dreamwind/archive/tool`, [the jobs](@/docs/api/dreamwind-archive-tool.md): info, verify, diff, safe extraction, create and add, each with a plan. |
| `dream.nif` | `@dreamwind/nif`: Morrowind NIFs opened, read, edited, and saved byte for byte. `/gamebryo` reads every version OpenMW reads, Morrowind to Starfield; `/header`, `/check`, `/info`, `/fix`, and `/scene` are the header, OpenMW's compatibility checks, the scene at a glance, the one-clear-fix repairs, and a renderer's view; `/assets` runs check, info, and fix over files, folders, archives, and a load order, with textures found the way OpenMW finds them. |
| `dream.starwind` | `@dreamwind/starwind`: the Starwind Merged Plugin Project's Definitive build, on [its own page](@/docs/scripting/starwind.md). Requires `dream.morrowind-tools`. |
| `dream.tes3` | `@dreamwind/tes3`: TES3 plugins and saves, read and written raw-preserving, [below](#tes3-plugins-and-saves). |
| `dream.tes3cmd` | `@dreamwind/tes3cmd`: what `jess plugin` does to plugins. Requires `dream.tes3`. |
| `dream.saves` | `@dreamwind/saves`: what `jess save` does to saves. Requires `dream.tes3`. |

The upstream tools each have their own version (the importer follows dream-ini 1.0.1, S3LightFixes 0.4.59, Merged Lands 0.2.0); those live in the modules. The package has its own.

### Running Dreamwind

Each tool module returns a function, called with `(state, openmw)`: an options table and a loaded config. A script loads the config and calls it:

```luau
--!strict
local config: string = ...
local openmw = jess.openmw.load(config)
local tes3merge = require '@dreamwind/tools/tes3merge'

tes3merge({}, openmw)
```

```sh
jess run --package dreamwind.jpk merge.jsl ~/.config/openmw/openmw.cfg
```

[Merging and Fixing a Load Order](@/docs/guides/load-order-scripts.md) runs all three, and [Load-Order Scripts](@/docs/scripting/load-order-scripts.md) covers writing your own in the same shape.

## openmw-import-morrowind

`@dreamwind/openmw-import-morrowind` is the `Morrowind.ini` importer, the one OpenMW's installer runs, as a port of [dream-ini](https://github.com/DreamWeave-MP/dream-ini) to Luau. It does what the Rust library did, from the same inputs to the same outputs: the INI parser with its `\n`-only line ends and one trailing `\r`, `;` comments, malformed-section and empty-value warnings, duplicate keys kept in order, windows-1250/1251/1252 and the byte-order marks; the `GameFileN` and `Archive N` sequences that stop at the first missing index; `Morrowind.bsa` first; the launcher's content order (masters, then time, then name backwards); explicit, then configured, then default data folders; OpenMW's config normalization; the same warnings, verbose events, options, and error messages. It used to be a global, `dreamIni`; it isn't any more, and jess no longer links dream-ini. The package is the importer.

```text
jpk/dreamwind/
  jess.toml
  src/openmw-import-morrowind/
    init.jsl              the module: version, the parsers, the imports, the writers
    ini/parser.jsl        the INI and cfg parsers and the cfg serializer
    ini/importer.jsl      the merge: fallback keys, no-sound, content, archives, the overrides
    ini/content_files.jsl content and archive resolution and ordering
    ini/config.jsl        multimaps as openmw-config documents, spelled as given or resolved
    ini/document.jsl      a preserved openmw.cfg: its lines, updated and written back
    ini/writers.jsl       the writers
    ini/options.jsl       option tables and multimaps, checked with the binding's messages
    ini/fallback_keys.jsl the 573 fallback keys, in dream-ini's order
    ini/plugin_header.jsl TES3 header and masters
    ini/storage.jsl       the filesystem questions, one host call each
    ini/types.jsl         the types
  tests/openmw-import-morrowind/
    run.jsl               the module's own cases
    differential.jsl      every case, against the goldens
    golden/                what the dream-ini library answered to every case
```

The importer itself is Luau. It reads INI and plugin-header bytes, metadata, and paths through `@dream/fs`, decodes codepages through `@dream/bytes`, normalizes configs through `openmwConfig`, logs through `@jess/log`, tests through `@jess/test`, and writes files through [`@dream/fs`](@/docs/api/fs.md)'s atomic `writeFile`.

### Importing

`parseIni`, `parseCfg`, `serializeCfg`, `importMaps`, `importPaths`, and `version` are the members `dreamIni` had, with the same option tables (`gameFiles`, `fonts`, `archives`, `verbose`, `encoding`, `dataDirs`, `dataLocal`, `resources`, `userData`, `cfgDir`, and `importMaps`' `iniPath`), the same `{ cfg, text, warnings, events }` result, the same warning kinds (`ignoredEmptyValue`, `malformedIniLine`, `missingGameFile`) and event kinds (`contentFileResolved`, `dataDirAddedForContent`, `archiveResolved`, `dataDirAddedForArchive`), and the same messages. The result also carries `changedKeys`, the cfg keys the import set, which the native binding never returned and the writers below need. `readPluginHeader(path, encoding?)` reads a plugin's name and masters, as the content ordering does.

INI and plugin-header reads use L3i's `@dream/fs`, not the native VFS host. Grant `filesystem.read` when running these jobs. An unreadable input throws `dream.ini: <path>: <OS reason>`; the importer doesn't catch its own API errors or turn them into a second result interface. Configuration loading and the writers below still use the native configuration seam pending its extraction.

```luau
--!strict
-- import-ini.jsl: write an openmw.cfg for jess from a Morrowind.ini. Reads the INI, writes only the output.
local ini = require '@dreamwind/openmw-import-morrowind'
local iniPath: string, outputPath: string = ...
local result = ini.importPaths { ini = iniPath, gameFiles = true }

for _, warning in result.warnings do
  print(`warning: {warning.message}`)
end

ini.saveCfg(result.cfg, outputPath)
print(`wrote {outputPath}`)
```

### Writing

dream-ini's Rust crate could write what it imported, and update an `openmw.cfg` in place without flattening it; its Luau binding exposed neither. The package has both, over `openmwConfig` and `@dream/fs`'s atomic `writeFile`, held to the crate's functions byte for byte (below).

- `saveCfg(cfg, path)` writes a multimap as a new `openmw.cfg`, every value spelled as given, keys in order, ending in openmw-config's serializer stamp (`save_cfg_output_to_path`).
- `saveResolvedCfg(cfg, path)` writes it with its folders resolved against the file's folder, for an output that lives somewhere else (`save_resolved_cfg_to_path`).
- `loadDocument(path)` loads an `openmw.cfg` (a file, or a folder holding one) as a document: its openmw-config chain and its own lines, with their comments (`load_cfg_document`). A missing file is an empty document.
- `applyUpdate(document, result, update?)` applies an import's changed keys to the document (`apply_preserved_cfg_update`). `update` is `{ gameFiles, archives, dataLocal, resources, userData }`: whether the content and archive lists were imported, and the folder overrides, with `archives` on unless turned off, as in the import options.
- `renderDocument(document)` is the document's text (`serialize_preserved_cfg_document`); `saveDocument(document, path)` writes it, through the target's links (`save_preserved_cfg_document_to_path`); `saveResolvedDocument(document, path)` writes the document flattened (`save_resolved_configuration_to_path`).

A preserved document keeps comments, blank lines, and the order of every line the import didn't touch. A changed list (`content`, `fallback-archive`, `fallback`, `no-sound`) gives up its old lines and gets its new ones at the end; a changed singleton (`encoding`, and `data-local`, `resources`, `user-data` when overridden) does the same, from every place it was written; a `data=` folder the chain doesn't have is appended. A removed line takes the comment above it along, since the comment was attached to it. Lines come out `\n`-terminated whatever they went in as, a second `encoding=` takes the first's place, and a stale serializer stamp at the end is dropped. Every write goes through a temporary file beside the target and a rename, so a failed write leaves the old file whole.

```luau
--!strict
-- update-cfg.jsl: import a Morrowind.ini into the openmw.cfg you already have, keeping its lines.
local ini = require '@dreamwind/openmw-import-morrowind'
local iniPath, cfgPath = 'Morrowind.ini', 'openmw.cfg'
local result = ini.importPaths { ini = iniPath, cfg = cfgPath, gameFiles = true }
local document = ini.loadDocument(cfgPath)
ini.applyUpdate(document, result, { gameFiles = true })
print(ini.renderDocument(document))
ini.saveDocument(document, cfgPath)
```

With this `openmw.cfg` beside the script, and a `Morrowind.ini` whose `[General]` has `Disable Audio=0` and whose `[Game Files]` lists `Morrowind.esm` and `Tribunal.esm`:

```text
# Keep this comment.
data="C:/Games/Morrowind/Data Files"

# Written by hand, kept by the import.
content=Old.esp
no-sound=0
fallback=Weather_Sunrise_Time,5
```

```
$ jess run --grant filesystem.read --package jpk/dreamwind update-cfg.jsl
# Keep this comment.
data="C:/Games/Morrowind/Data Files"
fallback=Weather_Sunrise_Time,5
encoding=win1252
no-sound=0
data=/home/you/Games/Morrowind/Data Files
content=Tribunal.esm
content=Morrowind.esm
fallback-archive=Morrowind.bsa
fallback-archive=Tribunal.bsa
```

The first comment and the quoted `data=` line stayed; so did the `fallback=` line, since the INI set no game settings. `content=Old.esp` went, and the comment above it with it, which is what "kept by the import" earned for sitting on a line the import replaced; `no-sound` was set again, so it moved to the end with the other changed keys. That is what dream-ini does, line for line.

### Testing it

```sh
jess package test jpk/dreamwind
cargo test -p jess-lua --test ini_experiment
JESS_BLESS=1 cargo test -p jess-lua --test ini_experiment the_port_matches_the_goldens
```

`tests/openmw-import-morrowind/run.jsl` is the importer's own suite, the edge semantics dream-ini pins, and the manifest names it, so `jess package test` runs it from the source and from the packed JPK. `tests/openmw-import-morrowind/differential.jsl` isn't in the manifest: the Rust harness (`libs/lua/tests/ini_experiment.rs`) builds its fixture folders with distinct file times, hands it the goldens, and runs the writers' comparison, so `cargo test` is how it runs. The harness activates Dreamwind the way `jess run --package` does, and runs the differential suite twice, from the Package Source and from it packed.

### The specification is frozen

dream-ini's Rust library was the executable specification, called side by side with the port on every case; it is now the frozen one. `tests/differential.jsl` runs every input through the port and expects what the library answered, read from `tests/openmw-import-morrowind/golden/<function>.jsonl`, one JSON line per call in the suite's order, each with a fingerprint of the call's input, and the generators' seeds and counts in the header. The cases: the fixture INIs in every combination of cfg, explicit folders, and verbosity (433 imports); 400 generated byte-soup INIs in three encodings from a seeded generator (1212 parses, with every byte after every byte-order mark); 200 generated cfgs, parsed and serialized back; 120 generated INIs with fallback keys, content, archives, and random options; the file-based `importPaths` with and without a cfg. The binding's own option messages (`ini.importMaps.verbose: expected boolean, got string`), which no library call produces, are pinned in the suite as text.

`JESS_BLESS=1` rewrites the goldens from the library, which stays a dev-dependency of `jess-lua` for exactly that: the harness answers every call through the same conversions the binding made, compares the port as it goes, and writes what the library said. A changed generator or seed is reported by the fingerprints and the header rather than compared against the wrong case. The writers are held the same way, without goldens: `the_writers_agree_with_the_library` runs one import through dream-ini's writers from Rust and the package's from Luau over cfgs with comments, blank lines, duplicated keys, spacing, CRLF, a `config=` line, a `replace=` line, and a stale stamp, and compares the written files byte for byte; `a_failed_write_leaves_the_target_whole` points both at a missing folder and a read-only one and checks the target afterwards.

### Numbers

`cargo bench -p jess-lua --bench ini_experiment` times three workloads two ways: the Rust library called from Rust, and the port. Release build, medians of seven batches, load average 2.15, a laptop that was also compiling. (The native binding's column is gone with the binding; when it was measured, under a load average of 0.41, it stood at 358, 1102, 413, 1394, and 3989 µs on these rows, so the port beat it on four of five.)

| Workload | Rust library, from Rust | The port |
| --- | --- | --- |
| parse: the 573 fallback keys (21 KB, 1146 lines) | **260 µs** | 285 µs |
| parse: dream-ini's large INI (127 KB, 6787 lines) | **860 µs** | 1405 µs |
| merge + serialize: 573 keys into an empty cfg | 184 µs | **148 µs** |
| merge + serialize: the large INI into a 256-folder cfg | 762 µs | **648 µs** |
| import from disk: 129 archives over 257 folders | **3817 µs** | 4630 µs |

The port beats the Rust library on both merges: the merge is table work, and a Luau table that stays in Luau costs less than a Rust map that has to be built from Luau tables and pushed back. It trails on the fallback-key parse by a tenth and on the disk import by a fifth. The port as first written was 1.5x to 2x slower everywhere; what closed the gap was translating the fallback keys once at load instead of once per value, walking the INI by position with plain `string.find` (memchr) instead of a substring per line, trimming ASCII byte by byte with the Unicode walk kept for the one value in a million that needs it, and not asking the host to canonicalize folders that aren't there.

Two workloads stay behind, and the reason is measured, not guessed. Parsing the large INI creates three interned strings per key line (the name, the `Section:Name` key, the value) and grows a 2000-key table; in the release VM that is about 300 µs of substrings, 370 µs of key interning, and 600 µs of table growth against 250 µs of finds, a floor of roughly 1.3 ms for any pure-Luau parser of this shape, while the Rust parser does the same string work in 500 µs. The disk import is the same work plus 386 `stat` calls both sides make, two `openmwConfig` round trips both sides make, and 129 `isFileName` checks; the port's remaining 800 µs is Luau doing the bookkeeping dream-ini does in Rust. The primitive that would close the parse gap is a native line tokenizer: one host call over a string or buffer that returns the `(name, value)` string pairs of every `key=value` line, cut at a comment byte, so the strings are created once by the host instead of three times by the script. l3i has `string.split` (one string per line, then a match per line, measured slower) and `@dream/bytes`' `find` (what the port uses), but not that. Nothing in the port is native, and nothing here argues for making part of it native: the numbers are the case for a general tokenizer, if one is ever wanted.

## TES3 plugins and saves

Three layers, stacked: `dream.tes3` is the file format and nothing else, and `dream.tes3cmd` and `dream.saves` are the applications on top of it. The two applications only see `@dreamwind/tes3`'s exports and never each other, so a script that edits saves doesn't drag in a linter. All three are Luau; none of them calls jess-omwaddon, jess-plugin, or jess-save. Those crates and the native [`jess.plugin`](@/docs/api/plugin.md) and [`jess.save`](@/docs/api/save.md) bindings are still in jess, as the oracle the layers are held to (below), until the layers replace them.

### dream.tes3

`tes3.open(bytes)` frames every record and every subrecord of a `.esp`, `.esm`, `.omwaddon`, `.ess`, or `.omwsave` and decodes nothing. `plugin:decode(i)` reads record `i` into the table jess-omwaddon's serde shape gives (field names camelCase), `plugin:encode(value)` writes one back as its `Save` writes it, and `plugin:save()` copies every record nobody touched straight from the original bytes, so an unedited file saves byte for byte and an edited one differs only where it was edited. Beside that: the header, masters and their sizes, record counts, profiles and their validation, ids (`tes3.records.id`, and `idSpan` for a script that would rather intern an id than decode it), the code pages, the readable values `jess plugin dump` prints (`tes3.display`), and the OpenMW save format (format versions and their history, the SAVE record, typed RefIds, the Lua data in `LUAD`).

### dream.tes3cmd

Lint (the 27 checks, presets, and master-dependent checks through a master source), clean, validation, header edits and master sync, selectors and the delete, modify, and extract they drive, dumps and byte layouts, stats, and comparisons (bytes, common records, diffs, overrides, overlapping dialogue topics), and `.esp`/`.esm` conversion. Everything edits a `@dreamwind/tes3` plugin in memory; `tes3cmd.files` writes it back with a backup. Masters come from a master source: `masters.paths()`, `masters.beside(source, pluginPath)`, `masters.memory(files)` for bytes a script already holds, or any table with a `locate`.

```luau
--!strict
-- lint.jsl: lint a plugin, clean it, and say what changed. Reads its argument, writes nothing.
local tes3 = require '@dreamwind/tes3'
local tes3cmd = require '@dreamwind/tes3cmd'
local path: string = ...
local plugin = tes3.open(assert(require('@dream/fs').readFileString(path)))

for _, finding in tes3cmd.lint(plugin, tes3cmd.preset 'all', tes3cmd.masters.none()) do
  print(`{finding.severity} {finding.check}: {finding.message}`)
end

local report = tes3cmd.clean(plugin)
print(`cleaned {report.removedRecords} records, {report.removedSubrecords} subrecords`)
```

```
$ jess run --package jpk/dreamwind lint.jsl Lights.omwaddon
info missingVersion: TES3 description has no Version line
error missingMaster: TestA.esp
error missingMaster: TestB.omwaddon
cleaned 0 records, 0 subrecords
```

### dream.saves

`saves.open` and `saves.parse` read OpenMW and Morrowind saves; `info`, `screenshot` (a JPEG, or a TGA built from a Morrowind save's pixels), `values` (what `jess save dump` prints), and `luaData` describe them; `validate` checks an OpenMW save's own structure. `check` and `strip` need a load order, which is a value, not a config jess goes looking for: `saves.loadOrder.fromOpenMw(context)` makes one from a [`jess.openmw`](@/docs/api/openmw.md) context, and a script that has its content files some other way builds the table itself (`content`, `resolve`, and optionally `read` and `encoding`). `strip` plans each removal, applies it in memory, verifies it, and only then writes, with every blocker, remap, and verification jess-save has; a blocked strip writes nothing and says why.

```luau
--!strict
-- strip.jsl: what removing a content file from a save would take. Reads, writes nothing.
local saves = require '@dreamwind/saves'
local path: string, config: string, dependency: string = ...
local save = saves.open(path)
local order = saves.loadOrder.fromOpenMw(jess.openmw.load(config))
print(saves.check(save, { loadOrder = order }).verdict)

local report = saves.strip(save, { dependency }, { loadOrder = order, mode = 'dryRun' })
for _, step in report.steps :: { any } do
  print(step.dependency, step.mutationReadiness.reason)
  for _, blocker in step.blockers.hard do
    print(`  {blocker.kind} x{blocker.count}: {blocker.action}`)
  end
end
```

```
$ jess run --package jpk/dreamwind strip.jsl Jetpack.omwsave openmw.cfg Starwind.omwaddon
brokenState
Starwind.omwaddon	blockedPlannedEditsPresent
  targetContentFileRefnums x4: deleteOrRewriteReferencingSaveStateBeforeRemovingDependency
  targetRefIdsWithIncompletePriorProvenance x214: makeTheOtherContentFilesReadableThroughTheOpenmwConfigOrRemoveThemToo
```

Reports are shaped the way `jess.save.check` and `jess.save.strip` return them: keys and codes camelCase.

### Held to the oracle

`libs/lua/tests/tes3_experiment.rs` runs each layer's own suite (the manifest names them too, so `jess package test jpk/dreamwind` runs them from the source and packed) and a differential suite per layer against the native binding, live, and against goldens frozen from it, so the comparison survives the binding's deletion:

- `dream.tes3`: every record of every fixture decoded and encoded as jess-omwaddon does, and every fixture saved byte for byte.
- `dream.tes3cmd`: lint, clean, stats, every record's readable value, validation in three profiles, sixteen selectors through ids, dumps, layouts, extract, delete, and four replacements, header edits, record counts, common records, diffs, and overrides, on ten synthetic plugins and jess's CLI fixtures.
- `dream.saves`: info, screenshot, dump, Lua data, validation, checks with and without a load order, and dry-run and in-memory strips, on 39 scenarios that rebuild the CLI's save tests, each with its content files and openmw.cfg.

`JESS_DATA_FILES` adds the retail masters to the first two (live only; they're too big to freeze), and `JESS_SAVES` (with `JESS_SAVES_CONFIG` for the load order) adds real saves to the first and third. `JESS_BLESS=1` rewrites the goldens.

### Numbers

`cargo bench -p jess-lua --bench tes3_experiment` times each workload three ways: the Rust libraries called from Rust, the native binding from Luau, and the port. Verified release build, medians of seven rounds, over Morrowind, Tribunal, and Bloodmoon (89 MiB) and five Starwind saves, on a laptop with a load average of 2.1 to 3.5 from other builds. Wall time is that noisy; cycles, counted from the CPU's own counters the way l3i's instruction bench counts them, much less so. Milliseconds, then millions of cycles:

| Workload | Rust library, from Rust | Native binding | The port |
| --- | --- | --- | --- |
| open: frame every record and subrecord | 54 / 100 | 53 / 87 | **18** / 68 |
| ids: open, every record's id | 54 / 120 | 93 / 244 | **47** / 173 |
| lazy decode: open, the LIGH records | 52 / 104 | 72 / 205 | **25** / 100 |
| full decode: open, every record | **182** / 580 | 1731 / 6765 | 1034 / 4049 |
| encode: decode and encode every record | **278** / 966 | 7026 / 27062 | 4250 / 15699 |
| round trip: open, save untouched | 120 / 167 | 197 / 241 | **64** / 160 |
| lint: every check | **169** / 472 | 181 / 506 | 282 / 866 |
| clean: every cleanup, save | 352 / 1067 | 367 / 1143 | **210** / 802 |
| header: author, description, recount, save | 105 / 155 | 154 / 237 | **76** / 176 |
| header sync: read, sync master sizes, save | **79** / 102 | 148 / 190 | 153 / 235 |
| dump: every record's readable value | **1161** / 4321 | 2316 / 8290 | 1937 / 6753 |
| stats | 190 / 625 | 191 / 618 | **131** / 495 |
| save info | 1.07 / 4.3 | 1.23 / 4.8 | **0.56** / 2.2 |
| save check: against the load order | 178 / 569 | **172** / 537 | 193 / 602 |
| save strip: dry run of the last content file | **278** / 901 | 313 / 913 | 334 / 1133 |

The Rust column decodes into Rust values, so its decode, encode, and dump rows aren't the same job: anything a script does with a record pays for Luau tables, and against the binding, which does, the port wins those by 1.2x to 1.7x. Against the binding it also wins opening, ids, round trips, clean, header edits, stats, and save info. Opening is the one host call that moved those: `tes3.open` frames every record and checks every subrecord with `@dream/bytes`' `frame`, one native walk that hands back each record's start and end.

It loses four, by less than before. Lint went from 300 to 282 ms and from 2023 to 1315 million instructions: every pattern check (the scripts' deprecated leveled lists and `GetSoundPlaying`, dialogue teleports, autocalc spells, `MenuMode`, the expansion functions) now runs as one `bytes.regex` `matchSpans` call over the records' bytes where they lie, and only the records it finds are decoded and checked exactly. What's left is the whole-plugin checks, duplicate records and info, and modified info ids. They intern an id or a text for every record, a call per record that jess-omwaddon spreads over every core with rayon. Header sync is a tie with the binding on time. Check and strip spend about half their time reading and opening each content file in the load order and interning its ids.

Splitting lint across [workers](@/docs/api/worker.md) doesn't pay. `tes3cmd.lintSlice` lints the per-record checks of a slice of the records, and `tes3cmd.lint` joins slices linted elsewhere with its own whole-plugin checks. The bench runs that in a script domain, with each worker reading and opening the file itself. Against the same read, open, and lint on one runtime (249 ms), two warm workers take 227 ms, one takes 239, and four take 244. Spawned for the run instead of kept warm, every worker count is slower than serial (333 to 415 ms): a worker is a whole jess runtime with Dreamwind activated. The per-record checks are cheap now, so the whole-plugin checks on the domain's own thread are the critical path, and more workers only add file reads. `jess run` doesn't make a script domain at all, so `worker.spawn` fails there; the split runs only in a Rust host that makes one.
