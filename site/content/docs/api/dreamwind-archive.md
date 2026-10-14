---
title: '@dreamwind/archive'
description: "BSA and BA2 archives in Luau: Dreamwind's port of dream_archive, the API its @dream/archive binding had, under its own name."
weight: 117
extra:
  kind: api
---

{{ api_signature(value="require('@dreamwind/archive'): { version, openPath, openBytes, detectPath, guessFormat, normalizePath, ba2, bsa }") }}

Bethesda shipped three archive formats in fifteen years and called two of them BSA. Morrowind's (`bsaTes3`) is a flat table of names, hashes, and stored payloads. Oblivion through Skyrim SE (`bsaTes4`) group files into folders, compress with zlib or, from version 105, LZ4 frames, and store names in string tables, inside the payloads, both, or nowhere at all. Fallout 4 and Starfield (`ba2`) split files into chunks, compress them with zlib or LZ4 blocks, and keep textures without their DDS headers, which have to be rebuilt from a texture record on the way out.

`@dreamwind/archive` reads, extracts, and builds all three, in Luau. It is Dreamwind's `dream.bethesda-archives` layer, a port of [dream_archive](https://dreamweave-mp.github.io/dream_archive/) 1.1.0, whose native binding, `@dream/archive` and the `dreamArchive` global, it replaced in jess. The port answers the binding's calls with the binding's results and the binding's error messages, so moving a script over is one `require`:

```luau
--!strict
-- list-bsa.jsl: what's in an archive, and one file out of it.
local archive = require '@dreamwind/archive'

local builder = archive.bsa.tes3.Builder.new()
builder:addBytes('Meshes/X/Hello.txt', 'hello')
builder:addBytes('textures/tx_rock.dds', string.rep('\0', 128))
builder:writePath 'demo.bsa'

local opened = archive.openPath 'demo.bsa'
print(opened:format(), opened:len()) -- bsaTes3  2
print(opened:readFile 'MESHES\\x\\hello.TXT') -- hello
for _, entry in opened:entries() do
  print(entry.index, entry.path, entry.size, entry.hashHex)
end

local into = buffer.create(128)
print(opened:readInto('textures/tx_rock.dds', into)) -- 128
```

```text
$ jess run --package jpk/dreamwind list-bsa.jsl
bsaTes3	2
hello
1	textures\tx_rock.dds	128	071d635d809c71f8
2	meshes\x\hello.txt	5	102f1654a26acb2a
128
```

The entries come out in the order the archive stores them, which for a TES3 builder is hash order. Nobody asked for that order. Bethesda's tools wanted it, so the builder writes it, and the same members make the same bytes every time.

## The module

| Module | Has |
| --- | --- |
| `@dreamwind/archive` | `openPath`, `openBytes` (detect the format), `detectPath`, `guessFormat`, `normalizePath`, `version`, and the submodules as `ba2` and `bsa`. |
| `@dreamwind/archive/ba2` | `openPath`, `openBytes`, `hashFile`, `Builder.new`, `Dx10Builder.new`, `compression`, `version`. |
| `@dreamwind/archive/bsa` | `encodeFilename`, `decodeFilenameLossy`, `normalizePath`, `encoding`, and `tes3`, `tes4`. |
| `@dreamwind/archive/bsa/tes3` | `openPath`, `openBytes`, `hashFile`, `Builder.new`. |
| `@dreamwind/archive/bsa/tes4` | `openPath`, `openBytes`, `hashDirectory`, `hashFile`, `Builder.new`, `nameMode`, `profile`, `archiveTypes`. |

Each name is `@dream/archive`'s with `@dreamwind` in front, and does what that one does: [dream_archive's Luau pages](https://dreamweave-mp.github.io/dream_archive/docs/luau/) are the member-by-member reference for both. Every table is frozen, and `require('@dreamwind/archive/ba2')` is `archive.ba2`.

## Opening

`openPath` reads an archive in place. It maps the file through L3i's `@dream/fs`, reads the index once, and checks it the way the crate does, in the crate's order, so a malformed archive fails at open with the crate's message. Names, lookup tables, and hash tables are built the first time something asks for them: an archive opened to read one file never builds the other forty thousand names. Payloads are read when asked for, straight from the map into the buffer that wants them.

Host reads require `filesystem.read`; builders and extraction that write files require `filesystem.write`. The host grants these capabilities, not the package. Requiring the modules or working entirely with bytes needs neither. For a filesystem job, use `jess run --grant filesystem.read --grant filesystem.write --package jpk/dreamwind ...`.

`openBytes` takes a string or a buffer and keeps a copy, as the crate does: a caller's buffer is the caller's to change afterwards. The format-specific `openPath` and `openBytes` in each submodule skip detection and say what's wrong with the wrong family (`invalid magic read from archive header: 0x00000100`), where the top-level ones say `unknown or disabled archive format`.

## Entries

`archive:entries()` is a frozen array of entry tables, one per entry for the archive's life, so `archive:get(path) == archive:entries()[i]` when they name the same entry. An entry is made the first time something asks for it, holding what its index record says outright (`index`, `id`, `format`, `path`, `storedSize`, `offset`, `compressed`, the hashes, and `size` when no decoding is needed to know it); the rest (`name`, `folder`, `hashHex`, `folderHashHex`, a compressed TES4 entry's `size`, and so on) is worked out the first time it is read, and kept. Assigning to any of it is an error.

Hashes are the native binding's: TES3 and TES4 hashes are 64-bit Luau integers (`entry.hash`, TES4's `entry.folderHash`) with `hashHex` beside them, BA2 hashes are the three `u32` fields. `containsHash` and `getByHash` take `(hash)` for TES3, `(folderHash, fileHash)` for TES4, and `(directory, file, extension)` for BA2.

## Reading and extracting

Every lookup matches a path the way OpenMW's VFS does: `\` and `/` are one separator, ASCII case is ignored, leading and repeated separators go, and a trailing separator names a folder, so it matches no file. A TES4 archive without names, or a BA2 without a string table, is looked up by hashing the path.

`readFile`, `readFileRequired`, `extractFile`, `readEntry`, and `extractEntry` return strings. `readInto(entry or path, buffer, offset?)` writes into a buffer you own and returns the count, and for a stored payload it is one copy, from the file's map into your buffer. `extractTo`, `extractToWithEncoding` (BSAs), and `extractToWithPaths` (TES4, for hash-only archives and a list of candidate names) refuse any path with a `..`, a NUL, or a `:` before writing anything, make every folder, and write each file beside its target before renaming it over, so a failed write leaves the old file whole.

## Building

The four builders (`ba2.Builder`, `ba2.Dx10Builder`, `bsa.tes3.Builder`, `bsa.tes4.Builder`) write what the crate writes, byte for byte, compressed members included: the codecs are the same Rust libraries behind [`@dream/bytes`](@/docs/api/downstream-globals.md). `addFile` and `addDir` record a host file's size and read it when the archive is written, which fails if the file changed size in between. `writePath` creates the file first and streams into it; `toBytes`, `toString`, and `toBuffer` build it in memory.

## Where the port differs

On purpose, and only here:

- **Lists and handles are tables.** `entries()` is a frozen array with a `toTable()` for a mutable copy, not a sequence view; entries are frozen tables, and `pairs` sees only the fields already in them, not the lazy ones. Two reads of an entry are the same table, so `rawequal` holds where the native handles needed `__eq`.
- **The tool is a module of its own.** dream_archivetool put `toolInfo`, `verify`, `extractAll`, and the rest on every native archive handle as methods. They're the application layer, not the format: [`@dreamwind/archive/tool`](@/docs/api/dreamwind-archive-tool.md) has them as functions that take the archive first.
- **Codec failures read differently after the colon.** A corrupt payload is `zlib decompression failed: ...`, `lz4 decompression failed: ...`, or `LZ4 frame operation failed: ...` in both; the detail is miniz_oxide's one-shot decoder's in the port and flate2's streaming one's in the crate. Past the declared size the crate decodes one byte more to tell a longer stream from a corrupt one, and the port caps its decoder one byte past the size: a stream corrupt exactly there can fail as corrupt in one and as one byte too long in the other.
- **A header that claims billions of entries** fails in the crate when it can't reserve the table, and in the port when the archive runs out of bytes: `unexpected end of file`, or whatever the bytes past the real index make of themselves.
- **Trailing bytes after a zlib stream** are found by comparing the stream's last four bytes with the Adler-32 of what it decoded to, since `@dream/bytes` doesn't say how much input it read. A trailer that happens to end in that checksum would pass.
- **Directory symlinks** are followed by `addDir`; the crate skips them.
- **Host errors** retain the operating system's reason, without L3i's operation/path prefix, so they read as the crate's do (`No such file or directory (os error 2)`). Native I/O refusals become archive errors; bad arguments and missing capability grants still throw directly.
- **Argument errors** read as the binding's (`dream.archive.Archive.readEntry: bad argument #1 (expected integer): Lua stack index 2: number 1.5 is not an exact i64`), down to the stack index, but the module name in them is still `dream.archive`.

## Testing it

```sh
jess package test jpk/dreamwind
cargo test -p jess-lua --test archive_experiment
JESS_DATA_FILES="$HOME/Games/Morrowind/Data Files" cargo test --release -p jess-lua --test archive_experiment the_port_matches_the_crate
```

`tests/archive/run.jsl` is the port's own suite: dream_archive's tests and its Luau API contracts, held to the port alone, with the crate's small fixtures beside it. `jess package test` runs it from the source and from the packed JPK. The harness runs it again with a scratch folder for the cases that write files.

`tests/archive/differential.jsl` makes one observation per case: every builder call's outcome and the bytes it built, every field of every entry, every read (as a length and an xxh3 digest) or its error, `readInto` at an offset, lookups by path in five spellings and by hash, and the errors. The cases: 160 seeded builder recipes across the four builders, with bad options, bad paths, duplicates, and bad textures mixed in; 88 archives written by hand (hash-only and embedded-name TES4s, LZ4 frames with the checksums, linked blocks, and stored blocks the crate never writes, stringless and multi-chunk BA2s, GNMF, colliding hashes); 1,392 malformed copies of those and of the 144 archives the recipes built, each a flipped byte, a replaced `u32`, a cut, or junk on the end; the crate's fixtures, from bytes and in place; extraction to disk, plain, with two encodings, and by a dictionary; builders over files and folders; and [the tool](@/docs/api/dreamwind-archive-tool.md#testing-it)'s jobs. With `JESS_ARCHIVE_FIXTURES` naming dream_archive's `tests/fixtures`, its large texture fixtures too, and with `JESS_DATA_FILES` or `JESS_ARCHIVES`, real archives, read in place and compared entry by entry.

Each observation's digest is in `tests/archive/golden/differential.txt`. They were recorded from the native binding while jess still linked it, with the port making the same observation on every case, so they're the crate's answers, and the binding being gone doesn't make them move. A real archive's golden is named by its file name and size. Morrowind.bsa's is there; any other archive fails as a case without a golden, and blessing it would only record what the port says, which checks nothing against the crate. `JESS_BLESS=1` records the port's answer for a case that has no golden yet, which makes it the port's word, not the crate's: check a new case against dream_archive's source before you keep it. An existing golden never changes by blessing. One that has to is deleted from the file by hand, and the commit that does it says why the port answers differently now.

## Numbers

`cargo bench -p jess-lua --bench archive_experiment` times the port from Luau: synthetic archives of every family (4,000 stored TES3 entries; 1,000 16 KiB members in TES4 zlib, TES4 LZ4 frames, BA2 zip, and BA2 LZ4; 300 BC7 DX10 textures), opened from bytes and in place, then building and extraction, then every `.bsa` named by `JESS_DATA_FILES` or `JESS_ARCHIVES`. Each row is the median of seven batches. The figures below are from while jess still linked the native binding and the bench timed both on the same bytes, through the same call boundary: the medians of three runs of a verified `--release` build on a 16-thread machine with a load average between 3.4 and 4.9. Times are microseconds per workload; under 1.00, the port is faster.

Morrowind.bsa (1 GB, 26,204 entries), read in place:

| Workload | `@dream/archive` | `@dreamwind/archive` | port / native |
| --- | ---: | ---: | ---: |
| open + index | 9,305 | 3,096 | 0.33 |
| open + first lookup | 9,170 | 9,906 | 1.08 |
| 52,408 lookups by path | 7,619 | 4,509 | 0.59 |
| 26,204 lookups by hash | 3,091 | 2,941 | 0.95 |
| list paths and sizes | 35,395 | 44,006 | 1.24 |
| read 2,016 files | 313,888 | 339,880 | 1.08 |
| `readInto` 2,016 files | 296,614 | 321,205 | 1.08 |

Across the synthetic archives:

| Workload | port / native |
| --- | ---: |
| open + index, from bytes | 0.45 to 0.64 |
| open + index, in place | 0.11 to 0.88 |
| open + first lookup | 0.66 to 1.32 |
| lookups by path | 0.51 to 0.58 |
| lookups by hash, TES3 and TES4 | 0.94 to 1.10 |
| lookups by hash, BA2 | 1.43 to 1.48 |
| list paths and sizes | 0.51 to 1.55 |
| read stored files (TES3) | 0.80 to 1.09 |
| read compressed files and textures | 1.14 to 1.35 |
| `readInto` compressed files | 1.08 to 1.55 |
| build (all four builders) | 0.99 to 1.09 |
| extract to disk | 1.39 to 1.58 |

Where it's slower, and why:

- **Compressed reads** copy the stored bytes out of the map into a buffer, hand that to the codec, and, for zlib, run Adler-32 over the output to find trailing junk (see above). A string result is one more copy. The crate decodes straight from the map. The codec itself is the same Rust either way, so the gap stays near a quarter.
- **Extraction** runs on one thread. The crate extracts on every core with rayon.
- **BA2 hash lookups** build a 64-bit key from two of the three hashes in Luau on every query; the crate compares a packed struct.
- **The first lookup** builds the whole name index in Luau: cheap per name, but every name. Opening is cheaper because the port reads only the header and record table until something asks for more, so open plus first lookup comes out about even.
- **Listing a TES4** in place is the one listing that loses, by half. Nothing in it is obviously wasted yet; it's the next thing to look at.

One row flatters the port. Lookups by path ask the same paths every batch, and the port remembers up to 65,536 answers per archive, so after the first batch most of its lookups are one table read; the crate hashes every time. Open + first lookup is the honest cold number.

The bench runs a full collection between batches, outside the timings. It had to while it timed the native binding too: a native archive opened from bytes kept its copy outside Luau's heap, where the collector doesn't count it, and without the collection those copies piled up past 4 GB before a cycle finished. The port's copies are Luau buffers, which the collector does count.
