//! What buffers cost and who owns them as they cross worker boundaries.

use jess::runtime::{Message, push_message};

use super::support;
use crate::allocations::measure;

const PAYLOAD: usize = 16 * 1024 * 1024;

#[test]
fn an_ordinary_buffer_is_copied_once_each_way() {
    let harness = support::domain("buffer copies", &[]);
    harness
        .domain
        .with_runtime(|runtime| {
            runtime.exec(&format!("payload = buffer.create({PAYLOAD})"))?;
            let value = runtime.global("payload")?;
            runtime.stack().with_frame(|frame| {
                value.with_value(frame, |frame, view| {
                    let (message, _, read) = measure(|| Message::read(frame, view, "payload").unwrap());
                    let (pushed, _, written) = measure(|| push_message(frame, &message));
                    pushed?;
                    assert!((PAYLOAD..PAYLOAD * 3 / 2).contains(&read), "read allocated {read} bytes");
                    // The fresh buffer is Luau's own allocation, which this allocator doesn't see.
                    assert!(written < PAYLOAD / 2, "push allocated {written} bytes");
                    Ok(())
                })
            })
        })
        .unwrap()
        .unwrap();
}
