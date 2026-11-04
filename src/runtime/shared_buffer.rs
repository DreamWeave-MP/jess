//! Immutable storage shared by worker messages.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use l3i::userdata::Userdata;

// Native allocations are outside Luau's allocator. Bound their total independently of VM heaps.
const LIMIT: usize = 1024 * 1024 * 1024;
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct Storage(Vec<u8>);

impl Drop for Storage {
    fn drop(&mut self) {
        ALLOCATED.fetch_sub(self.0.len(), Ordering::Relaxed);
    }
}

#[derive(Debug)]
pub struct Builder(RefCell<Option<Storage>>);

// SAFETY: the payload is owned by one VM; its destructor does not touch Luau.
unsafe impl Userdata for Builder {
    const NAME: &'static str = "jess.runtime.SharedBufferBuilder";
}

#[derive(Clone, Debug)]
/// An immutable view of reference-counted native bytes.
pub struct SharedBuffer {
    storage: Arc<Storage>,
    start: usize,
    len: usize,
}

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

fn range(offset: i32, length: usize, available: usize) -> l3i::Result<std::ops::Range<usize>> {
    let offset = usize::try_from(offset).map_err(|_| l3i::Error::runtime("negative buffer offset"))?;
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= available)
        .ok_or_else(|| l3i::Error::runtime("buffer range is out of bounds"))?;
    Ok(offset..end)
}

impl Builder {
    pub(crate) fn new(length: i32) -> l3i::Result<Self> {
        let length = usize::try_from(length).map_err(|_| l3i::Error::runtime("negative buffer length"))?;
        #[allow(deprecated)] // `try_update` is newer than the MSRV.
        ALLOCATED
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(length).filter(|total| *total <= LIMIT)
            })
            .map_err(|_| l3i::Error::runtime("shared buffer memory limit exceeded"))?;
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(length).is_err() {
            ALLOCATED.fetch_sub(length, Ordering::Relaxed);
            return Err(l3i::Error::runtime("shared buffer allocation failed"));
        }
        bytes.resize(length, 0);
        Ok(Self(RefCell::new(Some(Storage(bytes)))))
    }

    pub(crate) fn write(&self, offset: i32, bytes: &[u8]) -> l3i::Result<()> {
        let mut builder = self.0.borrow_mut();
        let storage = builder.as_mut().ok_or_else(|| l3i::Error::runtime("builder is sealed"))?;
        let range = range(offset, bytes.len(), storage.0.len())?;
        storage.0[range].copy_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn write_buffer(&self, offset: i32, bytes: &l3i::convert::BufferView<'_>) -> l3i::Result<()> {
        let mut builder = self.0.borrow_mut();
        let storage = builder.as_mut().ok_or_else(|| l3i::Error::runtime("builder is sealed"))?;
        let range = range(offset, bytes.len(), storage.0.len())?;
        bytes.read(0, &mut storage.0[range])
    }

    pub(crate) fn seal(&self) -> l3i::Result<SharedBuffer> {
        let storage = self
            .0
            .borrow_mut()
            .take()
            .ok_or_else(|| l3i::Error::runtime("builder is sealed"))?;
        let len = storage.0.len();
        Ok(SharedBuffer {
            storage: Arc::new(storage),
            start: 0,
            len,
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
        &self.storage.0[self.start..self.start + self.len]
    }

    pub(crate) fn slice(&self, offset: i32, length: i32) -> l3i::Result<Self> {
        let length = usize::try_from(length).map_err(|_| l3i::Error::runtime("negative buffer length"))?;
        let range = range(offset, length, self.len)?;
        Ok(Self {
            storage: Arc::clone(&self.storage),
            start: self.start + range.start,
            len: length,
        })
    }

    pub(crate) fn read_f64(&self, offset: i32) -> l3i::Result<f64> {
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
        target_offset: i32,
        source_offset: i32,
        length: i32,
    ) -> l3i::Result<()> {
        let length = usize::try_from(length).map_err(|_| l3i::Error::runtime("negative buffer length"))?;
        let source = range(source_offset, length, self.len)?;
        let destination = range(target_offset, length, target.len())?;
        target.write(destination.start, &self.bytes()[source])
    }

    pub(crate) fn read_u32(&self, offset: i32) -> l3i::Result<u32> {
        let range = range(offset, 4, self.len)?;
        let mut bytes = [0; 4];
        bytes.copy_from_slice(&self.bytes()[range]);
        Ok(u32::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    // `ALLOCATED` is process-wide, and exact totals can't be read while another test allocates.
    static ACCOUNTING: Mutex<()> = Mutex::new(());

    fn allocated() -> usize {
        ALLOCATED.load(Ordering::Relaxed)
    }

    #[test]
    fn storage_is_charged_once_and_released_by_its_last_owner() {
        let _accounting = ACCOUNTING.lock().unwrap();
        let before = allocated();
        let builder = Builder::new(1024).unwrap();
        assert_eq!(allocated(), before + 1024);

        let shared = builder.seal().unwrap();
        let clone = shared.clone();
        let slice = shared.slice(8, 8).unwrap();
        assert_eq!(allocated(), before + 1024);
        assert_eq!(shared.owners(), 3);

        drop(shared);
        drop(builder);
        drop(clone);
        assert_eq!(allocated(), before + 1024);
        assert_eq!(slice.owners(), 1);

        drop(slice);
        assert_eq!(allocated(), before);
    }

    #[test]
    fn a_builder_that_never_seals_releases_its_charge() {
        let _accounting = ACCOUNTING.lock().unwrap();
        let before = allocated();
        drop(Builder::new(1024).unwrap());
        assert_eq!(allocated(), before);
    }

    #[test]
    fn a_refused_allocation_charges_nothing() {
        let _accounting = ACCOUNTING.lock().unwrap();
        let before = allocated();
        assert!(Builder::new(-1).is_err());
        assert!(Builder::new(i32::MAX).is_err());
        assert_eq!(allocated(), before);
    }

    #[test]
    fn views_compare_by_their_bytes() {
        let _accounting = ACCOUNTING.lock().unwrap();
        let sealed = |value: u8| {
            let builder = Builder::new(16).unwrap();
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
        let _accounting = ACCOUNTING.lock().unwrap();
        let builder = Builder::new(24).unwrap();
        builder.write(8, &42.5f64.to_le_bytes()).unwrap();
        let address = builder.0.borrow().as_ref().unwrap().0.as_ptr();
        let shared = builder.seal().unwrap();
        assert_eq!(address, shared.bytes().as_ptr());
        assert!(builder.seal().is_err());
        assert!(builder.write(0, &[1]).is_err());
        let slice = shared.slice(8, 8).unwrap();
        assert!(Arc::ptr_eq(&shared.storage, &slice.storage));
        let allocation = Arc::downgrade(&shared.storage);
        assert_eq!(slice.read_f64(0).unwrap(), 42.5);
        assert!(shared.slice(-1, 1).is_err());
        assert!(shared.slice(i32::MAX, 1).is_err());
        assert!(slice.read_f64(1).is_err());
        drop(shared);
        assert!(allocation.upgrade().is_some());
        let thread = std::thread::spawn(move || slice.read_f64(0).unwrap());
        assert_eq!(thread.join().unwrap(), 42.5);
        assert!(allocation.upgrade().is_none());
    }
}
