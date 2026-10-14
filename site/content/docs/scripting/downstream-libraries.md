---
title: 'vfstool, Archives, and Friends'
description: 'The libraries jess is built on, exposed as globals: vfstool, openmwConfig, dreamPath; and BSA and BA2 archives, from Dreamwind.'
weight: 100
extra:
  kind: guide
---

jess stands on a few other DreamWeave libraries: one that builds OpenMW's virtual file system, one that reads and writes `openmw.cfg`, and one that spells virtual paths the way both do. Their Luau APIs are loaded into every jess script too, sharing one Luau state with jess. Reach for them when jess's own API stops short: jess answers "what does the game load"; these answer "where does it come from" and "what else is in this config". "Pack this folder into a BSA" is Dreamwind's, in Luau, [below](#archives-dreamwind-archive).

| Global | Library | For |
| --- | --- | --- |
| `vfstool` | [vfstool](https://github.com/DreamWeave-MP/vfstool) | The VFS in full: which source provides every file, what overrides what, extracting, collapsing. |
| `openmwConfig` | [openmw_config](https://github.com/DreamWeave-MP/Openmw_Config) | Every setting in an `openmw.cfg` chain, read and write. |
| `dreamPath` | [dream-path](https://dreamweave-mp.github.io/dream_path/docs/luau/) | The path normalization the VFS and archives share. |

Each is a global, and `require` returns it too, with or without a leading `@`: `require("vfstool")` and `require("@vfstool")` are both `vfstool`, and so is `require("@dream/vfs")`, the library's own module path. Lists they hand out are views, not tables: `#`, `[i]`, `for _, item in view`, and `:toTable()` when a plain table is wanted. The [downstream globals reference](@/docs/api/downstream-globals.md) has the conventions and points at each library's own pages for every function.

## The VFS: `vfstool`

`openmw:vfs()` builds vfstool's VFS for a config: every data folder and fallback archive, in OpenMW's priority order.

```luau
--!strict
local openmw = jess.openmw.default() or error 'needs an OpenMW config'
local vfs = openmw:vfs()

print(`{vfs:len()} files in the VFS`)

-- Where does a file come from, and what does it override?
local explained = vfs:explain 'meshes/f/furn_de_ex_bench_01.nif'
if explained then
  print(
    explained.key,
    explained.winner.source.path,
    explained.winner.source.kind,
    #explained.overridden
  )
end

-- pathsWith takes a folder; pathsMatching, any substring.
print(#vfs:pathsWith 'icons/a', 'armor icons')
for index, entry in vfs:pathsMatching 'tx_daedric' do
  if index <= 3 then print(entry.key, entry.file:isArchive()) end
end
```

```
26218 files in the VFS
meshes/f/furn_de_ex_bench_01.nif	/home/you/Games/Morrowind/Data Files/Morrowind.bsa	archive	0
421	armor icons
icons/w/tx_daedric_longsword.dds	true
icons/a/tx_daedric_terrifying_h.tga	true
icons/a/tx_daedric_gauntlet.dds	true
```

- `explain(path)` names the winning source and every source it beats. `duplicates()` lists every file more than one source provides: a whole load order's worth of overrides in one call.
- `getFile(path)` returns the winning file; `file:readAll()` reads it, and `file:isArchive()` says where it lives.
- `pathsWith(folder)` lists a folder's files (a whole path component: `"icons/a"`, not `"icons/a/tx_"`), and `pathsMatching(text)` every path containing the text. `findByRegex` takes a pattern.
- `extractFile(path, folder)` copies the winning file out, from a loose folder or an archive alike.
- `vfstool.VFS.fromDirectories(folders, { archives = ... })` builds a VFS without any `openmw.cfg`.

`openmw:readFile(path)` covers the common case of reading one file. Use the VFS when you need to know about the sources, not just the bytes.

## Configs: `openmwConfig`

`jess.openmw` covers what jess needs from a config: content, data folders, `data-local`. `openmwConfig` is the whole file, every setting and the chain of configs behind it:

```luau
--!strict
local path: string? = ...
local config = openmwConfig.new(path)

for _, link in config:configChain() do
  print(link.depth, link.status, link.path)
end
print(`{#config:contentFiles()} content files, {#config:groundcoverFiles()} groundcover files`)

-- fallback= values: the settings Morrowind.ini used to hold.
local sunrise = config:getGameSetting 'Weather_Clear_Sky_Sunrise_Color'
if sunrise then print(sunrise.key, sunrise.value, sunrise.source) end
```

```
0	loaded	/home/you/.config/openmw/openmw.cfg
1	loaded	/home/you/.config/openmw/Morrowind/openmw.cfg
71 content files, 9 groundcover files
Weather_Clear_Sky_Sunrise_Color	117,141,164	/home/you/.config/openmw/Morrowind/openmw.cfg
```

`openmwConfig.new(path)` loads a config from an `openmw.cfg` or its folder, or OpenMW's default with no argument. Beyond reading, it edits: `addContentFile`, `addGroundcoverFile`, `setDataLocal`, `setGameSetting`, and friends, then `saveUser()` to write the user's `openmw.cfg`, or `saveToPath(path)` to write a copy somewhere else. Every setting knows the `source` file that set it.

## Archives: `@dreamwind/archive`

BSA and BA2 archives aren't one of these libraries any more. They're [Dreamwind](@/docs/scripting/packages.md)'s, in Luau: [`@dreamwind/archive`](@/docs/api/dreamwind-archive.md) opens an archive and works with it as an object, and [`@dreamwind/archive/tool`](@/docs/api/dreamwind-archive-tool.md) does whole jobs, the way dream_archivetool's command line does. Both come with `--package jpk/dreamwind`.

Pack a mod's loose files into a Morrowind BSA, then look inside:

```luau
--!strict
-- Pack a mod's loose files into a Morrowind BSA, then check what went in.
local archive = require '@dreamwind/archive'
local tool = require '@dreamwind/archive/tool'
local folder: string, output: string = ...

local created = tool.create(output, folder, { format = 'bsaTes3' })
print(`packed {created.files} files into {output}`)

local opened = archive.openPath(output)
local info = tool.info(opened)
print(info.format, info.fileCount)
for _, entry in opened:entries() do
  print(entry.index, entry.path)
end

local nif = opened:readFile 'meshes/jess/all_types.nif'
print(nif and #nif)
print(dreamPath.normalize 'Meshes\\Jess\\All_Types.NIF')
```

```
$ jess run --package jpk/dreamwind pack.jsl "MyMod/Data Files" MyMod.bsa
packed 3 files into MyMod.bsa
bsaTes3	3
1	textures\readme.txt
2	meshes\jess\random_objects.nif
3	meshes\jess\all_types.nif
15877
meshes/jess/all_types.nif
```

Archives store their own spelling (Morrowind's BSAs use backslashes), but lookups normalize: `readFile("meshes/jess/all_types.nif")` finds `meshes\jess\all_types.nif`. `dreamPath.normalize` is that normalization on its own: lowercase, forward slashes.

Extracting works from either side:

```luau
--!strict
-- extract.jsl: one mesh out of a load order's archives, two ways.
local tool = require '@dreamwind/archive/tool'
local config: string, bsa: string = ...
local openmw = jess.openmw.load(config)
local vfs = openmw:vfs()

-- Pull one mesh out of whatever archive it's in, into a folder of your own.
local written = vfs:extractFile('meshes/f/furn_de_ex_bench_01.nif', 'extracted')
print(written)

-- Or ask the archive directly.
local summary = tool.extract(
  bsa,
  'meshes/f/furn_com_bench_01.nif',
  { output = 'extracted', preservePaths = true }
)
print(summary.extracted, summary.skipped)
```

```
$ jess run --package jpk/dreamwind extract.jsl ~/.config/openmw/openmw.cfg "Data Files/Morrowind.bsa"
extracted/furn_de_ex_bench_01.nif
1	0
```

`vfs:extractFile` writes just the file name into the folder; `preservePaths = true` keeps the archive's folders (`extracted/meshes/f/furn_com_bench_01.nif`). `extractAll`, `verify`, `diff`, and `add` round out the tool; `planExtract`, `planCreate`, and `planAdd` say what they would do without doing it.

## Morrowind.ini

A Morrowind install keeps its settings in `Morrowind.ini`: the load order, the archives, whether sound is off, and several hundred game settings OpenMW reads as `fallback=` values. The importer for it, the one OpenMW's installer runs, is [openmw-import-morrowind](@/docs/scripting/packages.md), a package written in Luau on top of the libraries above. It isn't a global.
