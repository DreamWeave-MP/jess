---
title: 'Starwind on Dreamwind'
description: 'The Starwind Merged Plugin Project''s Definitive build, ported from Python and four native tools to Luau: what runs, how to run it, how close it comes, and how fast.'
weight: 97
extra:
  kind: guide
---

The [Starwind Merged Plugin Project](https://github.com/DreamWeave-MP/s3stash) builds Definitive Starwind, the `Star_Data.omwaddon` and `Starwind.omwaddon` pair, from 316 MB of JSON. `build_starwind.py --strict` gets there by running four native programs 152 times (`tes3cmd` 81 times, `tes3conv` 61, `merge_to_master` 9, motherJungle's `addVanillaRefs` once), then grading the result with five Python validators, each of which converts the plugin back to JSON to look at it.

Dreamwind's `dream.starwind` layer does all of it in Luau, in one process, running no other program. It is the proof that a package can carry a real build: thirteen source plugins, eight merges, a recursive decoupling from the Bethesda masters, an independent replay of OpenMW's dialogue loading, and a partition checked byte for byte.

## Running it

`@dreamwind/starwind` exports `build`. A three-line script runs it:

```luau
--!strict
-- build-starwind.jsl: the Definitive build, as build_starwind.py --strict --keep-work runs it.
local starwind = require '@dreamwind/starwind'
local root: string, output: string = ...
local manifest = starwind.build { root = root, output = output, strict = true, keepWork = true }
print(starwind.report.encode(manifest.split_counts))
```

```sh
jess run --package jpk/dreamwind build-starwind.jsl ~/s3stash/content/sw_merged /tmp/starwind
```

`root` is the `sw_merged` folder, whose `plugins` folder holds the JSON sources. `output` defaults to `<root>/.swbuild/definitive`, where the Python build writes, and gets the same tree:

```text
out/Starwind-Definitive.omwaddon     the masterless monolith the split works from
out/Star_Data.omwaddon               definitions
out/Starwind.omwaddon                content, mastered on Star_Data
reports/                             main quest, source hygiene, closure, dialogue, late actors, split
work/                                with keepWork: the pre-decoupling plugin and decoupleLog.txt
build-manifest.json                  every source's SHA-256, the reports, and the counts
```

The closure report lands in `reports/` with the others; the Python build files it one folder up, under `.swbuild/reports/`, and the manifest names the path either way. A build that fails a check raises with the message the Python build exits with. `strict = false` lets dialogue order and link drift through, as the Python build does without `--strict`.

## What replaced what

| Python build | Dreamwind |
| --- | --- |
| `tes3conv` (JSON to plugin, plugin to JSON) | `@dreamwind/tools/tes3conv`: a JSON reader into the same record tables `record:decode()` gives, and a writer that terminates strings and orders records the way tes3conv 0.4.1 does |
| `merge_to_master` | `@dreamwind/tools/merge-to-master` |
| motherJungle's `addVanillaRefs` | `@dreamwind/tools/add-vanilla-refs` |
| `tes3cmd delete` and `modify` | the build's own edits (`starwind.surgery`), one function per command line |
| `motherjungle_audit.audit`, `dialogue_chain_audit.py`, `dialogue_source_hygiene.py`, `late_actor_provenance.py`, `split_starwind.py` | `starwind.audit`, `starwind.dialogue`, `starwind.hygiene`, `starwind.provenance`, `starwind.split` |

The three tools are generic: they take and return record lists, header first, and know nothing about Starwind. They live in `dream.morrowind-tools` with TES3Merge and friends; `dream.starwind` requires that layer and exports `@dreamwind/starwind`.

Plugins are never parsed in Luau. The sources are JSON, so the build reads JSON and keeps records as tables the whole way; jess encodes them once, at the end, and the validators read the written files back through `jess.plugin`, so they grade what was written, not what was meant.

### Faithful, warts included

The port does what the Python build does, not what its command lines appear to ask for. Every one of these is in the shipped Definitive plugins, so every one of them is in the port, commented where it happens:

- `tes3cmd modify --replace "/Name:Asteriod/Name:Asteroid/"` rewrites field values, never the `Name:...` text a dump prints, so neither typo fix changes anything.
- `--sub-no-match "ENAM:"` matches no subrecord's text, so the 375 enchantment capacity lands on every clothing, armor, and weapon record, enchanted or not.
- `--instance-match` is a scalar option: given twice, the second wins.
- A `delete --sub-match` rebuilds every record it looks at and drops their header flags, so all of Naboo loses `PERSISTENT` and `BLOCKED`.
- Deleting a leveled creature's `CNAM` leaves its `INTV`, which tes3 reads onto the entry before.
- A cell's `NAM0` (how many references are temporary) travels with the reference before it, so deleting that reference deletes the count, and every reference left reads as persistent.
- Exact ids and `--match` are case-insensitive; `ObjIdx:8517` also matches `ObjIdx:85170`.

The one place the port uses different inputs on purpose is the late-actor report. `late_actor_provenance.py` replays dialogue against two legacy defaults, a Solo build under `.swbuild/solo` and the canonical vanilla JSON, instead of the Definitive build's own, and a fresh checkout of the Python build fails at that step until something builds Solo. The port uses the projection the dialogue audit makes from the staged masters and the pre-decoupling plugin. The late-actor list itself (empty, today) doesn't change; the liveness counts the report embeds do. The Python build's diagnostic `tatooine_objidx_397.dump.txt`, a tes3cmd dump nothing reads, isn't written.

## How close it comes

Against the release pair checked in beside `index.md`, and against a fresh `build_starwind.py --strict` run of the same sources:

| Output | Against the release pair | Against a Python run |
| --- | --- | --- |
| `Star_Data.omwaddon` | byte for byte identical | byte for byte identical |
| `Starwind-Definitive.omwaddon` | not checked in | 30,548 records in the same order: 30,002 byte-identical, and 546 cells that differ only in reference numbering or in which references are temporary |
| `Starwind.omwaddon` | 17,218 records in the same order: 16,673 byte-identical, 537 renumbered cells, 8 that also disagree about which references are temporary | the same 16,673 byte-identical, and 545 cells split between the two kinds |
| main-quest, closure, dialogue, source hygiene, split reports | | equal, field for field; the split report's dependency lists, which follow reference order, as multisets |
| late-actor report | | equal but for its liveness counts (above) |

The cells are the Python build's own nondeterminism, not the port's. `merge_to_master` numbers a merged plugin's references in hash-map order, which changes from run to run, and where the tombstone sweep loses a cell's `NAM0` follows that numbering. So the split between the two kinds moves with every Python run: in the monolith, two of them came out at 535 renumbered and 11 temporary-flag cells, then 540 and 6. Every other record is identical every time. The port numbers in record order, and two of its runs write the same bytes.

Every invariant `index.md` documents holds: 30,548 records, no masters, no unresolved hard references, 591 of 591 topics, no missing or extra INFOs, no order, physical-order, link, or payload mismatches, no Star_Data dependency on Starwind, and an exact split of 13,330 and 17,218 records.

`tests/starwind/equivalence.jsl` checks all of that. `cargo test --release -p jess-cli --test cli starwind::` runs it when `JESS_STARWIND` names an `sw_merged` folder, and against a Python run's outputs and reports too when `JESS_STARWIND_PYTHON` names its `.swbuild/definitive`:

```sh
JESS_STARWIND=~/s3stash/content/sw_merged \
JESS_STARWIND_PYTHON=~/s3stash/content/sw_merged/.swbuild/definitive \
  cargo test --release -p jess-cli --test cli starwind:: -- --nocapture
```

`jess package test jpk/dreamwind` runs the ports' own cases, among them addVanillaRefs's Rust tests, case for case.

## How fast

Wall time and peak memory, end to end from the JSON sources to the split pair and every report, median of several runs on an Intel i7-10870H (8 cores, 16 threads) under a load average of 4 to 5 from other work:

| Build | Runs | Wall time | Peak memory |
| --- | --- | --- | --- |
| `python3 build_starwind.py --strict` (Python 3.14) | 3 | 162.6 s (157.2 to 172.0) | 1.2 GB, its largest process |
| `jess run --package jpk/dreamwind`, `strict = true` | 5 | 16.9 s (16.4 to 21.1) | 1.8 GB |

Ten times faster, with one process holding everything the Python build keeps in files. The memory is the price: the port holds every source as decoded records at once, where the Python build holds one tool's worth at a time and lets the disk carry the rest.

Where the Python build's time goes, from `cProfile`: 150 of its 176 seconds are spent waiting on other programs. The 134 tool runs it makes itself take 63 seconds, and the three validators it starts as separate Python processes (source hygiene, dialogue, late actors), which convert plugins back to JSON to read them, take 86 more. The rest is Python's own JSON parsing (11 seconds) and its in-process checks.

Where the port's goes, from the `timings` table `build` fills in (CPU seconds):

| Stage | Time |
| --- | --- |
| compile (JSON to records) | 5.8 s |
| split and its exact-reconstruction check | 3.1 s |
| decouple (addVanillaRefs) | 2.0 s |
| write | 1.4 s |
| source hygiene | 1.3 s |
| merge (merge_to_master) | 1.0 s |
| dialogue audit | 0.9 s |
| read back | 0.8 s |
| preprocess (the tes3cmd surgery) | 0.6 s |
| closure, late actors | 0.2 s |

A third of it is reading 316 MB of JSON. The checks that cost the Python build nearly two minutes (its three validators, the main-quest guard, and the split) cost the port about six seconds. Pass a table as `timings` to get the same breakdown from your own run.

*only baby scars*
