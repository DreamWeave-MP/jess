//! The `jess::shader` integration tests: one module per file, one binary for all of them, so a change
//! links one test binary instead of one per file.

mod builder;
mod common;
mod folding;
mod integers;
mod layout;
mod triangle;
mod validation;
mod wgsl;
