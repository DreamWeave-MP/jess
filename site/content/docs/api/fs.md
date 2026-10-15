---
title: 'Files'
description: 'Files and folders are @dream/fs: what jess.fs was, and what each of its functions is now.'
weight: 100
extra:
  kind: api
---

Scripts read and write files with
[`@dream/fs`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-fs), l3i's file
module, which every jess runtime has. Reading needs `filesystem.read` and changing the disk
`filesystem.write` ([`jess run --capability`](@/docs/cli/run.md#capabilities)); joining and taking
paths apart needs neither. jess had a second API, `jess.fs`, with its own error style and its own
idea of list order; it is gone. The [Files recipes](@/docs/scripting/files-toml-regex.md#files)
use `@dream/fs`.

A `@dream/fs` function that fails answers `nil`, a message naming the path, and a kind, where
`jess.fs` raised: wrap the call in `assert` where a failure should stop the script.

| `jess.fs` | `@dream/fs` |
| --- | --- |
| `read(path)` | `readFileString(path)` |
| `readBuffer(path)` | `readFile(path)` |
| `write(path, data)` | `writeFile(path, data, { atomic = true, createParents = true })` |
| `write(path, data, { backup = true })` | copy the file aside first: backups are a tool's policy |
| `replace(path, data)` | `writeFile(path, data, { atomic = true })` |
| `copy(from, to)`, `remove(path)` | `copy(from, to)`, `remove(path)` (which also takes `{ recursive = true }`) |
| `createDir(path)` | `mkdir(path, { recursive = true })` |
| `exists`, `isDir`, `isFile`, `isFileName` | the same names |
| `size(path)`, `modified(path)`, `stat(path)` | `stat(path)`: `size`, `modifiedSeconds`, `modifiedNanoseconds`, ... |
| `list(path)` | `list(path)`, sorted by name; `walk(root, options)` for a whole tree |
| `cwd`, `join`, `fileName`, `parent` | the same names, plus `extension` |
