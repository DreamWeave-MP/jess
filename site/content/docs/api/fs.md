---
title: 'Files'
description: 'Files and folders are @dream/fs, L3i''s file module: what jess adds is the capability gate and @jess/backup.'
weight: 200
extra:
  kind: api
---

Scripts read and write files with
[`@dream/fs`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-fs), L3i's file
module, which every jess runtime has. jess has no file API of its own, on purpose: one module, one
error style, one idea of list order. What jess adds is the gate in front of it. Reading needs
`filesystem.read` and changing the disk `filesystem.write`
([`jess run --capability`](@/docs/cli/run.md#capabilities), or the capabilities a target
declares); joining and taking paths apart needs neither. The
[Files recipes](@/docs/scripting/files-toml-regex.md#files) use it.

An ordinary `@dream/fs` operation failure answers `nil`, a message naming the path, and a kind:
wrap the call in `assert` where a failure should stop the script. A denied capability raises before
the operation; see [Capabilities](@/docs/reference/capabilities.md).

```luau
--!strict
local fs = require '@dream/fs'

local manifest = assert(fs.readFileString 'jess.toml')
assert(fs.writeFile('target/manifest.copy.toml', manifest, { atomic = true, createParents = true }))
```

Three things to know before reaching for it:

- `list(path)` sorts names by their bytes. `walk(root, options)` covers a whole tree, but does not sort by default; request `sort = true` when traversal order matters.
- `writeFile` writes what it's told and keeps nothing. A tool that overwrites someone's file
  should keep what was there, which is [`@jess/backup`](@/docs/api/backup.md).
- `stat(path)` has `size`, `modifiedSeconds`, and `modifiedNanoseconds`; there is no separate
  `size` or `modified` call.

The whole module, with the rest of L3i's primitives, is on [L3i's site](@/docs/api/l3i.md).
