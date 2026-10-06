---
title: 'jess.fs'
description: 'Files and folders.'
weight: 100
extra:
  kind: api
---

{{ api_signature(value="jess.fs: { read, write, replace, copy, remove, createDir, exists, isDir, isFile, size, modified, stat, list, cwd, join, fileName, parent, isFileName }") }}

Plain file access for scripts, with the host's filesystem permissions. jess doesn't sandbox scripts: a script can delete anything you can. Read a script before you run it, as you would a shell script.

Nothing here backs anything up unless asked. `copy` replaces what's there, and so does `write`, unless you pass it `backup = true`, when it keeps the old file the way `plugin:save` does.

{% usage_note(title="Host paths") %}
Every path here is a path on disk, used as given: relative paths are relative to the working directory, and nothing is looked up in the OpenMW VFS (use [context:readFile](@/docs/api/openmw.md#context-readfile) for that). Paths must be valid UTF-8. Errors name the path and the operating system's reason: `failed to write 'out/x.txt': Permission denied (os error 13)`.
{% end %}

## Reading and writing

### jess.fs.read

{{ api_signature(value="jess.fs.read(path: string) -> string?") }}

Returns a file's whole contents as a (byte) string, or `nil` when it doesn't exist.

- **Errors:** any other read failure, such as a folder or a permission problem.

```luau
--!strict
local text = jess.fs.read 'settings.toml'
if not text then print 'no settings; using defaults' end
```

### jess.fs.readBuffer

{{ api_signature(value="jess.fs.readBuffer(path: string) -> buffer?") }}

`jess.fs.read`, but the contents come back as a buffer. For binary files you're going to pick apart with `buffer.read*` or hand to something that takes a buffer, this skips making (and interning) a string the size of the file just to copy it out again. A 79 MB `Morrowind.esm` is one copy instead of two.

- **Errors:** the same as `jess.fs.read`.

```luau
--!strict
local esm = jess.fs.readBuffer 'Morrowind.esm'
if esm then print(`{buffer.len(esm)} bytes, starting {buffer.readstring(esm, 0, 4)}`) end
```

### jess.fs.write

{{ api_signature(value="jess.fs.write(path: string, data: string | buffer, options: { backup: boolean? }?) -> { path: string, backup: string? }") }}

Writes a file, replacing it if it exists, and creates the folders it goes in if they're missing.

- **Parameters:** `options.backup = true` copies a file that's already there to the next free `.bak`, `.bak1`, ... first, as every jess command that overwrites a file does. Off by default.
- **Returns:** where things went, the same `{ path, backup }` table `plugin:save` returns: `backup` is `nil` when nothing was backed up.
- **Errors:** a failed backup (the old file is left alone), or any write failure.

```luau
--!strict
jess.fs.write('report.txt', 'done\n')
jess.fs.write('rock-copy.nif', jess.nif.open('rock.nif', { openmw = false }):bytes())

local written = jess.fs.write('report.txt', 'done again\n', { backup = true })
print(written.backup) -- report.txt.bak
```

### jess.fs.replace

{{ api_signature(value="jess.fs.replace(path: string, data: string | buffer) -> ()") }}

Writes a file so that it is never half-written: the data goes to a temporary file beside `path` (`.openmw.cfg.jess-<pid>-<n>.tmp`), is flushed to disk, and is renamed over `path`. A crash or a full disk leaves the old file whole, and a reader never sees a mix of old and new. The old file's permission bits carry over; the folder has to exist already, since a rename can't create it. What to use for a config file another program reads.

- **Errors:** a folder that isn't there or can't be written, or a failed rename; the temporary file is removed, and `path` is as it was.

```luau
--!strict
local cfg = jess.fs.read 'openmw.cfg' or ''
jess.fs.replace('openmw.cfg', cfg .. 'content=MyMod.esp\n')
```

### jess.fs.copy

{{ api_signature(value="jess.fs.copy(from: string, to: string) -> ()") }}

Copies a file, replacing `to` if it exists.

```luau
--!strict
jess.fs.copy('MyMod.esp', 'MyMod-backup.esp')
```

### jess.fs.remove

{{ api_signature(value="jess.fs.remove(path: string) -> boolean") }}

Deletes a file, returning whether there was one. It doesn't remove folders.

- **Errors:** a folder (`Is a directory`), or a permission problem.

```luau
--!strict
print(jess.fs.remove 'MyMod-backup.esp', jess.fs.remove 'MyMod-backup.esp') -- true  false
```

### jess.fs.createDir

{{ api_signature(value="jess.fs.createDir(path: string) -> ()") }}

Creates a folder and any missing parents. A folder that already exists is fine.

```luau
--!strict
jess.fs.createDir(jess.fs.join('logs', '2026', '09'))
```

## Asking about paths

### jess.fs.exists

{{ api_signature(value="jess.fs.exists(path: string) -> boolean") }}

Whether anything is at the path.

### jess.fs.isDir

{{ api_signature(value="jess.fs.isDir(path: string) -> boolean") }}

Whether the path is a folder.

### jess.fs.isFile

{{ api_signature(value="jess.fs.isFile(path: string) -> boolean") }}

Whether the path is a file.

### jess.fs.size

{{ api_signature(value="jess.fs.size(path: string) -> number?") }}

A file's size in bytes, or `nil` when nothing is there.

```luau
--!strict
local path = 'MyMod.esp'
print(jess.fs.exists(path), jess.fs.isFile(path), jess.fs.isDir(path), jess.fs.size(path))
-- true  true  false  49483
```

### jess.fs.modified

{{ api_signature(value="jess.fs.modified(path: string) -> number?") }}

When a file was last modified, in seconds since 1970 with the fraction the filesystem keeps, or `nil` when nothing is there. Tools that sort plugins the way Morrowind's launcher did, masters first and then by date, need it.

```luau
--!strict
local older, newer = jess.fs.modified 'Morrowind.esm', jess.fs.modified 'MyMod.esp'
if older and newer then print(newer > older) end
```

### jess.fs.stat

{{ api_signature(value="jess.fs.stat(path: string) -> { size: number, isFile: boolean, isDir: boolean, modified: number?, modifiedSeconds: number?, modifiedNanoseconds: number? }?") }}

One metadata lookup, or `nil` when nothing is there: what `size`, `isFile`, `isDir`, and `modified` say, in one call instead of four. `modifiedSeconds` and `modifiedNanoseconds` are the modification time split exactly, for sorting files by time when two are closer together than a `number` can tell apart; they're absent for a time before 1970.

```luau
--!strict
local info = jess.fs.stat 'MyMod.esp'
if info and info.isFile then print(info.size, info.modifiedSeconds) end
```

### jess.fs.list

{{ api_signature(value="jess.fs.list(path: string) -> { string }") }}

The names of a folder's entries, files and folders alike, in the order the filesystem lists them. Sort them if order matters.

- **Errors:** a path that isn't a readable folder.

```luau
--!strict
local names = jess.fs.list '.'
table.sort(names)
for _, name in names do
  if name:lower():match '%.esp$' then print(name) end
end
```

## Building paths

### jess.fs.cwd

{{ api_signature(value="jess.fs.cwd() -> string") }}

The working directory, as an absolute path.

### jess.fs.join

{{ api_signature(value="jess.fs.join(...: string) -> string") }}

Joins path parts with the platform's separator. A part that's absolute replaces everything before it.

### jess.fs.fileName

{{ api_signature(value="jess.fs.fileName(path: string) -> string?") }}

The last part of a path, or `nil` when there's none (`"/"`, or a path ending in `..`).

### jess.fs.isFileName

{{ api_signature(value="jess.fs.isFileName(name: string) -> boolean") }}

Whether `name` is one plain file name under the host's path rules: not empty, no separators, not `.` or `..`, and nothing else the platform treats as more than a name (a drive or a root on Windows). What to check before joining a name from a config file onto a folder.

```luau
--!strict
print(jess.fs.isFileName 'Morrowind.esm', jess.fs.isFileName '../Morrowind.esm', jess.fs.isFileName '')
-- true  false  false
```

### jess.fs.parent

{{ api_signature(value="jess.fs.parent(path: string) -> string?") }}

The path without its last part, or `nil` for a root. A bare name's parent is `""`.

```luau
--!strict
local log = jess.fs.join(jess.fs.cwd(), 'logs', 'run.txt')
print(jess.fs.fileName(log)) -- run.txt
print(jess.fs.parent(log)) -- <cwd>/logs
print(jess.fs.join('meshes', 'x', 'rock.nif')) -- meshes/x/rock.nif on Linux
```

Related: [Files, TOML, and Regex](@/docs/scripting/files-toml-regex.md).
