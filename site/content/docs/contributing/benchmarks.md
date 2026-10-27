---
title: 'Benchmarks'
description: 'The harness under tools/bench: why it refuses an unverified build, how a bench checks its answer before it posts a time, and how two runs are compared.'
weight: 35
extra:
  kind: guide
---

"Feels faster" isn't a unit. A performance claim about jess is a number from a script that checked its answer before it posted a time, on a build that is the one people get. Everything in `tools/bench` exists to make that the only kind of number anyone can produce.

## The rule

A bench runs on a verified release build or not at all. The development build (`tools/dev/no-lto.sh`) links without cross-language LTO, so Luau runs slower in it by an amount that varies with the workload, and a number from it says nothing about what ships. `jess --version` marks such a binary (`unverified toolchain, no cross-language LTO: not for measuring`), `jess.bench.verified` is `false` in it, and every harness checks that first:

```luau
--!strict
if not jess.bench.verified then
  error('bench: this jess was built without the verified toolchain: measure on a release', 0)
end
```

The build to measure is `cargo build --release` in a shell where the development script was never sourced, built one at a time with two jobs. Measure on an idle machine, and never with a build running beside it.

## The canaries

```sh
jess run --capability filesystem.read --capability filesystem.write tools/bench/bench.jsl canaries
```

`tools/bench/bench.jsl` is the native kernel's own harness: small synthetic workloads for regressions in the runtime and jess's bindings, the things a new binding layer changes. They need no data and no packages. Each prints how long its hot loop took, which is the number the harness compares.

| Canary | Measures |
| --- | --- |
| `safeenv` | A `math` and `buffer` loop, against the same loop in an environment `setfenv` made unsafe. Fails unless it's at least 5 times faster. |
| `math` | `math.*` in a tight loop. |
| `buffer` | `buffer.*` reads and writes. |
| `vector` | Native vectors packed into a buffer and read back. |
| `audio` | A crowded scene rendered offline through `jess.audio`, a tick at a time: 600 positioned loops for 96 mixed voices, six one-shots a tick, a moving listener, a ducked music bus, a reverb send, and a limiter. Measures the script, the binding, and the engine together, and fails if anything came out late or not finite, or nothing was culled. |
| `audio-churn` | Voice churn: 64 short one-shots started a tick on their own frames, a quarter of the last tick's stopped early with a fade, 384 on one frame every half second, every fourth pitched and half sent to a reverb, into a context mixing 128 of the 4096 it holds. Nothing is routed, so it's the cost of starting, mixing, and ending voices. Fails if anything came out late or not finite, nothing was stolen, or slots weren't given back. |
| `audio-render` | The same scene as `audio-churn`, timing only the renders: the engine apart from the script and the binding. |

`safeenv` is the one that can't pass by accident. Luau calls builtins on its fast path only in an environment marked safe; when jess once stopped marking it, every workload got an order of magnitude slower and nothing failed. The ratio doesn't depend on the machine. The others' times do, so a canary result is only ever compared with another from the same machine.

`bench.jsl list` names the canaries; `bench.jsl compare <baseline> <results>` puts two result files from the same machine side by side.

## What a bench owes

Whatever it measures, a bench in this repository follows the same protocol, and a package's bench is expected to as well:

- **It checks its answer.** Every workload compares its result with a pinned answer before its first timed batch and after its last. A path that stops doing the work fails instead of posting a time. A port that's fast and wrong is a bug with good marketing.
- **It warms up, then sizes.** A workload is warmed, then sized to a batch of about a tenth of a second, and reported as the median of several batches with a full collection (`jess.bench.collect()`) between them, so one batch's garbage isn't collected inside the next one's timing.
- **It names its build.** A results file records the jess version, the binary's SHA-256, the runtime and Luau versions, and the machine. A number without those attached is a rumor.
- **It stays out of the tree.** Results land under `target/bench`; the baselines worth keeping are committed under `tools/bench/results`, named for the runtime and CPU they ran on.

## Whole programs

`tools/bench/site.jsl` is the one whole-program measurement the repository keeps: it builds this site through the `tools/site` command target from a cold process, several rounds, timed from outside by GNU time (wall, CPU, peak memory). It's the model for timing a command target the way a person runs it: starting jess, resolving and activating the package, and loading its modules are part of the cost, and the harness says so rather than hiding them.

A package's own hot paths belong with the package, in its `bench/bench.jsl`, under the same protocol, and the decisions they drive (which modules earn `--!native`, say) are recorded beside the code they justify. Nothing on this site quotes them.
