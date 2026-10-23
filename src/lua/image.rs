//! Packed workspaces as the host finds them: a `.jsw` file, or the one a standalone executable
//! carries.
//!
//! What a packed workspace is, and how one opens, is Luau (`stdlib/jsw.jsl`); the host only says
//! where its bytes are. That is a byte range of a file either way, so a `.jsw` run by `jess` and the
//! same bytes inside an executable are read by the same code from the same offsets on.
//!
//! An executable carries one as a payload followed by a [`TRAILER_LEN`]-byte trailer: the
//! payload's offset and length, little-endian `u64`s, then [`MAGIC`]. On a Windows executable
//! signed after the payload was added, the trailer ends where the signature's certificate table
//! begins, since signing appends that table. A Mach-O executable is refused: code signing on macOS
//! seals the file, and data after it is outside the signature.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// The last bytes of the trailer that marks an executable carrying a packed workspace.
pub const MAGIC: &[u8; 16] = b"jess-packed-wksp";

/// The trailer's length: the payload's offset and length, then [`MAGIC`].
pub const TRAILER_LEN: usize = 32;

/// Where a packed workspace's bytes are: `length` bytes from `offset` of the file at `path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// The file: a `.jsw`, or an executable carrying one.
    pub path: String,
    /// Where its bytes start.
    pub offset: u64,
    /// How many there are.
    pub length: u64,
}

impl Image {
    /// The packed workspace in the file at `path`: the payload of an executable carrying one, or
    /// else the whole file, which `stdlib/jsw.jsl` then reads as a `.jsw`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be read, or its trailer names bytes it doesn't have.
    pub fn open(path: &Path) -> std::io::Result<Self> {
        if let Some(image) = Self::carried(path)? {
            return Ok(image);
        }
        let length = std::fs::metadata(path)?.len();
        Ok(Self {
            path: text(path)?,
            offset: 0,
            length,
        })
    }

    /// The packed workspace this executable carries, if it carries one: what makes it a
    /// standalone application rather than jess. An executable it can't read carries none.
    ///
    /// # Errors
    ///
    /// Returns an error if it carries one that can't be trusted: a trailer naming bytes it
    /// doesn't have, or bytes after its signature.
    pub fn embedded() -> Result<Option<Self>, String> {
        let Ok(executable) = std::env::current_exe() else {
            return Ok(None);
        };
        match Self::carried(&executable) {
            Ok(image) => Ok(image),
            Err(problem) if problem.kind() == std::io::ErrorKind::InvalidData => Err(problem.to_string()),
            Err(_) => Ok(None),
        }
    }

    /// The payload the file at `path` carries after a trailer, if it has one.
    ///
    /// # Errors
    ///
    /// Returns an error if the file can't be read, its trailer names bytes it doesn't have, or it
    /// is a signed executable with bytes after its signature, which nothing vouches for.
    pub fn carried(path: &Path) -> std::io::Result<Option<Self>> {
        let mut file = File::open(path)?;
        let (end, signed) = content_end(&mut file)?;
        if signed == Some(false) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "'{}' is signed, and has bytes after its signature, which nothing vouches for; it was changed after it was signed",
                    path.display()
                ),
            ));
        }
        let signed = signed == Some(true);
        // Signing pads the file with zeros to the 8-byte boundary its certificate table starts on.
        let slack = if signed { 7 } else { 0 };
        let window_len = (TRAILER_LEN as u64 + slack).min(end);
        if window_len < TRAILER_LEN as u64 {
            return Ok(None);
        }
        let window_start = end - window_len;
        let mut window = vec![0; usize::try_from(window_len).unwrap_or(TRAILER_LEN)];
        file.seek(SeekFrom::Start(window_start))?;
        file.read_exact(&mut window)?;
        let ends_trailer = |at: &usize| {
            &window[at + 16..at + TRAILER_LEN] == MAGIC && window[at + TRAILER_LEN..].iter().all(|byte| *byte == 0)
        };
        let Some(at) = (0..=window.len() - TRAILER_LEN).rev().find(ends_trailer) else {
            return Ok(None);
        };
        let start = window_start + at as u64;
        let trailer = &window[at..at + TRAILER_LEN];
        let offset = u64::from_le_bytes(trailer[..8].try_into().unwrap_or_default());
        let length = u64::from_le_bytes(trailer[8..16].try_into().unwrap_or_default());
        if offset.checked_add(length).is_none_or(|payload_end| payload_end > start) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "'{}' has a packed workspace trailer naming {length} bytes at {offset}, past its payload",
                    path.display()
                ),
            ));
        }
        Ok(Some(Self {
            path: text(path)?,
            offset,
            length,
        }))
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

/// `runtime`, a jess executable, carrying `payload`, a packed workspace: the executable's bytes
/// unchanged, then the payload, then the trailer. Sign the result afterwards, if at all.
///
/// # Errors
///
/// As [`check_runtime`].
pub fn standalone(runtime: &[u8], payload: &[u8]) -> Result<Vec<u8>, String> {
    check_runtime(runtime)?;
    let offset = runtime.len() as u64;
    let mut bytes = Vec::with_capacity(runtime.len() + payload.len() + TRAILER_LEN);
    bytes.extend_from_slice(runtime);
    bytes.extend_from_slice(payload);
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    bytes.extend_from_slice(MAGIC);
    Ok(bytes)
}

/// Whether `runtime`, a jess executable, can carry a packed workspace.
///
/// # Errors
///
/// Returns an error naming why it can't: it is a Mach-O executable, it is already signed (signing
/// comes after), it already carries a packed workspace, or it predates them.
pub fn check_runtime(runtime: &[u8]) -> Result<(), String> {
    if is_mach_o(runtime) {
        return Err("the runtime is a macOS executable; a standalone macOS application needs the packed workspace inside a signed Mach-O segment, which this jess doesn't write yet: ship the .jsw beside jess instead".to_owned());
    }
    if certificate_table_in(runtime).is_some() {
        return Err(
            "the runtime is already signed; add the packed workspace to an unsigned runtime, then sign the result"
                .to_owned(),
        );
    }
    if runtime.len() >= TRAILER_LEN && &runtime[runtime.len() - 16..] == MAGIC {
        return Err("the runtime already carries a packed workspace; start from jess itself".to_owned());
    }
    // A jess that looks for a trailer holds its magic; one that doesn't would run as jess, its
    // payload ignored.
    if !runtime.windows(MAGIC.len()).any(|window| window == MAGIC) {
        return Err(
            "the runtime predates standalone executables, and would ignore what it carried; use a jess as new as this one"
                .to_owned(),
        );
    }
    Ok(())
}

fn text(path: &Path) -> std::io::Result<String> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("'{}' has a name that isn't UTF-8", path.display()),
        )
    })
}

fn is_mach_o(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0xcf | 0xce, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe] | [0xfe, 0xed, 0xfa, 0xce | 0xcf])
    )
}

/// Where the content of the file ends, and whether it is signed: before the certificate table of
/// a signed PE file, else at the end. A signature is `Some(true)` when its table runs to the end,
/// as signing leaves it, and `Some(false)` when bytes follow it.
fn content_end(file: &mut File) -> std::io::Result<(u64, Option<bool>)> {
    let length = file.seek(SeekFrom::End(0))?;
    // The headers that locate the certificate table are within the first few hundred bytes.
    let head_len = usize::try_from(length.min(4096)).unwrap_or(4096);
    let mut head = vec![0; head_len];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut head)?;
    Ok(match certificate_table_in(&head) {
        Some((offset, size)) if offset.checked_add(size) == Some(length) => (offset, Some(true)),
        Some(_) => (length, Some(false)),
        None => (length, None),
    })
}

/// The certificate table the PE headers in `head` name, as a file offset and a size. Its data
/// directory entry is the fifth: a file offset, unlike every other entry's address.
fn certificate_table_in(head: &[u8]) -> Option<(u64, u64)> {
    let read_u16 = |at: usize| head.get(at..at + 2).map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]));
    let read_u32 = |at: usize| {
        head.get(at..at + 4)
            .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    if head.get(..2) != Some(b"MZ") {
        return None;
    }
    let pe = usize::try_from(read_u32(0x3c)?).ok()?;
    if head.get(pe..pe + 4) != Some(b"PE\0\0") {
        return None;
    }
    let optional = pe + 24;
    let directories = match read_u16(optional)? {
        0x10b => optional + 96,
        0x20b => optional + 112,
        _ => return None,
    };
    let offset = read_u32(directories + 4 * 8)?;
    let size = read_u32(directories + 4 * 8 + 4)?;
    (offset != 0 && size != 0).then_some((u64::from(offset), u64::from(size)))
}
