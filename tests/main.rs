//! Every integration test, in one binary: the verified toolchain links each test binary with a
//! whole-program LTO over jess and l3i, so one link is the difference between a suite that builds
//! in minutes and one that builds in an hour. Run one area with a filter:
//! `cargo test --test integration cli::`, `... lua::package::`, `... lua::luau_style::`.

mod cli;
#[cfg(feature = "gpu")]
mod gpu;
mod host;
mod input;
mod lua;
mod render;
mod runtime;
mod shader;
