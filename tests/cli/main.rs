//! The jess CLI integration tests: one module per file, one binary for all of them, so a change
//! links one test binary instead of one per file.

mod fast_path;
mod cli;
mod common;
mod init;
mod luau_test;
