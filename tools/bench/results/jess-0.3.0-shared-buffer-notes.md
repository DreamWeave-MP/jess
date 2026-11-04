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
