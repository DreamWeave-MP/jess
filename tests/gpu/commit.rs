//! Changes deferred to a commit: applied together, in order, under the frame lock, and bounded.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use super::gpu;

use jess::gpu::{BYTES, BufferUsage, CHANGES, GpuError};

#[test]
fn deferred_changes_apply_together_at_the_commit() {
    let Some(gpu) = gpu("deferred_changes_apply_together_at_the_commit") else {
        return;
    };
    let buffer = Arc::new(gpu.buffer(8, BufferUsage::STORAGE).unwrap());
    gpu.write_buffer_later(&buffer, 0, vec![1, 2, 3, 4]).unwrap();
    gpu.write_buffer_later(&buffer, 4, vec![5, 6, 7, 8]).unwrap();
    assert_eq!(gpu.room().changes, CHANGES - 2);
    assert_eq!(gpu.room().bytes, BYTES - 8);
    assert_eq!(
        gpu.read_buffer(&buffer, 0, 8).unwrap(),
        [0; 8],
        "nothing applied before the commit"
    );

    let commits = gpu.commits();
    assert!(gpu.commit().unwrap());
    assert_eq!(gpu.commits(), commits + 1);
    assert_eq!(gpu.read_buffer(&buffer, 0, 8).unwrap(), [1, 2, 3, 4, 5, 6, 7, 8]);
    assert!(!gpu.commit().unwrap(), "nothing left to apply");
    assert_eq!(gpu.commits(), commits + 1);

    // In order, and every change applied even when one before it failed: whole, not transactional.
    let order = Arc::new(AtomicU32::new(0));
    for step in 1..=3 {
        let order = Arc::clone(&order);
        gpu.defer(0, move |_| {
            order.store(order.load(Ordering::Relaxed) * 10 + step, Ordering::Relaxed);
            if step == 2 {
                return Err(GpuError::Invalid("the second change failed".to_owned()));
            }
            Ok(())
        })
        .unwrap();
    }
    assert_eq!(gpu.commit().unwrap_err().to_string(), "the second change failed");
    assert_eq!(order.load(Ordering::Relaxed), 123);

    // Checked when it is asked for, not when it is applied.
    assert!(gpu.write_buffer_later(&buffer, 4, vec![0; 8]).is_err());
    assert!(gpu.defer(BYTES + 1, |_| Ok(())).unwrap_err().to_string().contains("no room"));
    for _ in 0..CHANGES {
        gpu.defer(0, |_| Ok(())).unwrap();
    }
    assert_eq!(gpu.room().changes, 0);
    assert!(gpu.defer(0, |_| Ok(())).is_err());
    gpu.commit().unwrap();
    assert_eq!(gpu.room().changes, CHANGES);
}
