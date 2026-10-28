//! Changes deferred to a commit: what one frame of a script changes about what a device draws,
//! applied together, so whatever draws on another thread never sees half of it.
//!
//! [`Gpu::defer`] queues a change; [`Gpu::commit`] applies every queued change, in order, under
//! the device's frame lock, which whoever records a frame on another thread holds while it
//! records. The queue is bounded: a frame that changes more than [`CHANGES`] things, or carries
//! more than [`BYTES`] bytes of data, is refused the change that doesn't fit, and
//! [`Gpu::room`] says how much fits.

use std::sync::atomic::Ordering;
use std::sync::{MutexGuard, PoisonError};

use super::{Gpu, GpuError};

/// The most changes a device holds between commits.
pub const CHANGES: usize = 65_536;

/// The most bytes of data the changes a device holds between commits carry.
pub const BYTES: usize = 256 << 20;

/// A change, applied at the next commit.
pub type Change = Box<dyn FnOnce(&Gpu) -> Result<(), GpuError> + Send>;

/// The changes waiting for the next commit, and the bytes they carry.
#[derive(Default)]
pub struct Pending {
    changes: Vec<Change>,
    bytes: usize,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pending")
            .field("changes", &self.changes.len())
            .field("bytes", &self.bytes)
            .finish()
    }
}

/// How much more the changes before the next commit can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Room {
    /// Changes.
    pub changes: usize,
    /// Bytes of the data they carry.
    pub bytes: usize,
}

impl Gpu {
    /// Defers `change`, carrying `bytes` bytes of data, to the next [`Gpu::commit`].
    ///
    /// # Errors
    ///
    /// No room for it: the queue holds [`CHANGES`] changes, or the data would pass [`BYTES`].
    pub fn defer(
        &self,
        bytes: usize,
        change: impl FnOnce(&Gpu) -> Result<(), GpuError> + Send + 'static,
    ) -> Result<(), GpuError> {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        if pending.changes.len() == CHANGES || pending.bytes.saturating_add(bytes) > BYTES {
            return Err(GpuError::Invalid(format!(
                "no room for another change before the next commit: {} changes carrying {} bytes wait, and a frame \
                 holds {CHANGES} changes and {BYTES} bytes",
                pending.changes.len(),
                pending.bytes
            )));
        }
        pending.changes.push(Box::new(change));
        pending.bytes += bytes;
        drop(pending);
        Ok(())
    }

    /// How much more fits before the next commit.
    #[must_use]
    pub fn room(&self) -> Room {
        let pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        Room {
            changes: CHANGES - pending.changes.len(),
            bytes: BYTES.saturating_sub(pending.bytes),
        }
    }

    /// Applies every deferred change, in the order they were deferred, while nothing records a
    /// frame; whether there were any.
    ///
    /// A commit is whole in what a frame sees, not a transaction: a change that fails is reported,
    /// not undone, the changes after it still apply, and none before it is rolled back, so a
    /// frame drawn after it sees every other change in full.
    ///
    /// # Errors
    ///
    /// The first change that failed; the rest were applied.
    pub fn commit(&self) -> Result<bool, GpuError> {
        let _frame = self.frame();
        let changes = std::mem::take(&mut *self.pending.lock().unwrap_or_else(PoisonError::into_inner));
        if changes.changes.is_empty() {
            return Ok(false);
        }
        let mut failed = None;
        for change in changes.changes {
            if let Err(problem) = change(self) {
                failed.get_or_insert(problem);
            }
        }
        self.commits.fetch_add(1, Ordering::Release);
        failed.map_or(Ok(true), Err)
    }

    /// How many commits have applied anything: whoever draws what they change draws again when
    /// it moves.
    #[must_use]
    pub fn commits(&self) -> u64 {
        self.commits.load(Ordering::Acquire)
    }

    /// The frame lock: held by a commit, and by whoever records a frame on another thread.
    pub(crate) fn frame(&self) -> MutexGuard<'_, ()> {
        self.frame.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
