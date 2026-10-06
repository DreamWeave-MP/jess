---
title: 'Downstream Globals'
description: 'vfstool, openmwConfig, dreamPath.'
weight: 140
extra:
  kind: api
---

{{ api_signature(value="vfstool, openmwConfig, dreamPath") }}

jess is built on DreamWeave's libraries, and their own Luau bindings are installed next to `jess` in every script: as globals, through `require` by that name with or without a leading `@` (`require("@vfstool")`), and through the libraries' own module paths (`require("@dream/vfs")`). They share the script's VM, so values pass between them and jess directly: [context:vfs()](@/docs/api/openmw.md#context-vfs) returns a `vfstool` VFS.

| Global | Module | Library | For |
| --- | --- | --- | --- |
| `vfstool` | `@dream/vfs` | [vfstool](https://dreamweave-mp.github.io/vfstool/docs/luau/) | OpenMW's virtual file system: which file wins, conflicts, provenance, materializing, host files. |
| `openmwConfig` | `@dream/openmw-config` | [openmw_config](https://dreamweave-mp.github.io/Openmw_Config/docs/lua/) | Reading and writing `openmw.cfg` chains directly. |
| `dreamPath` | `@dream/path` | [dream-path](https://dreamweave-mp.github.io/dream_path/docs/luau/) | The path normalization the VFS and archives share. |
| (none) | `@dream/bytes` | [l3i](https://DreamWeave-MP.github.io/l3i/) | Bytes and text: `find`, `rfind`, `slice`, `compare`, hex, varints, C strings, and `decode`/`encode` between a Luau string and any WHATWG-labelled encoding (`windows-1252`, `utf-16le`). Only through `require`. |
| (none) | `@dream/luau` | [l3i](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-luau) | Luau's own parser: `luau.parse(source, options?)` returns the syntax tree as tables, every node with its kind and an exact span, every comment, parse errors as data, and with `{ tokens = true }` Luau's token stream as a buffer. Typed for strict walkers: refine a `dream_luau_Expr` by its `kind`. What [`@jess/style`](@/docs/api/style.md) is built on. Only through `require`. |
| (none) | `@dream/intern` | [l3i](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-intern) | Textual identity as numbers: a pool turns a string or a span of a buffer into a small whole number, exactly or ASCII case-insensitively, without making a string; in native code with the pool annotated (`local ids: dream_intern_Pool = intern.new()`), a lookup never leaves it. Only through `require`. |

Each library documents its own surface, member by member, on its Luau API pages; those are the contract. jess ships vfstool_lib 1.0.1, openmw-config 3.0.2, and dream-path 1.1.0, and `jess luau-types` prints their complete type definitions as their extensions declare them: every module is a `Module__dream_<name>` table type (`Module__dream_vfs`), and every handle a class named after its library (`dream_vfs_VFS`, `dream_openmw_Config`).

## What they have in common

All three are [l3i](https://DreamWeave-MP.github.io/l3i/) extensions, and they follow one set of rules, which differ from jess's own API in a few places worth knowing before you paste a snippet from one into the other.

**Lists are views.** `vfs:keys()`, `vfs:entries()`, `cfg:contentFiles()`, `cfg:gameSettings()`, and every other list a library hands out is a sequence view, a userdata over the Rust collection, not a table. It answers `#view`, `view[i]` (1-based; `nil` past the end), `for _, item in view do`, and `view:toTable()` for a plain table when you need one. `ipairs`, `pairs`, and `table.clone` over a view fail. Only the element you touch is pushed, so a 60,000-key VFS costs nothing to hold.

**Hashes and ids are Luau integers.** A `VfsFile:size()`-style count is a `number`; a hash or a packed identity is an `integer`, which compares with `==` only and never equals a `number` (`42i ~= 42`). Counts and lengths stay plain numbers.

**Option tables are strict.** A key a function doesn't take is an error naming it, never a silently ignored default; the same rule jess's own [option tables](@/docs/api/jess.md) follow.

**Errors say who raised them.** `dream.vfs: <context>: <message>`; jess's own errors carry no prefix.

**Strings are bytes.** VFS paths and file contents are Luau strings holding raw bytes, so names that aren't UTF-8 survive. Host filesystem paths are UTF-8.

## vfstool

A VFS maps normalized keys (lowercase, forward slashes: `meshes/x/rock.nif`) to a stack of providers, loose files or archive entries, where the last-loaded one wins, as in OpenMW. `vfstool.VFS.fromDirectories(dirs, { archives = ... })` builds one the way OpenMW does; [context:vfs()](@/docs/api/openmw.md#context-vfs) builds the load order's.

```luau
--!strict
local vfs = vfstool.VFS.fromDirectories { 'low', 'high' }
print(vfs:len(), vfs:contains 'Meshes\\X\\Rock.nif')

local file = vfs:getFile 'meshes/x/rock.nif'
if file then print(file:path(), file:isLoose(), #file:readAll()) end

for _, key in vfs:keys() do
  print(key)
end

print(vfstool.serialize({ files = vfs:keys() }, 'json'))
```

`vfstool.serialize` takes views as well as tables. The `host` table (`vfstool.host.readFile`, `writeFile`, `list`, `stat`, ...) is files and directories on host paths with no VFS in between; jess's runtime grants the capability it needs, the same as [jess.fs](@/docs/api/fs.md).

Beyond lookups: `explain`, `duplicates`, `providersFor`, and `sourceContributions` say why a path resolves the way it does; `pushDirectory`, `pushArchive`, `setWinnerFile`, and `removeWinner` change the stack; `dumpToDirectory`, `collapseInto`, and `materializationPlan` write a merged folder; `LayerIndex` and `ConflictIndex` are provenance and conflict reports; `runSetup` and `runFinalize` are the dump-run-collect workflow for a child tool. Every one is on [vfstool's Luau pages](https://dreamweave-mp.github.io/vfstool/docs/luau/).

## openmwConfig

Reads and writes `openmw.cfg` chains the way OpenMW does, following `config=` entries and `replace=`. jess's own [OpenMwContext](@/docs/api/openmw.md#openmwcontext) is built on it; use `openmwConfig` for settings the context doesn't expose.

```luau
--!strict
local cfg = openmwConfig.newEmpty 'scratch-config'
cfg:addContentFile 'Morrowind.esm'
cfg:setDataDirectories { 'Data Files' }
print(table.concat(cfg:contentFiles():toTable(), ', '), cfg:hasContentFile 'Morrowind.esm')
print(cfg.isUserConfig, cfg.contentFileCount)
print(cfg:toString())

for _, setting in cfg:genericSettings() do
  print(setting.key, setting.value)
end
```

`isUserConfig` and the `...Count` members are fields, not methods. Setting rows (`gameSettings()`, `genericSettings()`) go stale after the config is edited and say so when read. Saving is `saveUser()` (the user's `openmw.cfg` only), `saveSubconfig(dir)`, `saveToPath(path)`, and `saveResolvedToPath(path)`; the module's `fromEnv`, `new`, `loadOptional`, and `default*`/`tryDefault*` functions find and load configs. Details: [openmw_config's Lua pages](https://dreamweave-mp.github.io/Openmw_Config/docs/lua/).

## dreamPath

Byte-preserving virtual path helpers, the normalization the VFS and archives share: lowercase ASCII, forward slashes. `normalize`, `isNormalized`, `fileName`, `parent`, `extension`, and `isUtf8`.

```luau
--!strict
print(dreamPath.normalize 'Meshes\\X\\Rock.NIF') -- meshes/x/rock.nif
print(dreamPath.parent 'meshes/x/rock.nif', dreamPath.extension 'meshes/x/rock.nif') -- meshes/x  nif
```

## Archives

There is no `dreamArchive` or `dreamArchivetool` any more. BSA and BA2 archives are [Dreamwind](@/docs/scripting/packages.md)'s, in Luau: [`@dreamwind/archive`](@/docs/api/dreamwind-archive.md) is the archive API under its own name, and [`@dreamwind/archive/tool`](@/docs/api/dreamwind-archive-tool.md) is dream_archivetool's, its operations taking an opened archive where the native module had methods on it. Both come with `jess run --package jpk/dreamwind`. vfstool still reads archives itself for the VFS.

There is no `dreamIni` any more. The `Morrowind.ini` importer is [openmw-import-morrowind](@/docs/scripting/packages.md), a Luau port of dream-ini, loaded from the repository rather than shipped in the binary.

Related: [Downstream Libraries](@/docs/scripting/downstream-libraries.md).
