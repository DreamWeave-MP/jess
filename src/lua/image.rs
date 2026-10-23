//! Packed workspaces as the host finds them: a `.jsw` file.
//!
//! What a packed workspace is, and how one opens, is Luau (`stdlib/jsw.jsl`); the host only says
//! where its bytes are: a byte range of a file, which the Luau reads through the host's file
//! reads, in place.

use std::path::Path;

/// Where a packed workspace's bytes are: `length` bytes from `offset` of the file at `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// The file: a `.jsw`.
    pub path: String,
    /// Where its bytes start.
    pub offset: u64,
    /// How many there are.
    pub length: u64,
}

impl Image {
    /// The packed workspace in the file at `path`, the whole file, which `stdlib/jsw.jsl` then
    /// reads as a `.jsw`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be read.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let length = std::fs::metadata(path)?.len();
        Ok(Self {
            path: text(path)?,
            offset: 0,
            length,
        })
    }

    /// This image as the table the bootstrap reads: `{ path, offset, length }`.
    pub(crate) fn table(&self, scope: &impl l3i::stack::Scope) -> l3i::Result<l3i::value::Table> {
        let table = l3i::value::Table::new(scope, 0, 3)?;
        table.set(scope, "path", &self.path)?;
        let number = |value: u64| {
            i64::try_from(value).map_err(|_| crate::lua::error(format!("'{}' is too large to read", self.path)))
        };
        table.set(scope, "offset", &number(self.offset)?)?;
        table.set(scope, "length", &number(self.length)?)?;
        Ok(table)
    }
}

fn text(path: &Path) -> std::io::Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("'{}' has a name that isn't UTF-8", path.display()),
        )
    })
}
