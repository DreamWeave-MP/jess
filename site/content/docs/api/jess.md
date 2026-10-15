---
title: 'jess'
description: 'The global table: modules, jess.null, jess.meta, version, jess.select, and require(''jess'').'
weight: 10
extra:
  kind: api
---

{{ api_signature(value="jess: { version, null, meta, plugin, select, save, nif, openmw, luadata, compare, fs, shader, context, worker, input }") }}

`jess` is the global table every script gets, however it was started: `jess run`, `jess test`, `jess plugin modify --script`, or `jess nif modify --script`. It holds one table per module, plus the few values that belong to no module and had to live somewhere.

{% usage_note(title="Conventions") %}
- Functions, methods, table fields, and report codes are camelCase (`check.verdict == "willNotLoad"`). Types are PascalCase.
- Failures raise errors. Uncaught, one stops the script with its message and a stack traceback. `pcall` hands you the message alone, as a string. An outcome isn't a failure: a save that won't load or a blocked strip comes back as a report, because finding that out was the job.
- Record numbers (`jess.meta(record).index`, for plugin and NIF records alike) are 0-based and match `jess plugin dump` and `jess nif dump`. Lists you get back are ordinary 1-based arrays.
- Raw bytes are `buffer`s, and every function that takes bytes also accepts a string.
- A record handle's names are its record's fields and its methods, nothing else; its own metadata is `jess.meta(record)`.
- Plugin names go through your OpenMW config unless you pass `{ openmw = false }`. See [jess.openmw](@/docs/api/openmw.md).
{% end %}

## Module map

| Member | What it's for | Page |
| --- | --- | --- |
| `jess.plugin` | Open, parse, create, check, and save plugins | [jess.plugin and Plugin](@/docs/api/plugin.md) |
| Record handles | Read and edit one record in place | [Record](@/docs/api/record.md) |
| `jess.select`, `plugin:select` | Pick records by type, id, content, or reference | [Selection and Selectors](@/docs/api/selection.md) |
| `jess.save` | Read OpenMW and Morrowind saves; check and strip OpenMW saves | [jess.save and Save](@/docs/api/save.md) |
| `jess.nif` | Check, describe, dump, fix, and edit NIFs | [jess.nif, Nif, and NifRecord](@/docs/api/nif.md) |
| `jess.openmw` | Config contexts, active content, VFS reads | [jess.openmw and OpenMwContext](@/docs/api/openmw.md) |
| `jess.luadata` | OpenMW's serialized Lua data | [jess.luadata](@/docs/api/luadata.md) |
| `jess.compare` | Byte comparisons, common records, diffs, overlapping topics | [jess.compare](@/docs/api/compare.md) |
| `@dream/fs` | Files and folders | [Files](@/docs/api/fs.md) |
| `@jess/data/toml` | TOML documents as tables, and back | [@jess/data/toml](@/docs/api/toml.md) |
| `bytes.regex` | Rust regular expressions | [Regular expressions](@/docs/api/regex.md) |
| `jess.shader` | Typed shader programs from Luau modules | [jess.shader](@/docs/api/shader.md) |
| `jess.input` | Action graphs and a headless input runtime | [jess.input](@/docs/api/input.md) |
| `jess.context` | Handlers for the phases a ScriptDomain schedules | [jess.context](@/docs/api/context.md) |
| `jess.worker` | Isolated Luau workers and their messages | [jess.worker](@/docs/api/worker.md) |
| `@jess/l10n` | Localized text from message bundles | [@jess/l10n](@/docs/api/l10n.md) |
| `LightRecord`, `NiNodeRecord`, ... | Generated record and NIF types | [Record and NIF Types](@/docs/api/record-types.md) |
| `vfstool`, `openmwConfig`, `dreamPath` | The libraries jess is built on | [Downstream Globals](@/docs/api/downstream-globals.md) |

## jess.version

{{ api_signature(value="jess.version: string") }}

The version of jess that's running, such as `"0.1.0"`.

It's the same version `jess --version` prints and the one written at the top of `jess luau-types` output. Check it when a script depends on a feature added in a later release.

```luau
--!strict
print('running under jess ' .. jess.version)
```

## jess.null

{{ api_signature(value="jess.null: JessNull") }}

The value that fills an empty slot in an array, where `nil` would end the array early.

Luau arrays can't hold `nil` in the middle: `{ a, nil, b }` has length 1 or 3 depending on how it was built, and `ipairs` stops at the gap. jess uses `jess.null` wherever a list keeps its positions, most often an empty link in a NIF record's `children`, `properties`, or `effects`. You'll find it in tables from `decode()` and `toTable()`, and you can write it wherever an empty link is allowed.

- **Type:** `JessNull`, an opaque userdata. `typeof(jess.null)` is `"userdata"`.
- **Comparing:** `value == jess.null` works. A live list view reads an empty link as `nil`, and a copied table holds `jess.null` in that slot, so the types spell such a table's items as `number | JessNull`.

```luau
--!strict
local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'Root' }
local node = root :: NiNodeRecord
node.children:push(jess.null) -- an empty link
print(#node.children, node.children[1]) -- 1  nil (views read empty links as nil)
local copy = node.children:toTable()
print(copy[1] == jess.null) -- true (tables keep the slot)
```

Related: [NIF record views](@/docs/api/nif.md#nifrecord), [Record and NIF Types](@/docs/api/record-types.md).

## jess.meta

{{ api_signature(value="jess.meta(record: Record) -> RecordMeta; jess.meta(record: NifRecord) -> NifRecordMeta") }}

What a record handle knows about its record besides the record's own fields: its number, its type, and the plugin or NIF it belongs to.

It's a function and not a field on the handle because the handle has no room for one. Every name on a record handle is a field of its record, so `ltex.index` is an `LTEX`'s texture index and `node.name` a node's name, whatever else the handle knows. Put a record number under `index` on the handle and an `LTEX`'s own `index` becomes unreadable through it, which is a fun thing to find out halfway through merging somebody's terrain.

- **Parameters:** `record`, a plugin record handle or a NIF record handle.
- **Returns:** read-only properties, read when asked, so they follow the record through edits. For a plugin record, a `RecordMeta`: `index`, `tag`, `typeName`, `id`, `size`, `headerFlags`, and `plugin` ([Record: Metadata](@/docs/api/record.md#metadata)). For a NIF record, a `NifRecordMeta`: `index`, `typeName`, and `nif` ([NifRecord: Metadata](@/docs/api/nif.md#nifrecord-metadata)).
- **Errors:** `jess.meta takes a plugin or NIF record handle, not <type>` for anything else, views included. Reading a property of a deleted record raises `record was deleted` (`this record has been deleted` for a NIF's).

What isn't there is the record's data. A light's id is `light.id`, a node's name is `node.name`, and `jess.meta` doesn't copy either.

```luau
--!strict
local plugin = jess.plugin.new()
local ltex = plugin:insert { type = 'LandscapeTexture', id = 'dirt', index = 7, fileName = 'dirt.dds' } :: LandscapeTextureRecord
local meta = jess.meta(ltex)
print(ltex.index, meta.index, meta.tag, meta.typeName, meta.plugin == plugin) -- 7  1  LTEX  LandscapeTexture  true

local nif = jess.nif.new()
local root = nif:insert { type = 'NiNode', name = 'Root' } :: NiNodeRecord
print(root.name, jess.meta(root).index, jess.meta(root).typeName) -- Root  0  NiNode
```

## jess.select

{{ api_signature(value="jess.select(spec: SelectorSpec) -> Selector") }}

Compiles a selector spec once, so you can use it on many plugins.

- **Parameters:** `spec`, a [selector spec](@/docs/api/selection.md#selector-spec) table.
- **Returns:** an opaque `Selector` that `plugin:select` and `plugin:layout` accept in place of a spec.
- **Errors:** an unknown key, an unknown record type, an unknown flag, or an invalid regex. Compiling once means these surface before the first plugin is touched.

```luau
--!strict
local lights = jess.select { type = 'LIGH' }
local plugin = jess.plugin.open 'MyMod.esp'
print(plugin:select(lights):count())
```

Related: [Selection and Selectors](@/docs/api/selection.md).

## require

{{ api_signature(value="require(name: string) -> any") }}

Loads a module the way OpenMW's `require` does, with the jess modules preloaded.

- `require("jess")` returns the `jess` table. `vfstool`, `openmwConfig`, and `dreamPath` load the same way, with or without a leading `@` (`require("@vfstool")`).
- Each of jess's own modules is `@jess/<name>` as well: `require("@jess/shader")` is `jess.shader`, and `require("@jess/plugin")` is `jess.plugin`.
- `coroutine`, `math`, `string`, `table`, `utf8`, and `os` are preloaded, as they are in OpenMW.
- A name starting with `./` or `../` is relative to the script calling `require`, as in Luau's require by string: next to it on disk for a script run from a file (`jess run tools/patch.jsl` finds `require("./util")` at `tools/util.jsl`), or in the VFS for a script loaded from it. jess tries `name.jsl`, `name.lua`, `name/init.jsl`, then `name/init.lua`. An `init` script stands for its folder: its `./` starts beside the folder, and `@self/` inside it.
- Any other name is a path in the VFS of the default OpenMW config, when the script has one ([`jess.openmw.configPath`](@/docs/api/openmw.md#jess-openmw-configpath)). Dots become slashes, and jess tries `name.lua`, `name.jsl`, `name/init.lua`, then `name/init.jsl`: `require("scripts.myMod.util")` loads `scripts/mymod/util.lua`, archives included.
- Each module runs once, called with the name as `...`, and is cached: a module required by two names, such as `./util` and `scripts.myMod.util`, is the same value.
- **Errors:** `module not found: <name>` when nothing matches, or when a VFS name has no config to look in, as in a `jess run` script that hasn't set one.

```luau
--!strict
type Util = { answer: number }
local jessAgain = require 'jess'
local util: Util = require 'scripts.demo.util' -- scripts/demo/util.lua in the VFS
print(util.answer, jessAgain == jess)
```

`require` is typed `(name: string) -> any`: a module's type is whatever you annotate the result with, as with `util: Util` above. The one exception is `require("@jess/shader")`, typed as the [shader module](@/docs/api/shader.md), since a shader module starts with that line.

Related: [Scripts over the OpenMW VFS](@/docs/scripting/openmw-and-vfs.md).

## Globals set by the runner

`jess run script.jsl a b` also sets these globals:

| Global | Value |
| --- | --- |
| `...` | The script's arguments as strings: `"a", "b"`. |
| `arg` | The same arguments as a table, with the script path at `arg[0]`: `{ [0] = "script.jsl", "a", "b" }`. |
| `print` | Luau's `print`, writing through jess's own stdout so `\n` stays `\n` on Windows and lines come out in order. |

Scripts run in other ways get different `...`:

- A [load-order script](@/docs/scripting/load-order-scripts.md) is a module whose function its driver calls with `(state, openmw)`: a table the whole run shares, and an [OpenMwContext](@/docs/api/openmw.md#openmwcontext).
- `jess plugin modify --script` passes `(plugin, path)` and `jess nif modify --script` passes `(nif, path)`. See [Modify Scripts](@/docs/scripting/modify-scripts.md).

```luau
-- jess run args.jsl a b
local first, second = ...
print(first, second) -- a  b
print(#arg, arg[0]) -- 2  args.jsl
```

Both are typed: `...` as the script's varargs, and `arg` as `{ [number]: string }`.

## Type definitions

`jess luau-types types` writes the types for everything on these pages, plus every record and NIF type, and a stub for every module `require` resolves. `jess init` does it for you, along with a strict `.jessrc` aliasing the modules and your editor's luau-lsp settings. Start scripts with `--!strict`: the types were written for the checker, and the checker catches the typo in `data.radiuss` before jess has to. [Record and NIF Types](@/docs/api/record-types.md) explains the generated half.
