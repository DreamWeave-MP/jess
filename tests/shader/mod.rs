//! The `jess::shader` integration tests: one module per file, one binary for all of them, so a change
//! links one test binary instead of one per file.

mod bindings;
mod builder;
mod builtins;
mod common;
mod compute;
mod conversions;
mod folding;
mod functions;
mod integers;
mod layout;
mod logic;
mod operators;
mod statements;
mod storage;
mod textures;
mod triangle;
mod validation;
mod wgsl;
