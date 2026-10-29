//! The jess CLI integration tests: one module per file, one binary for all of them, so a change
//! links one test binary instead of one per file.

mod application;
mod capabilities;
mod check;
mod common;
mod docs;
mod fast_path;
mod init;
mod install;
mod jsw;
mod luau_test;
mod toplevel;
mod workspace;
