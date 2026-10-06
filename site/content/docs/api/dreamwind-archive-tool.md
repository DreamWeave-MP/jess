---
title: '@dreamwind/archive/tool'
description: "Archive jobs in Luau: info, verify, diff, extraction that refuses unsafe paths, and create and add, each with a plan. Dreamwind's port of dream_archivetool."
weight: 118
extra:
  kind: api
---

{{ api_signature(value="require('@dreamwind/archive/tool'): { version, info, verify, diff, extract, extractMany, planExtract, extractAll, planExtractAll, create, planCreate, add, planAdd, ... }") }}

[`@dreamwind/archive`](@/docs/api/dreamwind-archive.md) is the format: open, look up, read, build. This module is the jobs people actually run on archives, with the policy that keeps them from going wrong: extraction that refuses `..` and absolute paths and won't overwrite what's there unless told, packing and adding that refuse symbolic links unless told, and a plan for each that says what would happen without doing it. It's a port of [dream_archivetool](https://dreamweave-mp.github.io/dream_archivetool/) 1.1.0's Luau module, `@dream/archivetool`, which jess linked until it didn't: the same functions, options, results, and messages, in the same `dream.bethesda-archives` layer as the archives themselves.

```luau
--!strict
-- tool.luau: pack a folder, check it, plan an extraction, then add to it.
local archive = require '@dreamwind/archive'
local tool = require '@dreamwind/archive/tool'
local folder: string = ...

print(tool.create('demo.bsa', folder).files)

local opened = archive.openPath 'demo.bsa'
local report = tool.verify(opened, { readPayloads = true })
print(report.format, report.fileCount, report.payloadsRead, #report.unsafePaths)

local plan = tool.planExtractAll('demo.bsa', { output = 'out', overwrite = 'skip' })
for _, row in plan.entries do
  print(row.action, row.path, row.target)
end

local added = tool.planAdd('demo.bsa', { inputs = { `{folder}/textures` }, output = 'new.bsa' })
print(added.files, added.added, added.replaced, added.preserved)
```

```text
$ jess run --grant filesystem.read --grant filesystem.write --package jpk/dreamwind tool.luau "MyMod/Data Files"
3
bsaTes3	3	3	0
extract	textures/readme.txt	out/textures/readme.txt
extract	meshes/jess/random_objects.nif	out/meshes/jess/random_objects.nif
extract	meshes/jess/all_types.nif	out/meshes/jess/all_types.nif
4	1	0	3
```

The last line is a folder's files going in by their paths below it: `textures/readme.txt` there is `readme.txt` in the archive, a new member beside the three it keeps.

## The functions

Every function that reads an archive takes it as a host path, opened for that one call, or as an archive `@dreamwind/archive` opened already, so several jobs on one archive read its index once. Results are plain tables.

| Function | Does |
| --- | --- |
| `info(archive)` | Format, counts, whether `add` could rewrite it and why not, and the TES4 or BA2 header facts. |
| `verify(archive, { readPayloads? })` | Duplicate and unsafe names, and with `readPayloads` every named file read to the end. |
| `diff(old, new, { fingerprintPayloads? })` | Added, removed, and changed entries by normalized path; with `fingerprintPayloads`, an FNV-1a fingerprint of every file. |
| `extract(archive, entry, options?)` | One entry, by name in any spelling or by an entry of `archive:entries()`. |
| `extractByPathHex(archive, hex, options?)`, `extractHex` | One entry by its `pathBytesHex` key. |
| `extractMany(archive, entries, options?)`, `extractManyByPathHex` | Several in one pass: every one must exist, every target be safe and distinct, and under `overwrite = 'fail'` none may exist, or nothing is written. |
| `planExtract(archive, entries, options?)`, `planExtractByPathHex` | What `extractMany` would do: a row per entry with its target and action. |
| `extractAll(archive, options?)`, `planExtractAll` | Every named entry, and its plan. An archive with unnamed entries is refused. |
| `create(output, input, options?)`, `planCreate` | Packs a folder or a file into a new archive, and its plan. |
| `add(archive, options)`, `planAdd` | Writes the archive again with the inputs added or replacing entries, and its plan. A host path only. |

Extraction options: `output` (`'.'`), `overwrite` (`'fail'`, `'overwrite'`, or `'skip'`), and, except for the `All` forms, `preservePaths` (`true`; `false` writes each file straight into `output`). Creating takes `format` (`'tes3'` or `'bsaTes3'`, `'tes4'` or `'bsaTes4'`, `'ba2'`; `'tes3'` by default), `tes4Version` (`'oblivion'`, `'fallout3'`, `'skyrim'`, `'skyrimSe'` or `'sse'`), `ba2Kind` (`'gnrl'`, `'dx10'`, or `'gnmf'`, which is refused), `ba2Version` (`'fallout4'`, `'starfield'`, `'fallout4NextGen'`), `compress`, and `followSymlinks`; an option for another format is an error (`compress is not valid with format bsaTes3`). Adding takes `inputs` (one or more host paths), `output` (the archive itself when left out, but never named as it), and `followSymlinks`. Option tables are strict, as everywhere: `extract: unknown option 'overwirte'; known options are fsync, output, overwrite, preservePaths`.

What the reports and plans hold, field for field, is on [dream_archivetool's Luau pages](https://dreamweave-mp.github.io/dream_archivetool/docs/luau/results/): `VerifyReport`, `DiffReport`, `ExtractPlan`, `CreatePlan`, and `AddPlan`, and the `info` table on [its module page](https://dreamweave-mp.github.io/dream_archivetool/docs/luau/module/). Here they're what the native module's `:toTable()` returned: a size is a number, a fingerprint an integer, and a field with nothing to say is left out. `pathBytesHex` is an entry's normalized path bytes in hex, the stable key for names that aren't text.

Failures raise the crate's messages: `archive entry not found: missing.dds`, `target already exists: out/readme.txt`, `unsafe archive path: ../readme.txt`, `refusing to follow symlink input path: <path>; set followSymlinks to opt in`, and `archive error: ...` or `I/O error: ...` for the rest.

## Where the port differs

- **Opened archives are arguments.** The native module put `toolInfo`, `verify`, `diff`, and the extraction functions on every archive handle as methods. Here they're the module's functions taking the archive first: `tool.verify(opened)` for `opened:verify()`, `tool.info(opened)` for `opened:toolInfo()`. An error names the function where the method's named the method (`planExtract.entries[1]: ...` for `archive:planExtract.entries[1]: ...`), and an archive opened from bytes is `<memory>` in reports, as before.
- **Results are tables.** Reports and plans were userdata with views over their rows and a `:toTable()`; here they're the tables `:toTable()` gave, so `ipairs` and `table.sort` work on them and there's no `tostring` name.
- **`fsync = true` is refused**, naming the option. L3i can flush a writer, but doesn't expose directory syncing. A tool that says it synced when it didn't is worse than one that says it can't.
- **Existing targets are checked, not locked.** Under `overwrite = 'fail'` or `'skip'` the crate renames a finished file into place only if nothing is there, in one step; the port still looks again just before an ordinary `@dream/fs` rename. Something that appears in between is overwritten.
- **Symbolic links are found by where they lead**: a path whose real location isn't its folder's plus its name. A link that leads nowhere fails as an archive error rather than as a refused link. This is the current port's policy, not a limitation of L3i's filesystem API.
- **Adding in place leaves the archive with new permissions**, a new file's, where the crate kept the old ones. L3i doesn't expose permission setters.
- **Names are bytes everywhere.** The crate also refused names that aren't UTF-8 as extraction targets on macOS and Windows; the port treats every host as Linux does.

## Testing it

```sh
jess package test jpk/dreamwind
cargo test -p jess-lua --test archive_experiment
```

`tests/archive/tool.luau` is the module's own suite, dream_archivetool's tests and its Luau API's, run from source and packed. `tests/archive/tool_differential.luau` runs the jobs that only read on every archive the archive suites build and break, and every job on folders, links, and archives on disk, and holds each result to the native module's, frozen in `tests/archive/golden/differential.txt` while jess still linked it. [`@dreamwind/archive`](@/docs/api/dreamwind-archive.md#testing-it) has how those goldens change now.
