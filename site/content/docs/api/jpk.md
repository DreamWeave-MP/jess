---
title: '@jess/jpk'
description: 'The JPK container: a narrow ZIP subset, read lazily under budgets and written as one canonical byte sequence.'
weight: 114
extra:
  kind: api
---

{{ api_signature(value="require('@jess/jpk'): { open, write, checkPath, budgets }") }}

A [built-in module](@/docs/scripting/built-in-modules.md), typed as `Module__jess_jpk`. A JPK is a ZIP file, and this is the part of jess that reads and writes it. It knows entries, paths, and bytes. It does not know what a package is: manifests, digests, and layers are [`@jess/package`](@/docs/api/package.md)'s.

It is Luau, all of it: the end record, the central directory, the local headers, the budgets, the canonical layout. The codecs and the CRC are [`@dream/bytes`](@/docs/scripting/downstream-libraries.md)'s, so the only native code a JPK passes through is the decompressor.

It is not a general ZIP library, and it says so. A general ZIP library is a swamp: twenty compression methods, three encryption schemes, ZIP64, spanned archives, and every tool's private extra field. This reads what a package needs and refuses the rest by name:

| Reads | Refuses |
| --- | --- |
| Stored (method 0), Deflate (8), Zstandard (93) | Every other method, by number |
| One-disk archives with a comment or without | Multi-disk archives, ZIP64, encrypted entries |
| Folder entries other writers add (skipped) | An entry path that isn't canonical: absolute, `..`, `.`, empty components, backslashes, colons, control bytes, a trailing slash, not UTF-8 |
| Extra fields (ignored) | The same path twice |

## The canonical writer

`write` makes one byte sequence from one set of files, on every machine. Entries are sorted bytewise by path. Each is Zstandard when that's smaller and stored when it isn't, so a PNG or an already-packed archive isn't compressed twice. Every header field that could vary is fixed:

- the time is DOS 1980-01-01 00:00, which is as close to "no time" as ZIP allows;
- "made by" is MS-DOS 6.3, so no Unix permission bits ride along in the external attributes, which are zero;
- the UTF-8 flag is always set, and nothing else is;
- no extra fields, no comments, no data descriptors.

A JPK packed on Windows and one packed on Linux from the same files are the same file. `jess pack` is this writer; the [package digest](@/docs/api/package.md#the-content-digest) doesn't depend on it, but reproducible artifacts are nice to have anyway.

The Zstandard encoder is ruzstd's, through `@dream/bytes`' `zstdCompress`, at the one level it implements, roughly zstd's level 1. That buys decompression speed over ratio, which is the right trade for something read far more often than it's written. Any Zstandard reader opens the result: Python's `zipfile`, for one, reads a packed JPK and checks every CRC.

## jpk.open

{{ api_signature(value="jpk.open(source: buffer | JessJpkSource, budgets: JessJpkBudgets?) -> JessJpkArchive") }}

Reads the end record and the central directory and checks every entry against the budgets, before reading a single entry's bytes. `source` is the whole archive as a buffer, or `{ size, read }`, where `read(offset, length)` returns that many bytes: a source over a file reads only the directory and the entries asked for, never the whole file.

```luau
--!strict
local jpk = require '@jess/jpk'
local archive = jpk.open(jpk.write {
  { path = 'jess.toml', data = buffer.fromstring 'manifest_version = 1\n' },
  { path = 'src/init.jsl', data = buffer.fromstring(string.rep('return 1\n', 100)) },
})

for _, entry in archive.entries do
  print(entry.path, entry.method, entry.size, entry.compressedSize)
end

print(buffer.tostring(archive.read 'jess.toml'))
```

```text
jess.toml	stored	21	21
src/init.jsl	zstd	900	65
manifest_version = 1

```

The archive is `{ entries, find, read }`:

- `entries` is every file entry, in central-directory order, each `{ path, method, compressedSize, size, crc32, offset }`. `method` is `'stored'`, `'deflate'`, or `'zstd'`.
- `find(path)` is the entry with that path, or nil.
- `read(entry | path)` unpacks one entry. Its size and CRC-32 are checked: a corrupt entry is an error (`jpk: 'a.bin' fails its CRC-32`), never wrong bytes.

### Budgets

A ZIP file declares its own sizes, and a corrupt or hostile one declares whatever it likes. The reader checks the declarations before it believes them, so a broken archive fails on purpose instead of asking for 4 GiB.

| Budget | Default | Limits |
| --- | --- | --- |
| `maxEntries` | 65535 | The entry count. |
| `maxPathLength` | 1024 | One entry path, in bytes. |
| `maxCentralDirectory` | 16 MiB | The central directory. |
| `maxEntrySize` | 1 GiB | One entry, unpacked. A Luau buffer can't be bigger anyway. |
| `maxTotalSize` | 4 GiB | Every entry, unpacked, added up. |

Pass any of them to `open` to tighten or loosen it; `jpk.budgets` is the defaults. Decompression is capped at each entry's declared size as it runs, so a stream that claims 100 bytes and inflates to a gigabyte is an error at byte 101, not a gigabyte later.

This is not a sandbox, and the budgets are not a security boundary. They make corrupt archives fail deliberately. That's all.

## jpk.write

{{ api_signature(value="jpk.write(files: { JessJpkFile }) -> buffer") }}

The canonical archive of `files`, each `{ path, data }`, as described above. A path that isn't canonical, or one given twice, is an error. More than 65535 entries, or an archive past 4 GiB, is an error rather than ZIP64.

## jpk.checkPath

{{ api_signature(value="jpk.checkPath(path: string) -> string?") }}

Why `path` can't name an entry, or nil when it can. Package manifests are checked with this, so a `jess.toml` can't name a file the container couldn't hold.
