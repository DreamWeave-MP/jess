---
title: 'OpenMW Config and the VFS'
description: 'Load configs, list content, read files through the VFS, and change what''s active.'
weight: 80
extra:
  kind: guide
---

jess reads `openmw.cfg` the way OpenMW does: the same file lookup, the same `config=` chains, the same data folders and archives in the same order. From a script you get all of that as an **OpenMW context**: what's active, where each file lives, and a way to read any file the game would see.

## Getting a context

```luau
--!strict
local config: string = ...
local openmw = jess.openmw.load(config)

print('config folder:', openmw.userConfigDir)
print('data-local:', openmw.dataLocal)
print(`{#openmw.dataDirs} data folders, {#openmw.fallbackArchives} archives`)

for _, content in openmw:content() do
  print(content.name, content.path or '(not found)')
end
```

```
config folder:	/home/you/.config/openmw
data-local:	/home/you/.local/share/openmw/data
2 data folders, 1 archives
Morrowind.esm	/home/you/Games/Morrowind/Data Files/Morrowind.esm
Tribunal.esm	/home/you/Games/Morrowind/Data Files/Tribunal.esm
Bloodmoon.esm	/home/you/Games/Morrowind/Data Files/Bloodmoon.esm
```

`jess.openmw.load(path)` loads a config, from an `openmw.cfg` or the folder it's in, and raises an error if it can't. Load-order scripts get the context their driver loaded as their second input.

`jess.openmw.default()` is the script's default config, the one names resolve through. A `jess run` script starts without one, so it's `nil` until the script calls `jess.openmw.configPath` ([below](#which-config-names-use)). A Rust host embedding jess's Luau runtime picks the starting default itself.

A context's fields:

| Field | What it is |
| --- | --- |
| `dataDirs` | Every data folder, lowest priority first, `data-local` last. |
| `fallbackArchives` | The `fallback-archive=` BSAs, by name. |
| `dataLocal` | The `data-local=` folder, or `nil` if the config doesn't set one. |
| `encoding` | The config's `encoding=` (`"win1251"` and so on), or `nil` if it doesn't set one. Plugins and saves opened through this config default to it. |
| `userConfigDir` | The folder of the last `openmw.cfg` in the chain: the user's, where tools keep their settings and logs. |
| `jessScripts` | The config's `jess-script=` lines, in config order: VFS paths (lowercased, forward slashes) or `@jess/...` names. The CLI doesn't run them; they're there for a script or host that does. |
| `loadScriptsDir` | The `jess-load-scripts` folder in `data-local`, else in the first `data=` folder, where a Rust host running jess-lua's bundled load-order scripts keeps their editable copies; `nil` when there's neither. |

## Content and files

`openmw:content()` lists the active content files in load order, each with its `name` and the `path` it resolved to, or `nil` when no data folder has it. `.omwscripts` files are content too, and show up in the list.

For other files there are two lookups, and the difference matters:

```luau
--!strict
local config: string = ...
local openmw = jess.openmw.load(config)

-- resolve: the loose file on disk, if there is one.
print(openmw:resolve 'Morrowind.esm')
print(openmw:resolve 'meshes/f/furn_de_ex_bench_01.nif')

-- readFile: the file's bytes, from a folder or an archive, whichever wins.
local mesh = openmw:readFile 'meshes/f/furn_de_ex_bench_01.nif'
if mesh then
  print(buffer.len(mesh), 'bytes')
  print(jess.nif.checkBytes(mesh, 'furn_de_ex_bench_01.nif').worst or 'no findings')
end
```

```
/home/you/Games/Morrowind/Data Files/Morrowind.esm
nil
11097	bytes
no findings
```

- `resolve(name)` finds a **loose file** in the data folders and returns its path on disk, or `nil`. The bench mesh lives in `Morrowind.bsa`, so there's no loose file to point at.
- `readFile(vfsPath)` reads a file from the **whole VFS**, archives included, and returns a `buffer`, or `nil`. It picks the same winner OpenMW would when several sources have the file.

For more than that (which source wins, what it overrides, everything under a folder) `openmw:vfs()` returns [vfstool](@/docs/scripting/downstream-libraries.md)'s VFS for the config.

## Changing what's active

`activate(name)` and `deactivate(name)` change the content list; `save()` writes it back to the user's `openmw.cfg`:

```luau
--!strict
local path: string = ...
local openmw = jess.openmw.load(path)

print(openmw:deactivate 'Bloodmoon.esm')
print(openmw:activate 'My Patch.omwaddon')
openmw:save()

for _, content in jess.openmw.load(path):content() do
  print(content.name)
end
```

```
true
true
Morrowind.esm
Tribunal.esm
My Patch.omwaddon
```

Both return whether the list changed. `activate` appends the file at the end of the load order; it has to be in a data folder (`{ force = true }` skips that check). Nothing is written until `save()`, and `save()` writes the user's `openmw.cfg`, the last one in the chain, which is the real one even when you loaded a config that chains to it. It keeps a backup of the file first and returns `{ path, backup }`.

## Which config names use

Everything that takes a plugin or file name (`jess.plugin.open`, `jess.plugin.new`'s masters, `jess.nif.open`, `jess.save.check`, `require`) resolves it the same way: a path that exists is used as is; otherwise the name is looked up through the default config, when there is one.

```luau
--!strict
local other: string = ...

-- Every name from here on resolves through the other config.
jess.openmw.configPath(other)
local tribunal = jess.plugin.open 'Tribunal.esm'
print(tribunal.path)

-- Opt one call out entirely: the name is a plain path, relative to the working folder.
local copy = jess.plugin.open('backups/Tribunal.esm', { openmw = false })
print(copy.path)
```

- `jess.openmw.configPath(path)` switches the config names resolve through, for the rest of the script; `configPath()` with no path makes it OpenMW's own, found the way OpenMW finds it ([the order](@/docs/getting-started/openmw-config.md#in-scripts)).
- `{ openmw = false }` on a call turns the lookup off for that call. Every call that resolves names also takes `{ openmw = context }`, to resolve through a context you loaded, or `{ config = path }`, to load one for that call.
- A `jess run` script starts with no default config, so until it calls `configPath`, a name is a path relative to the working folder, as it is on the command line.

A name that isn't a file and isn't in the load order is an error that says where jess looked: `'x.esp' doesn't exist and isn't in the OpenMW data paths (2 data directories)`. With no config to look in, it's `'x.esp' doesn't exist`. A path with a folder in it (`./x.esp`, `mods/x.esp`) is never looked up; if it doesn't exist, it doesn't exist.

## `require`

`require` works exactly like it does in OpenMW's Lua: modules come from the VFS, so they can ship loose or in an archive, and a later mod can override them.

- `require("scripts.myMod.util")` turns dots into slashes and loads `scripts/mymod/util.lua`, or `scripts/mymod/util/init.lua`, as OpenMW does. jess also takes `.luau` files there.
- `require("./util")` and `require("../shared/util")` are relative to the requiring script, like Luau's require by string: in the VFS for a module loaded from it, and next to the file for a script you run from disk (`jess run tools/patch.luau`). They try `util.luau`, `util.lua`, `util/init.luau`, and `util/init.lua`. An `init` script stands for its folder, so its `./` is beside the folder and `@self/` inside it.
- Each module runs once and is cached; requiring it again, by any name that reaches the same file, returns the same value.
- `require("jess")` returns the jess API, and the standard libraries OpenMW preloads (`string`, `table`, `math`, `coroutine`, `utf8`, `os`) are there by name. So are the [downstream libraries](@/docs/scripting/downstream-libraries.md), with or without a leading `@`, and each of jess's modules as `@jess/<name>` (`require("@jess/shader")` is `jess.shader`).
- Without a default config there's no VFS: only those preloaded names, the exports of `--package` packages, and a disk script's relative requires work. That's where a `jess run` script starts; `jess.openmw.configPath` gives it one.

A module is an ordinary Luau file that returns a value, `scripts/lightbooks/util/init.lua`:

```luau
--!strict
local util = {}

function util.lowerSet(list: { string }): { [string]: boolean }
  local set = {}
  for _, value in list do
    set[string.lower(value)] = true
  end
  return set
end

return util
```

`require` is typed to return `any`, since a VFS module's type depends on the file. Annotate what you get back with the module's type, and the rest of the script is checked against it:

```luau
--!strict
type Util = { lowerSet: (list: { string }) -> { [string]: boolean } }

local util: Util = require 'scripts.lightbooks.util'

print(util.lowerSet({ 'A', 'b' }).a)
```

The jess API and the downstream libraries are globals anyway, so you rarely need `require` for them.

## Related

- [Load-Order Scripts](@/docs/scripting/load-order-scripts.md): the context as a script input.
- [vfstool, Archives, and Friends](@/docs/scripting/downstream-libraries.md): the VFS and `openmw.cfg` in full detail.
- The [OpenMW context reference](@/docs/api/openmw.md) and [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md).
