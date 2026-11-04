# Shared buffer investigation

Run on an Intel Core i7-10870H with a verified Jess 0.3.0 release, built with two cargo jobs.
`shared-buffer-read` and `ordinary-buffer-read` each read 40 million f64 values after two warmups;
their JSON files contain five samples and the source hashes. The untagged shared median was
1.808 s against 0.174 s for an ordinary buffer. A second run with preferred userdata tags gave
1.835 s for shared reads; ordinary reads were noisy at 0.527 s median. Preferred tags did not
show an improvement in the shared median. L3i's native lowering for its byte helpers does not
directly lower an f64 read from external storage. A safe lowering needs its own bounds and lifetime
proof; these measurements do not establish that this implementation is suitable for a numerical
kernel.

The worker-transfer fixture measures sending an already constructed dataset and reading its first
f64 in each worker, not a complete calculation over it. Each combination has a first job followed
by two repeated jobs; worker startup and dataset construction are outside the measured interval.
Run it with `jess package run --capability shared-buffer.create --capability worker.spawn tools/bench/worker-transfer`. The two raw
logs were obtained in a user scope with `MemoryMax=7G` and `MemorySwapMax=2G`. The first ran copied
before shared; the second reversed that order. Both scopes were terminated during the repeated
480 MB, eight-worker copied case; neither produced a peak-memory report. The second log includes
all three 480 MB shared, eight-worker jobs. Allocation counts and per-case memory consumption were
not measured. No claim about end-to-end numerical throughput follows from these transfer timings.

## Native reads and the record sweep

`jess-0.3.0-shared-buffer-native.json` is the two read canaries on a verified build of `a904391`,
after `readf64` and `readu32` were lowered to native loads: 0.187 s for shared reads against 0.157 s
for ordinary ones, five samples after two warmups, where the method took 1.808 s against 0.174 s.

The `record-sweep` files are one process per row, from `tools/bench/record-sweep`: mode, records,
workers, sweeps, window, the one-minute load when the row started, peak RSS in KiB with user and
system seconds, and the fixture's own line. Every row's total equalled a reference computed outside
the workers. `-native` is the same `a904391` build, at a load of 3 to 7. `-method` is `7227325`,
before the lowering, at a load of 6 to 14. The machine was shared throughout, so small differences
mean nothing. The per-record work is deliberately small, so these compare ways of reading and say
nothing about end-to-end throughput.
