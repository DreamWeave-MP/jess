//! Buffers: bytes on the GPU, written from the CPU and read back.

use std::fmt;
use std::ops::BitOr;
use std::sync::{Arc, Mutex, PoisonError};

use super::live::{Kind, Token};
use super::{Gpu, GpuError};

/// What a buffer is bound as, any number at once. Every buffer can be written and read back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BufferUsage(u8);

impl BufferUsage {
    /// Vertices or instances, a vertex buffer.
    pub const VERTEX: BufferUsage = BufferUsage(1);
    /// Indices of the vertices to draw.
    pub const INDEX: BufferUsage = BufferUsage(2);
    /// A uniform block.
    pub const UNIFORM: BufferUsage = BufferUsage(4);
    /// A storage buffer.
    pub const STORAGE: BufferUsage = BufferUsage(8);
    /// The counts of indirect draws: what the GPU wrote says how much to draw.
    pub const INDIRECT: BufferUsage = BufferUsage(16);

    /// Whether this includes every usage `other` does.
    #[must_use]
    pub fn contains(self, other: BufferUsage) -> bool {
        self.0 & other.0 == other.0
    }

    fn raw(self) -> wgpu::BufferUsages {
        let mut usages = wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        for (usage, raw) in [
            (BufferUsage::VERTEX, wgpu::BufferUsages::VERTEX),
            (BufferUsage::INDEX, wgpu::BufferUsages::INDEX),
            (BufferUsage::UNIFORM, wgpu::BufferUsages::UNIFORM),
            (BufferUsage::STORAGE, wgpu::BufferUsages::STORAGE),
            (BufferUsage::INDIRECT, wgpu::BufferUsages::INDIRECT),
        ] {
            if self.contains(usage) {
                usages |= raw;
            }
        }
        usages
    }
}

impl BitOr for BufferUsage {
    type Output = BufferUsage;

    fn bitor(self, other: BufferUsage) -> BufferUsage {
        BufferUsage(self.0 | other.0)
    }
}

impl fmt::Display for BufferUsage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = [
            (BufferUsage::VERTEX, "vertex"),
            (BufferUsage::INDEX, "index"),
            (BufferUsage::UNIFORM, "uniform"),
            (BufferUsage::STORAGE, "storage"),
            (BufferUsage::INDIRECT, "indirect"),
        ]
        .into_iter()
        .filter(|(usage, _)| self.contains(*usage))
        .map(|(_, name)| name)
        .collect();
        if names.is_empty() {
            return formatter.write_str("copies only");
        }
        formatter.write_str(&names.join(", "))
    }
}

/// Bytes on the GPU.
pub struct Buffer {
    pub(crate) raw: wgpu::Buffer,
    size: u64,
    usage: BufferUsage,
    /// What the CPU wrote into an indirect buffer, on a device that draws an indirect draw from
    /// instance 0 only: what a draw's check reads the first instance it would start at from.
    /// `None` in it once the GPU may have written the buffer, and for every other buffer: a
    /// storage buffer is the GPU's to write at any time.
    written: Option<Mutex<Option<Box<[u8]>>>>,
    token: Token,
}

impl fmt::Debug for Buffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Buffer")
            .field("size", &self.size)
            .field("usage", &self.usage)
            .finish_non_exhaustive()
    }
}

impl Buffer {
    /// Its identity, which no other buffer or texture ever has.
    pub(crate) fn id(&self) -> u64 {
        self.token.id()
    }

    /// How many bytes it holds.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// What it may be bound as.
    #[must_use]
    pub fn usage(&self) -> BufferUsage {
        self.usage
    }

    /// The first instance the indirect arguments ending with the word at `offset` start at, when
    /// it is known not to be 0 on a device that draws from 0 only.
    pub(super) fn first_instance(&self, offset: u64) -> Option<u32> {
        let start = usize::try_from(offset).ok()?;
        let written = self.written.as_ref()?.lock().unwrap_or_else(PoisonError::into_inner);
        let word: [u8; 4] = written.as_deref()?.get(start..start + 4)?.try_into().ok()?;
        drop(written);
        Some(u32::from_le_bytes(word)).filter(|first| *first != 0)
    }

    /// Says the GPU wrote it, a copy or a clear: what the CPU wrote no longer says what it holds.
    pub(super) fn gpu_wrote(&self) {
        if let Some(written) = &self.written {
            *written.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
    }

    /// Remembers that the CPU wrote `bytes` at `offset`, a range inside it.
    fn cpu_wrote(&self, offset: u64, bytes: &[u8]) {
        let Some(written) = &self.written else {
            return;
        };
        let mut written = written.lock().unwrap_or_else(PoisonError::into_inner);
        let range = usize::try_from(offset)
            .ok()
            .and_then(|start| written.as_deref_mut()?.get_mut(start..start + bytes.len()));
        if let Some(range) = range {
            range.copy_from_slice(bytes);
        }
    }
}

/// Copies move whole 4-byte words.
pub(super) const COPY_ALIGNMENT: u64 = wgpu::COPY_BUFFER_ALIGNMENT;

impl Gpu {
    /// A buffer of `size` bytes, zeroed, bound as `usage`.
    ///
    /// # Errors
    ///
    /// A size of 0, not a multiple of 4, or past the device's largest buffer.
    pub fn buffer(&self, size: u64, usage: BufferUsage) -> Result<Buffer, GpuError> {
        if size == 0 || !size.is_multiple_of(COPY_ALIGNMENT) || size > self.info.max_buffer_size {
            return Err(GpuError::Invalid(format!(
                "a buffer of {size} bytes is invalid: expected a multiple of 4 from 4 to {}",
                self.info.max_buffer_size
            )));
        }
        let raw = self.checked(|| {
            self.device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size,
                usage: usage.raw(),
                mapped_at_creation: false,
            })
        })?;
        // Only an indirect buffer nothing but copies and the CPU write, where a first instance
        // would be dropped unseen, is worth a copy of what it holds.
        let shadowed = usage.contains(BufferUsage::INDIRECT)
            && !usage.contains(BufferUsage::STORAGE)
            && !self.info.indirect_first_instance;
        let written = shadowed
            .then(|| usize::try_from(size).ok())
            .flatten()
            .map(|size| Mutex::new(Some(vec![0; size].into_boxed_slice())));
        Ok(Buffer {
            raw,
            size,
            usage,
            written,
            token: self.live.token(Kind::Buffer),
        })
    }

    /// Writes `bytes` into `buffer` from `offset`, before the next work the GPU is given.
    ///
    /// # Errors
    ///
    /// An offset or length that isn't a multiple of 4, or bytes that run past the buffer's end.
    pub fn write_buffer(&self, buffer: &Buffer, offset: u64, bytes: &[u8]) -> Result<(), GpuError> {
        let length = bytes.len() as u64;
        check_range(buffer, offset, length)?;
        // Before the GPU has them: a draw checked from another thread in between is checked
        // against what it will draw with.
        buffer.cpu_wrote(offset, bytes);
        self.checked(|| self.queue.write_buffer(&buffer.raw, offset, bytes))
    }

    /// Writes `bytes` into `buffer` at `offset`, as [`Gpu::write_buffer`] does, at the next
    /// [`Gpu::commit`]: checked now, and applied with the rest of the frame's changes.
    ///
    /// # Errors
    ///
    /// As [`Gpu::write_buffer`], or no room before the next commit.
    pub fn write_buffer_later(&self, buffer: &Arc<Buffer>, offset: u64, bytes: Vec<u8>) -> Result<(), GpuError> {
        check_range(buffer, offset, bytes.len() as u64)?;
        let buffer = Arc::clone(buffer);
        self.defer(bytes.len(), move |gpu| gpu.write_buffer(&buffer, offset, &bytes))
    }

    /// Reads `length` bytes of `buffer` from `offset`, once everything submitted so far is done.
    ///
    /// # Errors
    ///
    /// An offset or length that isn't a multiple of 4, or a range past the buffer's end.
    pub fn read_buffer(&self, buffer: &Buffer, offset: u64, length: u64) -> Result<Vec<u8>, GpuError> {
        check_range(buffer, offset, length)?;
        if length == 0 {
            return Ok(Vec::new());
        }
        self.read_back(length, |encoder, staging| {
            encoder.copy_buffer_to_buffer(&buffer.raw, offset, staging, 0, length);
        })
    }
}

/// Whether `length` bytes from `offset` are whole words inside `buffer`.
pub(super) fn check_range(buffer: &Buffer, offset: u64, length: u64) -> Result<(), GpuError> {
    let aligned = offset.is_multiple_of(COPY_ALIGNMENT) && length.is_multiple_of(COPY_ALIGNMENT);
    if !aligned || offset.checked_add(length).is_none_or(|end| end > buffer.size) {
        return Err(GpuError::Invalid(format!(
            "{length} bytes at {offset} are invalid in a buffer of {}: expected a multiple of 4 bytes at a \
             multiple of 4, inside it",
            buffer.size
        )));
    }
    Ok(())
}
