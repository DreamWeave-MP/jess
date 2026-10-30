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

### backup.write

{{ api_signature(value="backup.write(path: string, data: string | buffer, options: Options?) -> Written") }}

Writes `data` to `path`, making the folders it goes in, after copying what was there aside.

`Options` is `{ backup: boolean? }`; `Written` is `{ path: string, backup: string? }`.

- **Parameters:** `options.backup = false` writes without keeping a backup, for a tool whose user
  turned them off. On by default.
- **Returns:** `{ path, backup }`: `backup` is where the old file went, or `nil` when there was no
  file there or `backup` was `false`.
- **Errors:** `jess.backup.write: <message>` when the backup or the write fails
  (`jess.backup.write: dream.fs.writeFile: /proc/nope/x.txt: No such file or directory (os error
  2)`). A failed backup does not replace the destination; the module attempts to remove the
  reserved backup file but cannot guarantee cleanup if removal also fails. The atomic write stages
  the new bytes beside the destination and renames them into place after writing and syncing the
  temporary file. Permissions carry over from the file replaced. A capability denial from
  `@dream/fs` raises directly rather than being rewritten as a `jess.backup.write` error.

The backup copy and the replacement are separate operations. They do not lock out another writer
and do not form a transaction: concurrent changes to the destination can affect what the backup
contains. The exclusive reservation prevents cooperating writers from choosing the same backup
name; coordinate writers when the backup must correspond to a particular version. An atomic
rename alone is not a guarantee of recovery after every power loss.

```luau
--!strict
local backup = require('@jess/backup')
backup.write('notes.txt', 'first\n')
local written = backup.write('notes.txt', 'second\n')
print(written.backup) -- notes.txt.bak
print(backup.write('notes.txt', 'third\n').backup) -- notes.txt.bak1
print(backup.write('notes.txt', 'fourth\n', { backup = false }).backup) -- nil
```
