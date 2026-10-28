---
title: 'Keep a Backup When Writing'
description: 'Replace a file while preserving its previous contents.'
weight: 20
extra:
  kind: guide
---

When a tool replaces a person's file, the previous bytes deserve a way back. `@jess/backup` copies the existing file to the next unused backup name before replacing it. Try this in a scratch directory:

```luau
--!strict
local backup = require '@jess/backup'

backup.write('notes.txt', 'first\n')
local result = backup.write('notes.txt', 'second\n')
print(result.backup)
```

Save it as `write.jsl` in that directory and run:

```sh
jess run --capability filesystem.read --capability filesystem.write write.jsl
```

On a fresh directory the output is `notes.txt.bak`. The new `notes.txt` contains `second`; the backup contains `first`. If the backup name is already taken, the next available numbered name is used. This is why the example belongs in scratch: repeating it changes files and creates more backups.

The read and write grants are both required. A failed backup leaves the original in place; a failed replacement does too. The [backup reference](@/docs/api/backup.md) describes the return value, options, and failure behavior. It is not a general version-control system: decide how long your application keeps backups and how a user restores them.
