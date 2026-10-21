---
title: '@jess/backup'
description: 'Overwrite a file the way jess tools do: what was there goes to the next free .bak first, never over an earlier one.'
weight: 155
extra:
  kind: api
---

{{ api_signature(value="require('@jess/backup'): { write }") }}

[`@dream/fs`](@/docs/api/fs.md) writes what it's told and keeps nothing. A tool that overwrites
someone's config, data file, or report should keep what was there, and `@jess/backup` is how
jess's own tools do it, so they all do it the same way: the old file is copied to the first of
`.bak`, `.bak1`, `.bak2`, ... beside it that isn't taken, then the new bytes replace it atomically.
An earlier backup is never written over, not even by a second writer racing for the same name.
It needs `filesystem.read` and `filesystem.write`.

## write

{{ api_signature(value="backup.write(path: string, data: string | buffer, options: { backup: boolean? }?) -> { path: string, backup: string? }") }}

Writes `data` to `path`, making the folders it goes in, after copying what was there aside.

- **Parameters:** `options.backup = false` writes without keeping a backup, for a tool whose user
  turned them off. On by default.
- **Returns:** `{ path, backup }`: `backup` is where the old file went, or `nil` when there was no
  file there or `backup` was `false`.
- **Errors:** `jess.backup.write: <message>` when the backup or the write fails
  (`jess.backup.write: dream.fs.writeFile: /proc/nope/x.txt: No such file or directory (os error
  2)`). A failed backup leaves the old file as it was, and no half-made backup behind; a failed
  write leaves the old file whole, since the new one only replaces it once it's all on disk.
  Permissions carry over from the file replaced.

```luau
--!strict
local backup = require('@jess/backup')
backup.write('notes.txt', 'first\n')
local written = backup.write('notes.txt', 'second\n')
print(written.backup) -- notes.txt.bak
print(backup.write('notes.txt', 'third\n').backup) -- notes.txt.bak1
print(backup.write('notes.txt', 'fourth\n', { backup = false }).backup) -- nil
```
