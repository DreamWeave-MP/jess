---
title: 'jess.openmw and OpenMwContext'
description: 'Config contexts, content, VFS reads, and activation.'
weight: 70
aliases:
  - /docs/cli/openmw/
extra:
  kind: api
---

{{ api_signature(value="jess.openmw: { default, load, configPath }") }}

`jess.openmw` loads OpenMW configs. A loaded config is an `OpenMwContext`: the `openmw.cfg` chain, its data folders and archives (the VFS), and its active content. It's the Luau face of the `jess-openmw` crate. The `jess` CLI doesn't read OpenMW configs itself; a script that wants a load order loads one here.

jess keeps one default config per script. Plugin names, `require`, and the load-order functions (`jess.nif.check()` with no inputs, `jess.plugin.assets()` with no plugins) resolve through it unless a call says otherwise. Under `jess run` (and `jess test`, and `--script` on `plugin modify` and `nif modify`) it starts turned off: names are plain paths until the script calls [`configPath`](#jess-openmw-configpath). A Rust host embedding jess's Luau runtime chooses where it starts. [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md#in-scripts) explains how OpenMW's own config is found.

## Functions

### jess.openmw.default

{{ api_signature(value="jess.openmw.default() -> OpenMwContext?") }}

Returns the default config's context, loading it on first use, or `nil` when config lookup is turned off, as it is when a `jess run` script starts.

The same context is returned every time until `configPath` changes the default.

- **Errors:** a config that can't be loaded.

```luau
--!strict
local openmw = jess.openmw.default()
if openmw then
  for _, content in openmw:content() do
    print(content.name)
  end
end
```

### jess.openmw.load

{{ api_signature(value="jess.openmw.load(path: string?) -> OpenMwContext") }}

Loads a config as a new context, separate from the default one.

- **Parameters:** `path`, an `openmw.cfg` or its folder. `nil` loads the current default config afresh.
- **Errors:** `OpenMW config resolution is disabled` (no path, and no default config, as under `jess run` before `configPath`), or a config that can't be loaded.

```luau
--!strict
local other = jess.openmw.load 'cfg/openmw.cfg'
print(#other.dataDirs, other.userConfigDir)
local plugin = jess.plugin.open('MyMod.esp', { openmw = other })
print(plugin.path)
```

### jess.openmw.configPath

{{ api_signature(value="jess.openmw.configPath(path: string?) -> ()") }}

Changes the default config: plugin names, `require`, and `jess.openmw.default()` then resolve through the config at `path`. `nil` makes it OpenMW's own config, found the way OpenMW finds it.

The new config loads lazily, on first use. Contexts you already hold keep working.

```luau
--!strict
jess.openmw.configPath 'cfg/openmw.cfg'
local openmw = jess.openmw.default()
print(openmw and #openmw:content())
```

## OpenMwContext

A loaded config. Its fields are read-only properties, recomputed on every read; its methods use `:`. Two handles are `==` when they're the same loaded context, such as two calls to `jess.openmw.default()`.

### context.dataDirs

{{ api_signature(value="context.dataDirs: { string }") }}

The data folders (`data=` lines), resolved, in load order: later folders win.

```luau
--!strict
for _, dir in jess.openmw.load('cfg/openmw.cfg').dataDirs do
  print(dir)
end
```

### context.fallbackArchives

{{ api_signature(value="context.fallbackArchives: { string }") }}

The archives (`fallback-archive=` lines), as the config names them.

```luau
--!strict
print(#jess.openmw.load('cfg/openmw.cfg').fallbackArchives)
```

### context.dataLocal

{{ api_signature(value="context.dataLocal: string?") }}

The config's `data-local=` folder, if it sets one.

```luau
--!strict
print(jess.openmw.load('cfg/openmw.cfg').dataLocal)
```

### context.encoding

{{ api_signature(value="context.encoding: Encoding?") }}

The config's `encoding=`, if it sets one: `"win1250"`, `"win1251"`, or `"win1252"`, how OpenMW reads the load order's text. When several configs in the chain set it, the last one wins. Plugins and saves opened through this config are read in it unless you pass `encoding`, and in `"win1252"` when it's `nil`.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
local plugin = jess.plugin.open('MyMod.esp', { openmw = openmw })
print(openmw.encoding, plugin.encoding)
```

### context.userConfigDir

{{ api_signature(value="context.userConfigDir: string") }}

The folder of the last `openmw.cfg` in the chain: the user's, where tools keep their settings and logs, and where `save()` writes.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
jess.fs.write(jess.fs.join(other.userConfigDir, 'my-tool.log'), 'ran\n')
```

### context.jessScripts

{{ api_signature(value="context.jessScripts: { string }") }}

The config's `jess-script=` lines, in config order: VFS paths (lowercase, forward slashes) or bundled `@jess/...` script names. The `jess` CLI doesn't run them; a Rust host running jess-lua's load-order scripts (`jess_lua::scripts::load_order_scripts`) does.

```luau
--!strict
for _, script in jess.openmw.load('cfg/openmw.cfg').jessScripts do
  print(script) -- scripts/jess/fix.jsl, @jess/s3lightfixes
end
```

Related: [Load Order Scripts](@/docs/scripting/load-order-scripts.md).

### context.loadScriptsDir

{{ api_signature(value="context.loadScriptsDir: string?") }}

The `jess-load-scripts` folder that bundled `@jess/...` scripts run from when a Rust host runs them as load-order scripts: in `data-local=` when the config sets it, else in the first `data=` folder. `nil` when the config has neither, and then they run from jess's own copies. The folder may not exist yet; it's created the first time a bundled script runs there.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
if openmw.loadScriptsDir then
  print(jess.fs.join(openmw.loadScriptsDir, 'tes3merge.jsl'))
end
```

### context.writableDataDir

{{ api_signature(value="context.writableDataDir: string?") }}

Where a tool should write what it generates: the config's `data-local=` folder, else its first `data=` folder, the same folder native TES3Merge picks. [TES3Merge](@/docs/bundled-scripts/tes3merge.md) and [Merged Lands](@/docs/bundled-scripts/mergedlands.md) write there unless told otherwise. `nil` when the config has neither.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
print(openmw.writableDataDir or 'nowhere to write')
```

### context:content

{{ api_signature(value="context:content() -> { { name: string, path: string? } }") }}

Lists the active content files in load order, with the file each resolves to (`nil` when it isn't in any data folder).

```luau
--!strict
for _, content in jess.openmw.load('cfg/openmw.cfg'):content() do
  print(content.name, content.path or 'missing')
end
```

### context:resolve

{{ api_signature(value="context:resolve(name: string) -> string?") }}

Returns the loose file a VFS path resolves to in the data folders (the last folder that has it wins), or `nil`. The lookup ignores case and slash direction; a bare name like `"MyMod.esp"` is only looked for at the top of each data folder.

A loose file always wins over an archive's copy, so when there's a loose file, it's the one OpenMW loads. A file only in an archive has no path of its own and returns `nil`: read it with [readFile](#context-readfile), and find its archive with `context:vfs():getFile(path):parentArchivePath()`.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
print(openmw:resolve 'Meshes\\X\\Rock.nif')
```

### context:readFile

{{ api_signature(value="context:readFile(path: string) -> buffer?") }}

Reads a file from the whole VFS, archives included, or returns `nil` when there's no such file.

- **Errors:** a file that exists but can't be read.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
local bytes = openmw:readFile 'meshes/x/rock.nif'
if bytes then print(jess.nif.checkBytes(bytes, 'meshes/x/rock.nif').worst) end
```

### context:vfs

{{ api_signature(value="context:vfs() -> dream_vfs_VFS") }}

Builds vfstool's VFS over the config's data folders and fallback archives, the way OpenMW builds its own, for queries beyond what jess covers.

Each call builds a new VFS. See [vfstool](@/docs/api/downstream-globals.md#vfstool) for its methods.

```luau
--!strict
local vfs = jess.openmw.load('cfg/openmw.cfg'):vfs()
print(vfs:len())
for _, entry in vfs:pathsWith 'meshes' do
  print(entry.key, entry.file:path())
end
```

### context:activate

{{ api_signature(value="context:activate(name: string, options: { force: boolean? }?) -> boolean") }}

Adds a content file to the end of the active content, returning whether it wasn't already active. A name already active in another case (`mymod.esp` for `MyMod.esp`) counts as active, since OpenMW finds content files ignoring case: adding it again would load the plugin twice.

The file must be at the top of one of the data folders, matched ignoring case. The folders are read again on every call, so a file your script just wrote there counts. `{ force = true }` activates a name anyway, for the rare case where you know better than the data folders. The change is in memory until [save](#context-save).

The check exists because OpenMW won't start with active content it can't find. Refusing the typo here is cheaper than finding it in OpenMW's log.

- **Errors:** `'<name>' isn't in the data folders; check the name, or pass { force = true } to activate it anyway`, or an entry the config rejects.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
print(openmw:activate 'MyMod.esp', openmw:activate 'MyMod.esp') -- true  false
```

### context:deactivate

{{ api_signature(value="context:deactivate(name: string) -> boolean") }}

Removes a content file from the active content, by name ignoring case, returning whether it was active. In memory until [save](#context-save).

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
print(openmw:deactivate 'Missing.esp') -- true
```

### context:save

{{ api_signature(value="context:save() -> { path: string, backup: string? }") }}

Writes the context's config changes to the user's `openmw.cfg` (in `userConfigDir`), after copying the file as it was to the next free backup (`openmw.cfg.bak`, then `.bak1`, ...).

OpenMW and its launcher keep lines they don't know, and so does jess, so `jess-script=` entries survive.

- **Returns:** the `openmw.cfg` written and its backup (`nil` when there was no file yet).
- **Errors:** a failure to write the backup or the config.

```luau
--!strict
local openmw = jess.openmw.load 'cfg/openmw.cfg'
if openmw:deactivate 'Missing.esp' then
  print(openmw:save().backup) -- cfg/openmw.cfg.bak
end
```

Related: [The OpenMW config and the VFS](@/docs/scripting/openmw-and-vfs.md), [`openmwConfig`](@/docs/api/downstream-globals.md#openmwconfig) for editing any `openmw.cfg` directly.
