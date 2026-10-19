//! Fixture builders shared by the CLI integration tests.

pub mod http;

pub use std::fs;
pub use std::path::{Path, PathBuf};
pub use std::process::Command;

/// The jess binary.
pub fn jess() -> Command {
    Command::new(env!("CARGO_BIN_EXE_jess"))
}

pub fn temp_dir_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("jess-cli-{name}-dir-{}", std::process::id()))
}
