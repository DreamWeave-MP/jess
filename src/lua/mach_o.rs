//! A packed workspace inside a macOS jess: the `__JESS,__jsw` section a macOS jess reserves when it
//! is linked, grown to hold one.
//!
//! Code signing on macOS seals a file, and bytes appended after the seal can't be signed again, so
//! the payload goes where Apple says data belongs: in a section of its own (TN2206). jess reserves
//! the section, holding [`super::image::MAGIC`], between `__DATA` and `__LINKEDIT`. Packing writes
//! the payload there, moves `__LINKEDIT` and every offset into it along, and signs the result ad
//! hoc, as the linker signed the runtime, so it runs as it is and `codesign` can sign it again.
//!
//! Only thin 64-bit little-endian executables are read: what jess is built as on macOS.

use sha2::{Digest, Sha256};

use super::image::MAGIC;

const MH_MAGIC_64: u32 = 0xfeed_facf;
const HEADER_LEN: usize = 32;
const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
const LC_CODE_SIGNATURE: u32 = 0x1d;
const LC_DYLD_INFO: u32 = 0x22;
const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;
const LC_NOTE: u32 = 0x31;
/// The commands that are one `linkedit_data_command`: an offset into `__LINKEDIT` and a size.
const LINKEDIT_DATA: [u32; 9] = [
    LC_CODE_SIGNATURE,
    0x1e,        // LC_SEGMENT_SPLIT_INFO
    0x26,        // LC_FUNCTION_STARTS
    0x29,        // LC_DATA_IN_CODE
    0x2b,        // LC_DYLIB_CODE_SIGN_DRS
    0x2e,        // LC_LINKER_OPTIMIZATION_HINT
    0x36,        // LC_ATOM_INFO
    0x8000_0033, // LC_DYLD_EXPORTS_TRIE
    0x8000_0034, // LC_DYLD_CHAINED_FIXUPS
];

/// The segment and section a macOS jess reserves for a packed workspace.
const SEGMENT: &[u8] = b"__JESS";
const SECTION: &[u8] = b"__jsw";

/// Segments start on 16 KiB boundaries, the arm64 page, which is a multiple of x86-64's.
const SEGMENT_ALIGN: u64 = 0x4000;
/// The code signature hashes 4 KiB pages.
const CODE_PAGE: usize = 4096;
const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade_0cc0;
const CSMAGIC_CODEDIRECTORY: u32 = 0xfade_0c02;
const CODE_DIRECTORY_VERSION: u32 = 0x20400;
/// Ad hoc and linker-signed, as the linker signs: `codesign` replaces it without being asked to.
const CODE_DIRECTORY_FLAGS: u32 = 0x0002_0002;
const CODE_DIRECTORY_HEADER: usize = 88;
const CS_EXECSEG_MAIN_BINARY: u64 = 1;

/// Whether `bytes` start as a Mach-O file of any kind: thin, 32-bit, universal, or big-endian.
pub fn is_mach_o(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([0xcf | 0xce, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe] | [0xfe, 0xed, 0xfa, 0xce | 0xcf])
    )
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    bytes
        .get(at..at + 4)
        .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

fn u64_at(bytes: &[u8], at: usize) -> Option<u64> {
    let word = bytes.get(at..at + 8)?;
    let mut array = [0; 8];
    array.copy_from_slice(word);
    Some(u64::from_le_bytes(array))
}

fn put_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// A 16-byte Mach-O name field as the bytes before its padding.
fn name(field: &[u8]) -> &[u8] {
    let end = field.iter().position(|byte| *byte == 0).unwrap_or(field.len());
    &field[..end]
}

/// One `LC_SEGMENT_64`: where its command is, and where it is in the file and in memory.
#[derive(Clone, Copy, Debug)]
struct Segment {
    command: usize,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
}

/// The load commands of a thin 64-bit Mach-O file: each one's offset and kind, and the segments
/// and sections packing needs.
struct Commands {
    all: Vec<(usize, u32)>,
    reserved: Option<(Segment, usize)>,
    linkedit: Option<Segment>,
    signature: Option<usize>,
}

fn commands(bytes: &[u8]) -> Option<Commands> {
    if u32_at(bytes, 0)? != MH_MAGIC_64 {
        return None;
    }
    let count = usize::try_from(u32_at(bytes, 16)?).ok()?;
    let mut found = Commands {
        all: Vec::with_capacity(count),
        reserved: None,
        linkedit: None,
        signature: None,
    };
    let mut at = HEADER_LEN;
    for _ in 0..count {
        let kind = u32_at(bytes, at)?;
        let size = usize::try_from(u32_at(bytes, at + 4)?).ok()?;
        if size < 8 {
            return None;
        }
        found.all.push((at, kind));
        if kind == LC_SEGMENT_64 {
            let segment = Segment {
                command: at,
                vmaddr: u64_at(bytes, at + 24)?,
                vmsize: u64_at(bytes, at + 32)?,
                fileoff: u64_at(bytes, at + 40)?,
                filesize: u64_at(bytes, at + 48)?,
            };
            let segname = name(bytes.get(at + 8..at + 24)?);
            if segname == b"__LINKEDIT" {
                found.linkedit = Some(segment);
            }
            if segname == SEGMENT {
                let sections = usize::try_from(u32_at(bytes, at + 64)?).ok()?;
                found.reserved = (0..sections)
                    .map(|index| at + 72 + index * 80)
                    .find(|section| bytes.get(*section..*section + 16).is_some_and(|field| name(field) == SECTION))
                    .map(|section| (segment, section));
            }
        }
        if kind == LC_CODE_SIGNATURE {
            found.signature = Some(at);
        }
        at += size;
    }
    Some(found)
}

/// Where the reserved section of the thin 64-bit Mach-O file whose start is `head` is, as a file
/// offset and a size. `head` must hold every load command.
pub fn reserved(head: &[u8]) -> Option<(u64, u64)> {
    let (_, section) = commands(head)?.reserved?;
    let size = u64_at(head, section + 40)?;
    let offset = u64::from(u32_at(head, section + 48)?);
    Some((offset, size))
}

/// `runtime`, a macOS jess, with `payload` in its reserved section, signed ad hoc again when it
/// was signed. Its bytes before the section are unchanged but for the load commands' sizes and
/// offsets; `__LINKEDIT` follows the payload, every offset into it moved along.
///
/// # Errors
///
/// Returns an error naming why `runtime` can't carry it.
pub fn standalone(runtime: &[u8], payload: &[u8]) -> Result<Vec<u8>, String> {
    check(runtime)?;
    let found = commands(runtime).ok_or("the runtime's load commands don't read")?;
    let (jess, section) = found.reserved.ok_or("the runtime has no __JESS,__jsw section")?;
    let linkedit = found.linkedit.ok_or("the runtime has no __LINKEDIT segment")?;
    let size = |value: u64| usize::try_from(value).map_err(|_| "the runtime is too large to read".to_owned());
    // What __LINKEDIT holds before the signature, which is rewritten.
    let content_end = match found.signature {
        Some(command) => {
            let dataoff = u64::from(u32_at(runtime, command + 8).unwrap_or_default());
            let datasize = u64::from(u32_at(runtime, command + 12).unwrap_or_default());
            if dataoff + datasize != runtime.len() as u64 || dataoff < linkedit.fileoff {
                return Err("the runtime's code signature isn't the last thing in it".to_owned());
            }
            dataoff
        }
        None => runtime.len() as u64,
    };

    let grown = (payload.len() as u64).div_ceil(SEGMENT_ALIGN).max(1) * SEGMENT_ALIGN;
    let linkedit_fileoff = jess.fileoff + grown;
    let mut bytes = runtime[..size(jess.fileoff)?].to_vec();
    bytes.extend_from_slice(payload);
    bytes.resize(size(linkedit_fileoff)?, 0);
    bytes.extend_from_slice(&runtime[size(linkedit.fileoff)?..size(content_end)?]);
    move_offsets(
        &mut bytes,
        &found.all,
        linkedit.fileoff,
        i128::from(linkedit_fileoff) - i128::from(linkedit.fileoff),
    )?;

    put_u64(&mut bytes, jess.command + 32, grown);
    put_u64(&mut bytes, jess.command + 48, grown);
    put_u64(&mut bytes, section + 32, jess.vmaddr);
    put_u64(&mut bytes, section + 40, payload.len() as u64);
    put_u32(
        &mut bytes,
        section + 48,
        u32::try_from(jess.fileoff).map_err(|_| "the reserved section is past 4 GiB")?,
    );
    put_u64(&mut bytes, linkedit.command + 24, jess.vmaddr + grown);
    put_u64(&mut bytes, linkedit.command + 40, linkedit_fileoff);

    let Some(command) = found.signature else {
        let filesize = bytes.len() as u64 - linkedit_fileoff;
        put_u64(&mut bytes, linkedit.command + 48, filesize);
        put_u64(
            &mut bytes,
            linkedit.command + 32,
            filesize.div_ceil(SEGMENT_ALIGN) * SEGMENT_ALIGN,
        );
        return Ok(bytes);
    };
    let identifier = identifier(runtime, command).unwrap_or_else(|| b"jess".to_vec());
    sign(&mut bytes, command, linkedit.command, linkedit_fileoff, &identifier)?;
    Ok(bytes)
}

/// Moves every offset into `__LINKEDIT`, which started at `from`, by `delta`: the symbol tables',
/// dyld's, a note's, and each `linkedit_data_command`'s but the signature's, which is rewritten.
fn move_offsets(bytes: &mut [u8], commands: &[(usize, u32)], from: u64, delta: i128) -> Result<(), String> {
    let moved = |value: u64| -> Result<u64, String> {
        if value == 0 || value < from {
            return Ok(value);
        }
        u64::try_from(i128::from(value) + delta).map_err(|_| "an offset moved out of range".to_owned())
    };
    let moved_u32 = |bytes: &mut [u8], at: usize| -> Result<(), String> {
        let value = moved(u64::from(u32_at(bytes, at).unwrap_or_default()))?;
        put_u32(bytes, at, u32::try_from(value).map_err(|_| "an offset passed 4 GiB")?);
        Ok(())
    };
    for (at, kind) in commands {
        let at = *at;
        match *kind {
            LC_SYMTAB => {
                moved_u32(bytes, at + 8)?;
                moved_u32(bytes, at + 16)?;
            }
            LC_DYSYMTAB => {
                for field in [32, 40, 48, 56, 64, 72] {
                    moved_u32(bytes, at + field)?;
                }
            }
            LC_DYLD_INFO | LC_DYLD_INFO_ONLY => {
                for field in [8, 16, 24, 32, 40] {
                    moved_u32(bytes, at + field)?;
                }
            }
            LC_NOTE => {
                let value = moved(u64_at(bytes, at + 24).unwrap_or_default())?;
                put_u64(bytes, at + 24, value);
            }
            kind if kind != LC_CODE_SIGNATURE && LINKEDIT_DATA.contains(&kind) => moved_u32(bytes, at + 8)?,
            _ => {}
        }
    }
    Ok(())
}

/// Signs `bytes` ad hoc as `identifier`: the signature command at `command` and `__LINKEDIT`'s
/// sizes (its command at `linkedit`, its data from `linkedit_fileoff`) are set first, since they
/// are in the first page the signature hashes, and then the signature follows, on a 16-byte
/// boundary as the linker places it.
fn sign(
    bytes: &mut Vec<u8>,
    command: usize,
    linkedit: usize,
    linkedit_fileoff: u64,
    identifier: &[u8],
) -> Result<(), String> {
    bytes.resize(bytes.len().next_multiple_of(16), 0);
    let signature_at = bytes.len();
    let slots = signature_at.div_ceil(CODE_PAGE);
    let directory_len = CODE_DIRECTORY_HEADER + identifier.len() + 1 + slots * 32;
    let signature_len = 12 + 8 + directory_len;
    put_u32(
        bytes,
        command + 8,
        u32::try_from(signature_at).map_err(|_| "the signature is past 4 GiB")?,
    );
    put_u32(
        bytes,
        command + 12,
        u32::try_from(signature_len).map_err(|_| "the signature is too large")?,
    );
    let filesize = (signature_at + signature_len) as u64 - linkedit_fileoff;
    put_u64(bytes, linkedit + 48, filesize);
    put_u64(bytes, linkedit + 32, filesize.div_ceil(SEGMENT_ALIGN) * SEGMENT_ALIGN);

    let text = commands(bytes)
        .and_then(|signed| {
            signed.all.iter().find_map(|(at, kind)| {
                (*kind == LC_SEGMENT_64 && name(&bytes[at + 8..at + 24]) == b"__TEXT").then(|| {
                    (
                        u64_at(bytes, at + 40).unwrap_or_default(),
                        u64_at(bytes, at + 48).unwrap_or_default(),
                    )
                })
            })
        })
        .ok_or("the runtime has no __TEXT segment")?;
    let signature = signature(bytes, identifier, text, signature_len)?;
    bytes.extend_from_slice(&signature);
    Ok(())
}

/// The identifier of the code directory the signature command at `command` points to.
fn identifier(bytes: &[u8], command: usize) -> Option<Vec<u8>> {
    let read_be = |at: usize| {
        bytes
            .get(at..at + 4)
            .map(|word| u32::from_be_bytes([word[0], word[1], word[2], word[3]]))
    };
    let blob = usize::try_from(u32_at(bytes, command + 8)?).ok()?;
    if read_be(blob)? != CSMAGIC_EMBEDDED_SIGNATURE {
        return None;
    }
    let count = usize::try_from(read_be(blob + 8)?).ok()?;
    (0..count).find_map(|index| {
        let entry = blob + 12 + index * 8;
        let directory = blob + usize::try_from(read_be(entry + 4)?).ok()?;
        if read_be(entry)? != 0 || read_be(directory)? != CSMAGIC_CODEDIRECTORY {
            return None;
        }
        let ident = directory + usize::try_from(read_be(directory + 20)?).ok()?;
        let end = bytes.get(ident..)?.iter().position(|byte| *byte == 0)?;
        Some(bytes[ident..ident + end].to_vec())
    })
}

/// The ad hoc signature of `bytes`, everything before it: a superblob holding one code directory,
/// version 0x20400, hashing each 4 KiB page with SHA-256.
fn signature(bytes: &[u8], identifier: &[u8], (text_off, text_size): (u64, u64), length: usize) -> Result<Vec<u8>, String> {
    let be32 = |value: usize| -> Result<[u8; 4], String> {
        u32::try_from(value)
            .map(u32::to_be_bytes)
            .map_err(|_| "the signature is too large".to_owned())
    };
    let slots = bytes.len().div_ceil(CODE_PAGE);
    let hash_offset = CODE_DIRECTORY_HEADER + identifier.len() + 1;
    let directory_len = hash_offset + slots * 32;
    let mut out = Vec::with_capacity(length);
    out.extend_from_slice(&CSMAGIC_EMBEDDED_SIGNATURE.to_be_bytes());
    out.extend_from_slice(&be32(length)?);
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&20u32.to_be_bytes());
    out.extend_from_slice(&CSMAGIC_CODEDIRECTORY.to_be_bytes());
    out.extend_from_slice(&be32(directory_len)?);
    out.extend_from_slice(&CODE_DIRECTORY_VERSION.to_be_bytes());
    out.extend_from_slice(&CODE_DIRECTORY_FLAGS.to_be_bytes());
    out.extend_from_slice(&be32(hash_offset)?);
    out.extend_from_slice(&be32(CODE_DIRECTORY_HEADER)?);
    out.extend_from_slice(&0u32.to_be_bytes()); // special slots
    out.extend_from_slice(&be32(slots)?);
    out.extend_from_slice(&be32(bytes.len())?); // code limit
    out.extend_from_slice(&[32, 2, 0, 12]); // hash size, SHA-256, platform, log2 page size
    out.extend_from_slice(&0u32.to_be_bytes()); // spare2
    out.extend_from_slice(&0u32.to_be_bytes()); // scatter offset
    out.extend_from_slice(&0u32.to_be_bytes()); // team offset
    out.extend_from_slice(&0u32.to_be_bytes()); // spare3
    out.extend_from_slice(&0u64.to_be_bytes()); // 64-bit code limit, unused under 4 GiB
    out.extend_from_slice(&text_off.to_be_bytes());
    out.extend_from_slice(&text_size.to_be_bytes());
    out.extend_from_slice(&CS_EXECSEG_MAIN_BINARY.to_be_bytes());
    out.extend_from_slice(identifier);
    out.push(0);
    for page in bytes.chunks(CODE_PAGE) {
        out.extend_from_slice(&Sha256::digest(page));
    }
    debug_assert_eq!(out.len(), length);
    Ok(out)
}

/// Whether `runtime`, a Mach-O file, is a macOS jess that can carry a packed workspace.
///
/// # Errors
///
/// Returns an error naming why it can't: it is universal, 32-bit, or big-endian; it reserves no
/// section, which a jess older than standalone executables doesn't; it already carries one; or its
/// reserved section isn't directly before `__LINKEDIT`.
pub fn check(runtime: &[u8]) -> Result<(), String> {
    if u32_at(runtime, 0) != Some(MH_MAGIC_64) {
        return Err("the runtime is a universal, 32-bit, or big-endian macOS executable; use the one-architecture jess for the Mac it is for".to_owned());
    }
    let found = commands(runtime).ok_or("the runtime's load commands don't read")?;
    let Some((jess, section)) = found.reserved else {
        return Err("the runtime predates standalone executables, and has no __JESS,__jsw section to carry one in; use a jess as new as this one".to_owned());
    };
    let size = u64_at(runtime, section + 40).unwrap_or_default();
    let offset = usize::try_from(u32_at(runtime, section + 48).unwrap_or_default()).unwrap_or(usize::MAX);
    if size != MAGIC.len() as u64 || runtime.get(offset..offset + MAGIC.len()) != Some(&MAGIC[..]) {
        return Err("the runtime already carries a packed workspace; start from jess itself".to_owned());
    }
    let Some(linkedit) = found.linkedit else {
        return Err("the runtime has no __LINKEDIT segment".to_owned());
    };
    if jess.fileoff + jess.filesize != linkedit.fileoff || jess.vmaddr + jess.vmsize > linkedit.vmaddr {
        return Err("the runtime's __JESS segment isn't directly before __LINKEDIT, where it can grow".to_owned());
    }
    Ok(())
}
