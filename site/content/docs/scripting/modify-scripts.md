---
title: 'Per-File Scripts'
description: 'plugin modify --script and nif modify --script: edit one file with a script.'
weight: 50
extra:
  kind: guide
---

Sometimes you don't want a patcher, you want to fix *this* file. `jess plugin modify --script` and `jess nif modify --script` open one file, hand it to your script, and write it back, with a backup, if the script changed anything. The script never deals with paths, saving, or backups; it just edits.

## Fix a plugin

A script that takes the flicker out of every light in a plugin:

```luau
--!strict
-- Takes the flicker out of every light in a plugin.
local plugin: Plugin, path: string = ...

local steadied = 0
for record in plugin:select({ type = 'LIGH' }):records() do
  local light = record :: LightRecord
  local kept: { string } = {}

  for flag in string.gmatch(light.data.flags, '[%w_]+') do
    if flag ~= 'FLICKER' and flag ~= 'FLICKER_SLOW' then table.insert(kept, flag) end
  end

  local flags = table.concat(kept, ' | ')
  if flags ~= light.data.flags then
    light.data.flags = flags
    steadied += 1
  end
end

print(`{path}: steadied {steadied} lights`)
```

```sh
$ jess plugin modify --script steady.jsl MyMod.esp
MyMod.esp: steadied 2 lights
input: MyMod.esp
path: MyMod.esp
script: steady.jsl
changed: true
written:
  path: MyMod.esp
  backup: MyMod.esp.bak
```

Run it again and nothing's left to change, so nothing's written:

```sh
$ jess plugin modify --script steady.jsl MyMod.esp
MyMod.esp: steadied 0 lights
input: MyMod.esp
path: MyMod.esp
script: steady.jsl
changed: false
written: null
```

The script's `...` is two values: the plugin, as a [plugin handle](@/docs/api/plugin.md), and the path it was opened from, as a string. Edit through the handle however you like: set fields, `insert`, `delete`, run selections. When the script returns, jess compares the plugin's bytes to what it read and writes the file only if they differ. Light flags are one string of names joined by ` | `, which is why the script splits and rejoins them.

- The file is written in place, and the original kept as `MyMod.esp.bak`. Earlier backups are kept too: the next one is `.bak1`, then `.bak2`.
- `--output Fixed.esp` writes somewhere else and leaves the input alone. A file already at the output is backed up before it's replaced, like the input would be.
- `--dry-run` runs the script and reports whether it changed anything, and writes nothing. `--no-backup` skips the `.bak`, for when the file is already under version control and a backup would only be litter.
- The input resolves like every other plugin argument: a path that exists, or a name found through your `openmw.cfg`.
- The report goes to stdout in the global `--format` like every other command; the script's own `print`s come before it.

If you've used tes3cmd, this is the job `tes3cmd modify --program-file` did, except the script gets the whole plugin once, as a handle, instead of being run against each matching record.

## Fix a mesh

`jess nif modify --script` does the same for a Morrowind NIF. The script gets a [NIF handle](@/docs/api/nif.md) and the path. This one points every texture at its `.dds` version:

```luau
--!strict
-- Points every .tga/.bmp texture in a NIF at its .dds version.
local nif: Nif, path: string = ...

local repointed = 0
for _, texture in nif:textures() do
  local dds = string.gsub(texture.path, '%.%a+$', '.dds')
  if dds ~= texture.path then
    repointed += nif:setTexture(texture.path, dds)
  end
end

print(`{path}: {repointed} textures now .dds`)
```

The input can be a file, or a VFS path in your load order. A mesh inside a BSA can't be written back into the archive, so it needs `--output`:

```sh
$ jess nif modify --script dds.jsl meshes/f/furn_de_ex_bench_01.nif --output bench.nif
meshes/f/furn_de_ex_bench_01.nif: 2 textures now .dds
input: meshes/f/furn_de_ex_bench_01.nif
path: meshes/f/furn_de_ex_bench_01.nif
script: dds.jsl
changed: true
written:
  path: bench.nif
```

`--data` adds folders or archives to look the VFS path up in, over the load order. The [NIF deep dive](@/docs/nif/editing.md) covers what you can do to a NIF handle.

## Many files

A per-file script handles one file per run. For a handful of files, a shell loop is fine:

```sh
for plugin in Mods/*/*.esp; do
    jess plugin modify --script steady.jsl "$plugin"
done
```

Each run starts jess, loads your config, and reads the file. For hundreds of files, or when you need to look at other plugins while editing, write one `jess run` script that loops instead:

```luau
--!strict
-- Takes the flicker out of every light in every plugin in a folder, in one run.
local folder: string = ...

local function steady(plugin: Plugin): number
  local steadied = 0
  for record in plugin:select({ type = 'LIGH' }):records() do
    local light = record :: LightRecord
    local kept: { string } = {}

    for flag in string.gmatch(light.data.flags, '[%w_]+') do
      if flag ~= 'FLICKER' and flag ~= 'FLICKER_SLOW' then table.insert(kept, flag) end
    end

    local flags = table.concat(kept, ' | ')
    if flags ~= light.data.flags then
      light.data.flags = flags
      steadied += 1
    end
  end
  return steadied
end

local names = jess.fs.list(folder)
table.sort(names)

for _, name in names do
  local extension = string.lower(string.match(name, '%.(%w+)$') or '')
  if extension ~= 'esp' and extension ~= 'esm' and extension ~= 'omwaddon' then continue end

  local plugin = jess.plugin.open(jess.fs.join(folder, name), { openmw = false })
  local steadied = steady(plugin)
  if steadied > 0 then
    plugin:save()
    print(`{name}: steadied {steadied} lights`)
  end
end
```

```sh
$ jess run steady-all.jsl "Mods/Lanterns"
A.esp: steadied 2 lights
B.esp: steadied 2 lights
C.esp: steadied 2 lights
```

Here the script owns saving: `plugin:save()` writes the plugin back to where it was opened from, with a `.bak` of what it replaced. `jess.fs.list` returns names in whatever order the filesystem does, hence the sort.

## Pitfalls

- **Don't save inside a modify script.** jess writes the file after your script returns. Calling `plugin:save()` yourself writes it first, and then jess writes it again, leaving you two backups for one change.
- **Changed means bytes.** jess compares the bytes before and after. Setting a field to the value it already had changes nothing, and nothing is written; that's what you want.
- **Only Morrowind NIFs.** `nif modify` edits Morrowind's NIF format; other games' NIFs are read-only in jess.

## Related

- [Records and Views](@/docs/scripting/records-and-views.md) and [Selections](@/docs/scripting/selections.md), for what to do with the handle.
- The [`jess plugin`](@/docs/cli/plugin.md) and [`jess nif`](@/docs/cli/nif.md) references.
