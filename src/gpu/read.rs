//! Reads: bytes the GPU copies back for the CPU, asked for now and taken when they are there, so
//! a frame never waits on the GPU to finish what it is drawing to learn what it drew.

use std::sync::{Arc, Mutex, PoisonError};

use super::{Buffer, Gpu, GpuError, Texture};

/// What a mapping callback reports, once the GPU has copied the bytes and the buffer is mapped.
type Mapped = Arc<Mutex<Option<Result<(), wgpu::BufferAsyncError>>>>;

/// Bytes on their way back from the GPU: a buffer's range, or a level of a layer of a texture,
/// as the copy saw it when it was submitted.
///
/// [`PendingRead::poll`] says whether they are there without waiting; [`PendingRead::wait`] waits
/// for them. Either gives the same bytes every time once they are.
#[derive(Debug)]
pub struct PendingRead {
    device: wgpu::Device,
    staging: wgpu::Buffer,
    mapped: Mapped,
    /// A texture's rows: their bytes, and how far apart the copy laid them.
    rows: Option<(usize, usize)>,
    bytes: Option<Vec<u8>>,
}

impl PendingRead {
    /// The bytes, if the GPU has copied them; `None` while it hasn't. Never waits.
    ///
    /// # Errors
    ///
    /// The device lost, or the copy failed.
    pub fn poll(&mut self) -> Result<Option<&[u8]>, GpuError> {
        if self.bytes.is_none() {
            self.device
                .poll(wgpu::PollType::Poll)
                .map_err(|error| GpuError::Backend(error.to_string()))?;
            self.take()?;
        }
        Ok(self.bytes.as_deref())
    }

    /// The bytes, waiting for the GPU to finish everything submitted so far if it hasn't.
    ///
    /// # Errors
    ///
    /// The device lost, or the copy failed.
    pub fn wait(&mut self) -> Result<&[u8], GpuError> {
        if self.bytes.is_none() {
            self.device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|error| GpuError::Backend(error.to_string()))?;
            self.take()?;
        }
        self.bytes
            .as_deref()
            .ok_or_else(|| GpuError::Backend("the GPU finished without mapping the read".to_owned()))
    }

    /// Whether the bytes are here, as of the last poll or wait.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.bytes.is_some()
    }

    /// The bytes out of the mapped staging buffer, if the mapping finished.
    fn take(&mut self) -> Result<(), GpuError> {
        let finished = self.mapped.lock().unwrap_or_else(PoisonError::into_inner).take();
        let Some(finished) = finished else {
            return Ok(());
        };
        finished.map_err(|error| GpuError::Backend(error.to_string()))?;
        let mapped = self
            .staging
            .slice(..)
            .get_mapped_range()
            .map_err(|error| GpuError::Backend(error.to_string()))?;
        let bytes = match self.rows {
            Some((row, padded)) => mapped.chunks(padded).flat_map(|line| &line[..row]).copied().collect(),
            None => mapped.to_vec(),
        };
        drop(mapped);
        self.staging.unmap();
        self.bytes = Some(bytes);
        Ok(())
    }
}

impl Gpu {
    /// Copies `size` bytes into a buffer the CPU can read with `copy`, submits it, and asks for it
    /// mapped; `rows` unpads a texture's.
    pub(crate) fn read_later(
        &self,
        size: u64,
        rows: Option<(usize, usize)>,
        copy: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) -> Result<PendingRead, GpuError> {
        let staging = self.checked(|| {
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("jess readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("jess readback"),
            });
            copy(&mut encoder, &staging);
            self.queue.submit([encoder.finish()]);
            staging
        })?;
        let mapped: Mapped = Arc::default();
        let reported = Arc::clone(&mapped);
        staging.slice(..).map_async(wgpu::MapMode::Read, move |result| {
            *reported.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        });
        Ok(PendingRead {
            device: self.device.clone(),
            staging,
            mapped,
            rows,
            bytes: None,
        })
    }

    /// `length` bytes of `buffer` from `offset`, as [`Gpu::read_buffer`] reads them, without
    /// waiting for them.
    ///
    /// # Errors
    ///
    /// As [`Gpu::read_buffer`].
    pub fn read_buffer_later(&self, buffer: &Buffer, offset: u64, length: u64) -> Result<PendingRead, GpuError> {
        super::buffer::check_range(buffer, offset, length)?;
        // A mapping of no bytes is no mapping: copy a word nobody reads instead.
        let size = length.max(super::buffer::COPY_ALIGNMENT);
        let mut read = self.read_later(size, None, |encoder, staging| {
            if length > 0 {
                encoder.copy_buffer_to_buffer(&buffer.raw, offset, staging, 0, length);
            }
        })?;
        if length == 0 {
            read.rows = Some((0, usize::try_from(size).unwrap_or(usize::MAX)));
        }
        Ok(read)
    }

    /// Mip `level` of `layer` of `texture`, as [`Gpu::read_texture`] reads it, without waiting for
    /// it.
    ///
    /// # Errors
    ///
    /// As [`Gpu::read_texture`].
    pub fn read_texture_later(&self, texture: &Texture, level: u32, layer: u32) -> Result<PendingRead, GpuError> {
        super::texture::read_later(self, texture, level, layer)
    }
}
