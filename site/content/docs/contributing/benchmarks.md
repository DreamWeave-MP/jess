---
title: 'Benchmarks'
description: 'The harnesses behind every performance number on this site: the showcase against the native tools, the site generator against Zola, the runtime canaries, and the benches each package carries.'
weight: 35
extra:
  kind: guide
---

No performance number on this site is typed in. Each comes out of a script that checks its answer before it posts a time, and the page that prints it names the script. "Feels faster" isn't a unit.

There are four kinds:

| Kind | Script | Measures |
| --- | --- | --- |
| The showcase | `tools/bench/showcase.luau` | The Luau ports of the load-order tools, as whole programs over your real load order, next to the native tools they port, and whether both wrote the same files. |
| The site | `tools/bench/site.luau` | The site generator building this site, next to Zola building it from the same sources. |
| The canaries | `tools/bench/bench.luau` | Small synthetic workloads for regressions in the runtime and jess's bindings. |
| Package benches | `jpk/<package>/bench/bench.luau` | Each package's own hot paths through its public API, recorded in its `BENCHMARKS.md`. |

All of them are Luau and run with `jess run`. All of them refuse a jess built without the verified toolchain: `jess --version` says when a build is unverified, its Luau runs slower than what ships by no fixed amount, and a number from it says nothing.

## The showcase

```sh
jess run --capability filesystem.read --capability filesystem.write \
  --capability process.spawn --capability process.environment \
  tools/bench/showcase.luau run --merged-lands ~/src/OpenMWMergedLands/target/release/merged_lands
```

It measures `--jess` (default: the `jess` on `PATH`) over the load order of your `openmw.cfg`, only ever read:

| Workload | jess | Native |
| --- | --- | --- |
| `tes3merge` | `jpk/tes3merge` | TES3Merge (`tes3merge` on `PATH`, or `--tes3merge`) |
| `s3lightfixes` | `jpk/s3lightfixes` | S3LightFixes (`s3lightfixes` on `PATH`, or `--s3lightfixes`) |
| `s3lightfixes-sun` | the same, `disable_interior_sun = true` | the same |
| `mergedlands` | `jpk/merged-lands` | Merged Lands, which has no release binary for Linux: build it with `cargo build --release` from its repository and name it with `--merged-lands` |
| `doctor` | `jpk/doctor` | none exists |
| `doctor-assets` | `jpk/doctor`, `--checks assets,meshes` | none |
| `doctor-rest` | `jpk/doctor`, the other four checks | none |

`--only tes3merge --only mergedlands` runs some, `--no-native` skips the originals, `--openmw-cfg` picks the config to flatten, and `--packages` names the folder the ports' packages are found in (default: the repository's `jpk`).

`flatten.luau` loads the config chain and writes one standalone `openmw.cfg` with absolute data folders and the content list, minus the three tools' own outputs, so neither side merges last run's result, and with each content file spelled as it is on disk, since native Merged Lands resolves names case-sensitively on Linux and your config may not. Each run then starts from an empty folder under `target/bench/showcase/<workload>/run` with that config, a `data-local=` inside it, and the workload's seed files (`TES3Merge.ini`, a `lightconfig.toml`), with a home folder of its own under `target/bench/home` and every `JESS_*` and `OPENMW_*` variable removed, so no tool finds or touches yours. jess and the native tool run in the same folder, one after the other.

The jess side is the way a person runs the tool: `jess package run --trusted --package-path jpk jpk/<package> -- <arguments>`, so the timings include activating the package from its source folder, which on a `jpk` holding fifty packages is about a third of a second before the first record is read. With `--packed`, each tool's package and every package its activation plan selects are packed once into `target/bench/jpk` with `jess pack`, and the jess side runs from those, with nothing else on the package path, as an install looks. The results then carry each JPK's digest and the file is named `-showcase-packed.json`.

Each side runs once to warm the page cache and `--runs` (5) times more, alternating, so a slow drift can't favor either one. GNU time (`/usr/bin/time`) measures each process from outside: wall seconds, user and system CPU, and peak resident memory. `--max-load` (1.5) waits up to `--patience` minutes (10) for the one-minute load average to drop below it before measuring; the load at the start and the end is recorded either way, along with the machine and the binaries' SHA-256s. After the measured runs, one more jess run calls the tool through its module with `facts.luau` and records what it reports about the load order (LAND records read, lights changed, and so on) for the pages to quote.

### Output equivalence

A port that's fast and wrong is a bug with good marketing. After the runs, `equivalence.luau` compares every file each side wrote and gives each one a verdict:

- `identical`: byte for byte.
- `identical-except-timestamp`: byte for byte once the time of the run in the plugin header's description is blanked, the only thing a merge can't reproduce. Nothing else is normalized.
- `known-differences`: after the rules the workload declares, the records the native tool alone writes are exactly the listed ones, and everything else matches byte for byte, record order and masters included. Only TES3Merge declares any, each with its reason: it merges the garbage in AIDT padding (`aidt-padding` zeroes it on both sides, and four creature records that change nothing are native's alone), writes magic effect subrecords in its own order (`mgef-subrecord-order` puts them back in the Construction Set's, and three effects that are the winner reshuffled are native's alone), and keeps a reference count in cells whose references it drops (`cell-nam0` drops a `NAM0` from a cell without references). The [TES3Merge](@/docs/bundled-scripts/tes3merge.md#differences-from-native-tes3merge) page explains each.
- `mismatch`: anything else, with the records that differ and which side alone has them. `run` prints it and exits with 1 after writing its results.

A mismatch is reported, not excused: the current native Merged Lands writes LAND records the port leaves out, and the [Merged Lands](@/docs/bundled-scripts/mergedlands.md#performance) page says so rather than the harness growing a rule.

`--against RESULTS` also compares every output this run kept with the same output in an earlier results file, by SHA-256, which is how a packed run is checked against a source run.

## The site

```sh
jess run --capability filesystem.read --capability filesystem.write \
  --capability process.spawn --capability process.environment \
  tools/bench/site.luau [runs] [zola binary]
```

Each side builds `site/` into a folder of its own under `target/bench/site`, from a cold process: the jess side through the package target in `tools/site`, the Zola side with `zola build`. One warm-up round, then `runs` (5) measured rounds alternating the two, timed by GNU time the same way; the medians, the ratio, and both outputs' file counts are printed. The landing page quotes it.

## Canaries

```sh
jess run --capability filesystem.read --capability filesystem.write tools/bench/bench.luau canaries
```

Small synthetic workloads for regressions in the Luau runtime and jess's bindings, the things a new binding layer changes. They need no game data. Each prints how long its hot loop took, which is the number the harness compares.

| Canary | Measures |
| --- | --- |
| `safeenv` | A `math` and `buffer` loop, against the same loop in an environment `setfenv` made unsafe. Fails unless it's at least 5 times faster. |
| `math` | `math.*` in a tight loop. |
| `buffer` | `buffer.*` reads and writes. |
| `vector` | Native vectors packed into a buffer and read back. |

`safeenv` is the one that can't pass by accident. Luau calls builtins on its fast path only in an environment marked safe; when jess stopped marking it, Merged Lands went from 3.4 s to 41 s and nothing failed. The ratio doesn't depend on the machine. The others' times do; `compare <baseline> <results>` puts two runs from the same machine side by side.

## Package benches

Every first-party package carries `bench/bench.luau`, run as `jess run --package jpk/<package> --package-path jpk jpk/<package>/bench/bench.luau`, and a `BENCHMARKS.md` that records what it measured, on which binary and machine, under what protocol, usually as a table of base against head for the change that was being judged. Each workload is warmed up, sized to a batch of about a tenth of a second, and reported as the median of seven batches with a full collection between them, and every workload checks its result against a pinned answer before its first batch and after its last. The landing page's in-package numbers are those tables' current heads.

## Results

A showcase run writes one JSON file to `target/bench/results/<time>-showcase.json`: the jess binary's version, path, SHA-256, runtime, and Luau version; the machine; the load averages; the flattened load order's counts and digest; each native's version and SHA-256; and per workload the facts, the equivalence verdicts, and each side's command, every sample (wall, CPU, peak memory), their medians, and every output's size and digests. The baselines the site's numbers come from are committed in `tools/bench/results`, named for the runtime and CPU they ran on. Anything else stays in `target/`.

## The site's numbers

```sh
jess run --capability filesystem.read --capability filesystem.write \
  --capability process.spawn --capability process.environment \
  tools/bench/showcase.luau docs tools/bench/results/<baseline>.json
```

`docs` writes `site/data/benchmarks.toml`: the run's context, and per workload the times, memory, ratios, and facts, already formatted. Pages print the values with the `bench` shortcode, `{{/* bench(workload="tes3merge", value="faster") */}}` or `{{/* bench(value="plugins") */}}`, and the site build fails on a name the file doesn't have. `faster` and `less_memory` are native divided by jess, so a value under 1 means the native won that one. To update the numbers, commit the new baseline and the data file it made; don't touch the pages. The landing page's other numbers live in `site/data/showcase.toml`, each beside the bench it came from.
