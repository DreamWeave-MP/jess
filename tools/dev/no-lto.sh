# Development builds without cross-language LTO. Source it in the shell you build jess from:
#
#   . tools/dev/no-lto.sh
#
# The repo's .cargo/config.toml links every binary, tests included, under l3i's verified
# toolchain: rustc emits LLVM bitcode and lld generates the code at each link, on every core.
# That is what releases ship and what every performance number is measured on. For an edit and
# test loop it is most of the time and nearly all of the CPU.
#
# RUSTFLAGS replaces the config's flags (`--config` and CARGO_TARGET_<triple>_RUSTFLAGS only
# append to them), so clang and lld still link but nothing is LTO'd; l3i's build script then
# warns instead of refusing. Builds go to target/fast, so the verified build in target/ survives
# switching back.
#
# Luau runs slower in these builds. Never measure one: `jess --version` says "unverified
# toolchain". Don't feed numbers from that binary into tools/bench/bench.luau, and don't
# source this from your shell profile either: it would reach every other cargo project, l3i's
# own benchmarks included.

export RUSTFLAGS='-Clinker=clang -Clink-arg=-fuse-ld=lld'
export L3I_UNVERIFIED_TOOLCHAIN=1
CARGO_TARGET_DIR="$(git rev-parse --show-toplevel)/target/fast"
export CARGO_TARGET_DIR
