---
title: 'Benchmarks'
description: 'The harness behind every performance number on this site: the showcase against the native tools, the runtime canaries, and comparing two runs.'
weight: 35
extra:
  kind: guide
---

Every performance number on this site comes out of one script, `tools/bench/bench.py`, and nobody types them in. The harness runs Dreamwind's load-order tools next to the native tools they port, checks that both wrote the same files, and records what it measured and where. The pages print the numbers from a data file the harness writes. "Feels faster" isn't a unit.

It's Python 3.11 or later with nothing to install, because its job is starting processes and waiting for them: `os.wait4` hands back each run's CPU time and peak memory, and a Rust crate would only add a lock file entry and a build to do the same. Linux only, for the same reason.

## The showcase

```sh
cargo build --release -p jess-cli
python3 tools/bench/bench.py showcase
```

It measures the release build (the repo's release profile, fat LTO) over your real load order:

| Workload | jess | Native |
| --- | --- | --- |
| `tes3merge` | `@dreamwind/tools/tes3merge` | TES3Merge (`tes3merge` on `PATH`, or `--tes3merge`) |
| `s3lightfixes` | `@dreamwind/tools/s3lightfixes` | S3LightFixes (`s3lightfixes` on `PATH`, or `--s3lightfixes`) |
| `s3lightfixes-sun` | the same, `disable_interior_sun = true` | the same |
| `mergedlands` | `@dreamwind/tools/merged-lands` | Merged Lands 0.2.0, which the harness clones and builds under `target/bench/src` (or `--merged-lands`) |
| `doctor` | `@dreamwind/tools/doctor` | none exists |
| `doctor-assets` | `@dreamwind/tools/doctor`, `checks = { 'assets', 'meshes' }` | none |
| `doctor-rest` | `@dreamwind/tools/doctor`, the other four checks | none |

`--only tes3merge mergedlands` runs some, `--no-native` skips the originals, and the harness's own `--openmw-cfg` picks the config to flatten (without it, OpenMW's own, found the way OpenMW finds it).

Build it in a shell without [`tools/dev/no-lto.sh`](@/docs/contributing/building-and-testing.md#building-without-lto). A binary built without the verified toolchain's cross-language LTO says so in `jess --version`, and the harness refuses it: its numbers would be slower than what ships, by no fixed amount.

Your OpenMW config is only read. `tools/bench/flatten.luau` loads its chain and writes one standalone `openmw.cfg` with absolute data folders and the content list, minus the three tools' own outputs, so neither side merges last run's result. Each run then starts from an empty folder under `target/bench/showcase/<workload>/run` with that config, a `data-local=` inside it, and a home folder of its own, so TES3Merge finds its `TES3Merge.ini` there and no tool finds or touches yours. jess and the native tool run in the same folder, one after the other, so paths they print match too. The jess side is the way a player runs the tools: a driver, `bench/run.luau` in the run folder, that loads the run's `openmw.cfg`, sets the workload's options in `state`, and calls the tool, under `jess run --package jpk/dreamwind`. The timings include activating the package from its source folder.

The harness measures right away and records the one-minute load average and the busiest processes at the start and end, so every result says what it ran under. `--max-load 1.0` makes it wait (up to `--patience` minutes, 30 by default) for the load to drop below that first. Then it runs each side once to warm the page cache and `--runs` times (5) alternating, so a slow drift can't favor either one. After the measured runs, one more jess run, with `tools/bench/facts.luau` called after the tool, lists what it reports (LAND records read, lights changed, and so on) for the pages to quote.

Delete `target/release` and `target/bench/src` when you're done; they're big.

### Output equivalence

A port that's fast and wrong is a bug with good marketing. After the runs, the harness compares every file each side wrote and gives each one a verdict:

- `identical`: byte for byte.
- `identical-except-timestamp`: byte for byte once the time of the run in the plugin header's description is blanked, the only thing a merge can't reproduce. Nothing else is normalized.
- `known-differences`: after the rules the workload declares, the records the native tool alone writes are exactly the listed ones, and everything else matches byte for byte, record order and masters included. Only TES3Merge declares any, each with its reason: it merges the garbage in AIDT padding (`aidt-padding` zeroes it on both sides, and four creature records that change nothing are native's alone), writes magic effect subrecords in its own order (`mgef-subrecord-order` puts them back in the Construction Set's, and three effects that are the winner reshuffled are native's alone), and keeps a reference count in cells whose references it drops (`cell-nam0` drops a `NAM0` from a cell without references). The [TES3Merge](@/docs/bundled-scripts/tes3merge.md#differences-from-native-tes3merge) page explains each.
- `mismatch`: anything else, with the records and subrecords that differ. `showcase` exits with 1.

The cargo tests keep the goldens from the original tools (`cargo test -p jess-cli --test cli s3lightfixes::`, `--test cli mergedlands::`). The harness doesn't replace them and never rewrites them.

## Canaries

```sh
python3 tools/bench/bench.py canaries
```

Small synthetic workloads for regressions in the Luau runtime and jess's bindings, the things a new binding layer changes. `tools/bench/canaries/setup.luau` writes their plugins first, so they need no game data. Each prints how long its hot loop took, which is the number the harness compares. `--max-load` works the same way as for the showcase, and `--scale` (10) sets how much synthetic data they chew through.

| Canary | Measures |
| --- | --- |
| `safeenv` | A `math` and `buffer` loop, against the same loop in an environment `setfenv` made unsafe. Fails unless it's at least 5 times faster. |
| `math` | `math.*` in a tight loop. |
| `buffer` | `buffer.*` reads and writes. |
| `vector` | Native vectors packed into a buffer and read back. |
| `select` | Iterating a selection across eight plugins. |
| `view` | Reading fields through record views. |
| `openall` | `jess.plugin.openAll` keeping one record type. |
| `land` | `jess.land.read` and `write` around a walk over every vertex. |
| `method` | `plugin:find` and `jess.meta` in a hot loop. |

`safeenv` is the one that can't pass by accident. Luau calls builtins on its fast path only in an environment marked safe; when jess stopped marking it, Merged Lands went from 3.4 s to 41 s and nothing failed. The ratio doesn't depend on the machine, so `cargo test -p jess-cli --test cli canaries::` runs it in the test suite, along with every other canary at its smallest to keep them from rotting.

The others' times do depend on the machine. `--check <results>` fails when a canary is more than `--tolerance` (1.5) times slower than in an earlier results file from the same machine.

## Results

Each run writes one JSON file to `target/bench/results/<time>-<kind>.json`:

| Key | What |
| --- | --- |
| `kind` | `showcase` or `canaries`. |
| `jess` | `commit`, `dirty`, the binary's path, SHA-256, and `profile`, `runtime` (the Luau binding, from `Cargo.lock`, or `--runtime`), and `luau` (the Luau the binary runs, read from the first line of `jess luau-types`). |
| `machine` | `os`, `kernel`, `cpu`, `threads`, `memory_bytes`, and the CPU `governor`. |
| `load_start`, `load_end`, `busiest_at_start` | The load averages, and what was busy. |
| `runs`, `warmup`, `scale` | How many, and how big the canaries were. |
| `load_order` | The showcase's config, its SHA-256, and its counts: content files, plugins, their bytes, data folders. |
| `natives` | Each native binary's name and SHA-256. |
| `workloads` | Per workload: `load_start`, `facts`, `equivalence`, and `implementations`, where `jess` and `native` each have their `command`, `version`, every sample (`wall`, `cpu`, `user`, `system`, `rss` in bytes, and `inner` for a canary), the median, minimum, and maximum of each, and every output's size, SHA-256, timestamp-blanked SHA-256, and record counts. A canary also has its script's `script_sha256`, so `compare` notices when one changed. |

The baselines the site's numbers come from are committed in `tools/bench/results`, named for the runtime and CPU they ran on. Anything else stays in `target/`.

## Comparing runs

```sh
python3 tools/bench/bench.py compare tools/bench/results/<baseline>.json target/bench/results/<run>.json
```

Prints both runs' commit, runtime, Luau version, CPU, and load, warns when the machine, load order, or canary scale differ, and puts each workload's median time and peak memory side by side with the ratio. It's how a new binding layer gets judged: same scripts, same inputs, same flags, same machine, and a `runtime` that says which is which.

## The site's numbers

```sh
python3 tools/bench/bench.py docs tools/bench/results/<baseline>.json
python3 tools/bench/bench.py graph tools/bench/results/<baseline>.json
```

`docs` writes `site/data/benchmarks.toml`: the run's context, and per workload the times, memory, ratios, and facts, already formatted. It only takes a release build of a clean checkout. Pages print the values with the `bench` shortcode, `{{/* bench(workload="tes3merge", value="faster") */}}` or `{{/* bench(value="plugins") */}}`, and the site build fails on a name the file doesn't have. To update the numbers, commit the new baseline and the data file it made; don't touch the pages.

`graph` draws wall time and peak memory for every workload with a native original to `target/bench/showcase.png` (and the SVG beside it) with ImageMagick.
