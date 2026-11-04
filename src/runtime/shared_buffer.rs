//! Immutable storage shared by worker messages, and the budgets that bound it.

use std::alloc::{Layout, alloc_zeroed};
use std::cell::RefCell;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};

use l3i::userdata::Userdata;

/// How much shared storage a process may hold until something raises the limit:
/// `@jess/sharedBuffer`'s `setLimit`, with the `shared-buffer.limit` capability.
pub const DEFAULT_SHARED_BUFFER_LIMIT: usize = 1024 * 1024 * 1024;

/// One level of the shared storage budget: a worker's, a domain's, or the process's. Native
/// allocations are outside Luau's allocator, so they are bounded independently of VM heaps.
///
/// These budget allocation, not retention or physical memory. An allocation is charged to the level
/// of the code that made it and every level above, once, however many views and messages share it,
/// and released when its last owner goes. Receiving and reading a view charge nothing.
#[derive(Debug)]
pub struct Ledger {
    used: AtomicUsize,
    limit: AtomicUsize,
    parent: Option<Arc<Ledger>>,
}

static PROCESS: LazyLock<Arc<Ledger>> = LazyLock::new(|| Ledger::new(DEFAULT_SHARED_BUFFER_LIMIT, None));

/// The top of every budget.
pub fn process_ledger() -> Arc<Ledger> {
    Arc::clone(&PROCESS)
}

impl Ledger {
    pub(crate) fn new(limit: usize, parent: Option<Arc<Ledger>>) -> Arc<Ledger> {
        Arc::new(Ledger {
            used: AtomicUsize::new(0),
            limit: AtomicUsize::new(limit),
            parent,
        })
    }

    /// What this level may hold: its own limit or the lowest above it.
    pub(crate) fn effective_limit(&self) -> usize {
        let own = self.limit.load(Ordering::Relaxed);
        self.parent.as_ref().map_or(own, |parent| own.min(parent.effective_limit()))
    }

    /// Raises or lowers this level's limit. What is already held stays held; a limit below it only
    /// refuses the next allocation.
    pub(crate) fn set_limit(&self, limit: usize) {
        self.limit.store(limit, Ordering::Relaxed);
    }

    /// Charges every level, or none if any would pass its limit.
    fn charge(&self, length: usize) -> bool {
        let limit = self.limit.load(Ordering::Relaxed);
        #[allow(deprecated)] // `try_update` is newer than the MSRV.
        let charged = self
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(length).filter(|total| *total <= limit)
            })
            .is_ok();
        if !charged {
            return false;
        }

        if self.parent.as_ref().is_some_and(|parent| !parent.charge(length)) {
            self.used.fetch_sub(length, Ordering::Relaxed);
            return false;
        }

        true
    }

    fn release(&self, length: usize) {
        self.used.fetch_sub(length, Ordering::Relaxed);
        if let Some(parent) = &self.parent {
            parent.release(length);
        }
    }
}

#[derive(Debug)]
struct Storage {
    bytes: Vec<u8>,
    ledger: Arc<Ledger>,
}

impl Drop for Storage {
    fn drop(&mut self) {
        self.ledger.release(self.bytes.len());
    }
}

/// `length` zero bytes, asked of the allocator as zeroed memory: pages the script never touches
/// stay unbacked, and a request the allocator refuses is an error. That is not a promise of physical
/// memory: where the system overcommits, a request can succeed and writing to it later can still
/// run the machine out.
fn zeroed(length: usize) -> Option<Vec<u8>> {
    if length == 0 {
        return Some(Vec::new());
    }

    let layout = Layout::array::<u8>(length).ok()?;
    // SAFETY: the layout has a non-zero size.
    let pointer = unsafe { alloc_zeroed(layout) };
    if pointer.is_null() {
        return None;
    }

    // SAFETY: the global allocator made this with `u8`'s alignment and `length` bytes of capacity,
    // and all of them are initialized.
    Some(unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(pointer, length)) }.into_vec())
}

#[derive(Debug)]
pub struct Builder(RefCell<Option<Storage>>);

// SAFETY: the payload is owned by one VM; its destructor does not touch Luau.
unsafe impl Userdata for Builder {
    const NAME: &'static str = "jess.runtime.SharedBufferBuilder";
}

#[derive(Clone, Debug)]
/// An immutable view of reference-counted native bytes. The address and length come first and
/// never change: native code reads them from the payload ([`crate::runtime::shared_buffer_lowering`]).
#[repr(C)]
pub struct SharedBuffer {
    address: usize,
    len: usize,
    storage: Arc<Storage>,
    start: usize,
}

/// Where native code finds a view's address and length in its payload.
pub const PAYLOAD_ADDRESS: usize = 0;
pub const PAYLOAD_LENGTH: usize = 8;

const _: () = {
    assert!(std::mem::offset_of!(SharedBuffer, address) == PAYLOAD_ADDRESS);
    // The lowering runs only on 64-bit targets.
    assert!(std::mem::offset_of!(SharedBuffer, len) == PAYLOAD_LENGTH || cfg!(not(target_pointer_width = "64")));
};

impl PartialEq for SharedBuffer {
    fn eq(&self, other: &Self) -> bool {
        let same_view = Arc::ptr_eq(&self.storage, &other.storage) && self.start == other.start && self.len == other.len;
        same_view || self.bytes() == other.bytes()
    }
}

// SAFETY: storage is immutable after publication and its destructor does not touch Luau.
unsafe impl Userdata for SharedBuffer {
    const NAME: &'static str = "jess.runtime.SharedBuffer";
}

fn range(offset: i64, length: usize, available: usize) -> l3i::Result<std::ops::Range<usize>> {
    let offset = usize::try_from(offset).map_err(|_| l3i::Error::runtime("negative buffer offset"))?;
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= available)
        .ok_or_else(|| l3i::Error::runtime("buffer range is out of bounds"))?;
    Ok(offset..end)
}

impl Builder {
    pub(crate) fn new(length: i64, ledger: Arc<Ledger>) -> l3i::Result<Self> {
        let length = usize::try_from(length).map_err(|_| l3i::Error::runtime("negative buffer length"))?;
        if !ledger.charge(length) {
            return Err(l3i::Error::runtime("shared buffer memory limit exceeded"));
        }

        let Some(bytes) = zeroed(length) else {
            ledger.release(length);
            return Err(l3i::Error::runtime("shared buffer allocation failed"));
        };
        Ok(Self(RefCell::new(Some(Storage { bytes, ledger }))))
    }

    pub(crate) fn write(&self, offset: i64, bytes: &[u8]) -> l3i::Result<()> {
        let mut builder = self.0.borrow_mut();
        let storage = builder.as_mut().ok_or_else(|| l3i::Error::runtime("builder is sealed"))?;
        let range = range(offset, bytes.len(), storage.bytes.len())?;
        storage.bytes[range].copy_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn write_buffer(&self, offset: i64, bytes: &l3i::convert::BufferView<'_>) -> l3i::Result<()> {
        let mut builder = self.0.borrow_mut();
        let storage = builder.as_mut().ok_or_else(|| l3i::Error::runtime("builder is sealed"))?;
        let range = range(offset, bytes.len(), storage.bytes.len())?;
        bytes.read(0, &mut storage.bytes[range])
    }

    pub(crate) fn seal(&self) -> l3i::Result<SharedBuffer> {
        let storage = self
            .0
            .borrow_mut()
            .take()
            .ok_or_else(|| l3i::Error::runtime("builder is sealed"))?;
        let len = storage.bytes.len();
        // Sealing moves the `Vec`, not its allocation, and nothing can reallocate it afterwards.
        let address = storage.bytes.as_ptr() as usize;
        Ok(SharedBuffer {
            address,
            len,
            storage: Arc::new(storage),
            start: 0,
        })
    }
}

impl SharedBuffer {
    /// Whether two views retain the same backing allocation.
    #[must_use]
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.storage, &other.storage)
    }

    /// How many views, in any VM or message, retain this backing allocation right now.
    #[must_use]
    pub fn owners(&self) -> usize {
        Arc::strong_count(&self.storage)
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        &self.storage.bytes[self.start..self.start + self.len]
    }

    pub(crate) fn slice(&self, offset: i64, length: i64) -> l3i::Result<Self> {
        let length = usize::try_from(length).map_err(|_| l3i::Error::runtime("negative buffer length"))?;
        let range = range(offset, length, self.len)?;
        Ok(Self {
            address: self.address + range.start,
            len: length,
            storage: Arc::clone(&self.storage),
            start: self.start + range.start,
        })
    }

    pub(crate) fn read_f64(&self, offset: i64) -> l3i::Result<f64> {
        let range = range(offset, 8, self.len)?;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.bytes()[range]);
        Ok(f64::from_le_bytes(bytes))
    }

    /// Copies `length` bytes starting at `source_offset` into `target`. Both ranges are checked
    /// before any byte moves, so a failed call leaves `target` untouched.
    pub(crate) fn copy_into(
        &self,
        target: &l3i::convert::BufferView<'_>,
        target_offset: i64,
        source_offset: i64,
        length: i64,
    ) -> l3i::Result<()> {
        let length = usize::try_from(length).map_err(|_| l3i::Error::runtime("negative buffer length"))?;
        let source = range(source_offset, length, self.len)?;
        let destination = range(target_offset, length, target.len())?;
        target.write(destination.start, &self.bytes()[source])
    }

    pub(crate) fn read_u32(&self, offset: i64) -> l3i::Result<u32> {
        let range = range(offset, 4, self.len)?;
        let mut bytes = [0; 4];
        bytes.copy_from_slice(&self.bytes()[range]);
        Ok(u32::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn used(ledger: &Ledger) -> usize {
        ledger.used.load(Ordering::Relaxed)
    }

    fn ledger() -> Arc<Ledger> {
        Ledger::new(1024 * 1024, None)
    }

    #[test]
    fn storage_is_charged_once_and_released_by_its_last_owner() {
        let ledger = ledger();
        let builder = Builder::new(1024, Arc::clone(&ledger)).unwrap();
        assert_eq!(used(&ledger), 1024);

        let shared = builder.seal().unwrap();
        let clone = shared.clone();
        let slice = shared.slice(8, 8).unwrap();
        assert_eq!(used(&ledger), 1024);
        assert_eq!(shared.owners(), 3);

        drop(shared);
        drop(builder);
        drop(clone);
        assert_eq!(used(&ledger), 1024);
        assert_eq!(slice.owners(), 1);

        drop(slice);
        assert_eq!(used(&ledger), 0);
    }

    #[test]
    fn a_builder_that_never_seals_releases_its_charge() {
        let ledger = ledger();
        drop(Builder::new(1024, Arc::clone(&ledger)).unwrap());
        assert_eq!(used(&ledger), 0);
    }

    #[test]
    fn a_refused_allocation_charges_nothing() {
        let ledger = ledger();
        assert!(Builder::new(-1, Arc::clone(&ledger)).is_err());
        assert!(Builder::new(i64::MAX, Arc::clone(&ledger)).is_err());
        assert_eq!(used(&ledger), 0);
    }

    #[test]
    fn every_level_is_charged_and_any_level_can_refuse() {
        let process = Ledger::new(100, None);
        let domain = Ledger::new(80, Some(Arc::clone(&process)));
        let worker = Ledger::new(50, Some(Arc::clone(&domain)));
        assert_eq!(worker.effective_limit(), 50);

        // The worker's own limit refuses first, and nothing above it is charged.
        assert!(Builder::new(51, Arc::clone(&worker)).is_err());
        assert_eq!((used(&worker), used(&domain), used(&process)), (0, 0, 0));

        let held = Builder::new(50, Arc::clone(&worker)).unwrap();
        assert_eq!((used(&worker), used(&domain), used(&process)), (50, 50, 50));

        // The domain's limit refuses a sibling, and what the sibling charged itself is undone.
        let sibling = Ledger::new(50, Some(Arc::clone(&domain)));
        assert!(Builder::new(31, Arc::clone(&sibling)).is_err());
        assert_eq!((used(&sibling), used(&domain), used(&process)), (0, 50, 50));

        drop(held);
        assert_eq!((used(&worker), used(&domain), used(&process)), (0, 0, 0));
    }

    // Mandatory and allocation-free: the 64-bit arithmetic the opt-in large test exercises for real.
    #[cfg(target_pointer_width = "64")]
    #[test]
    fn ranges_and_budgets_work_past_32_bits() {
        const FOUR_GIB: usize = 4 * 1024 * 1024 * 1024;
        let high = 3 * 1024 * 1024 * 1024;
        let range = range(high, 8, FOUR_GIB).unwrap();
        assert_eq!(range.start, 3_221_225_472);
        assert_eq!(range.end, 3_221_225_480);
        assert!(super::range(4 * 1024 * 1024 * 1024, 1, FOUR_GIB).is_err());
        assert!(super::range(i64::MAX, 8, FOUR_GIB).is_err());
        assert!(super::range(-1, 8, FOUR_GIB).is_err());

        let process = Ledger::new(FOUR_GIB, None);
        let worker = Ledger::new(FOUR_GIB, Some(Arc::clone(&process)));
        assert!(worker.charge(FOUR_GIB));
        assert!(!worker.charge(1));
        worker.release(FOUR_GIB);
        assert_eq!(used(&process), 0);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn a_refusal_from_the_allocator_releases_its_charge() {
        let ledger = Ledger::new(usize::MAX, None);
        let refused = Builder::new(i64::MAX, Arc::clone(&ledger));
        assert!(refused.is_err());
        assert_eq!(used(&ledger), 0);
    }

    #[test]
    fn a_lowered_limit_refuses_only_the_next_allocation() {
        let ledger = ledger();
        let held = Builder::new(1024, Arc::clone(&ledger)).unwrap();
        ledger.set_limit(16);
        assert!(Builder::new(1, Arc::clone(&ledger)).is_err());
        assert_eq!(used(&ledger), 1024);
        drop(held);
        assert!(Builder::new(16, Arc::clone(&ledger)).is_ok());
    }

    #[test]
    fn views_compare_by_their_bytes() {
        let ledger = ledger();
        let sealed = |value: u8| {
            let builder = Builder::new(16, Arc::clone(&ledger)).unwrap();
            builder.write(0, &[value; 16]).unwrap();
            builder.seal().unwrap()
        };
        let shared = sealed(1);
        assert_eq!(shared, shared.clone());
        assert_eq!(shared, sealed(1));
        assert_ne!(shared, sealed(2));
        assert_eq!(shared.slice(0, 8).unwrap(), shared.slice(8, 8).unwrap());
        assert_ne!(shared.slice(0, 8).unwrap(), shared);
    }

    #[test]
    fn sealing_and_slicing_preserve_the_allocation() {
        let builder = Builder::new(24, ledger()).unwrap();
        builder.write(8, &42.5f64.to_le_bytes()).unwrap();
        let address = builder.0.borrow().as_ref().unwrap().bytes.as_ptr();
        let shared = builder.seal().unwrap();
        assert_eq!(address, shared.bytes().as_ptr());
        assert!(builder.seal().is_err());
        assert!(builder.write(0, &[1]).is_err());
        let slice = shared.slice(8, 8).unwrap();
        assert!(Arc::ptr_eq(&shared.storage, &slice.storage));
        let allocation = Arc::downgrade(&shared.storage);
        assert_eq!(slice.read_f64(0).unwrap(), 42.5);
        assert!(shared.slice(-1, 1).is_err());
        assert!(shared.slice(i64::MAX, 1).is_err());
        assert!(slice.read_f64(1).is_err());
        drop(shared);
        assert!(allocation.upgrade().is_some());
        let thread = std::thread::spawn(move || slice.read_f64(0).unwrap());
        assert_eq!(thread.join().unwrap(), 42.5);
        assert!(allocation.upgrade().is_none());
    }
}
