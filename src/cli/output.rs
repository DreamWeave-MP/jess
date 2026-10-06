//! Structured output: every command prints one YAML, JSON, or TOML document to stdout.
//!
use std::io::{self, BufWriter, StdoutLock, Write};

use clap::ValueEnum;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Format {
    Yaml,
    Json,
    Toml,
}

fn write_error(error: &impl std::fmt::Display) -> String {
    format!("failed to write output: {error}")
}

/// Stdout that ends the process quietly once its reader goes away.
///
/// Unix gets this from SIGPIPE (`main` restores its default action); Windows has no SIGPIPE and
/// reports a closed pipe as a write error instead, which would otherwise print as a failure.
struct Stdout(StdoutLock<'static>);

impl Stdout {
    fn new() -> BufWriter<Self> {
        BufWriter::new(Self(io::stdout().lock()))
    }
}

fn end_on_broken_pipe<T>(result: io::Result<T>) -> io::Result<T> {
    if matches!(&result, Err(error) if error.kind() == io::ErrorKind::BrokenPipe) {
        std::process::exit(1);
    }
    result
}

impl Write for Stdout {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        end_on_broken_pipe(self.0.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        end_on_broken_pipe(self.0.flush())
    }
}

/// Prints `value` as one document.
///
/// # Errors
///
/// Returns an error if `value` can't be serialized in `format` or stdout can't be written.
pub fn print(value: &impl Serialize, format: Format) -> Result<(), String> {
    let mut out = Stdout::new();
    match format {
        Format::Yaml => serde_yaml::to_writer(&mut out, value).map_err(|error| write_error(&error))?,
        Format::Json => {
            serde_json::to_writer(&mut out, value).map_err(|error| write_error(&error))?;
            out.write_all(b"\n").map_err(|error| write_error(&error))?;
        }
        Format::Toml => {
            let text = toml::to_string(value).map_err(|error| write_error(&error))?;
            out.write_all(text.as_bytes()).map_err(|error| write_error(&error))?;
        }
    }
    out.flush().map_err(|error| write_error(&error))
}

/// A file a command wrote, and the backup it kept of the original.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Written {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
}
