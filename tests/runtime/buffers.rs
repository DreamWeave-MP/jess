//! What buffers cost and who owns them as they cross worker boundaries.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jess::runtime::l3i;
use jess::runtime::{
    CapabilitySet, Message, SHARED_BUFFER_CREATE, ScriptDomain, SharedBuffer, WorkerSpec, WorkerState, push_message,
};

use super::support::{self, Rendezvous};
use crate::allocations::measure;

const PAYLOAD: usize = 16 * 1024 * 1024;
const WAIT: Option<Duration> = Some(Duration::from_secs(20));

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

#[test]
fn copy_into_allocates_nothing_per_call() {
    let harness = support::domain("copy allocations", &[]);
    harness
        .domain
        .with_runtime(|runtime| {
            runtime.exec(
                r"
local builder = require('@jess/sharedBuffer').create(4096)
payload = builder:seal()
target = buffer.create(4096)

function copies(count)
  for _ = 1, count do
    payload:copyInto(target, 0, 0, 4096)
  end
end
",
            )?;
            runtime.exec("copies(1)")?;
            let (single, one, _) = measure(|| runtime.exec("copies(1)"));
            let (repeated, hundred_thousand, _) = measure(|| runtime.exec("copies(100000)"));
            single?;
            repeated?;
            assert!(
                hundred_thousand <= one + 8,
                "{one} allocations for one, {hundred_thousand} for 100000"
            );
            Ok::<(), l3i::Error>(())
        })
        .unwrap()
        .unwrap();
}

/// Seals 64 KiB of f64s equal to their index in the global `payload`, and reads it as a message.
/// The domain's global is one owner and the returned message another.
fn sealed(domain: &ScriptDomain) -> Message {
    domain
        .with_runtime(|runtime| {
            runtime.exec(
                r"
local builder = require('@jess/sharedBuffer').create(65536)
for index = 0, 8191 do
  builder:writef64(index * 8, index)
end
payload = builder:seal()
",
            )?;
            let value = runtime.global("payload")?;
            runtime.stack().with_frame(|frame| {
                value.with_value(frame, |frame, view| Ok(Message::read(frame, view, "payload").unwrap()))
            })
        })
        .unwrap()
        .unwrap()
}

fn shared(message: &Message) -> &SharedBuffer {
    let Message::SharedBuffer(bytes) = message else {
        panic!("expected a shared buffer, got {message:?}");
    };
    bytes
}

/// Waits for the owner count to reach `expected`: a worker drops what it held on its own thread,
/// after the reply or join that told the test it was done.
fn assert_owners(buffer: &SharedBuffer, expected: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while buffer.owners() != expected && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(buffer.owners(), expected);
}

#[test]
fn a_handle_nested_in_a_table_keeps_its_allocation() {
    let harness = support::domain("nested", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    let worker = domain
        .spawn_worker(WorkerSpec::new(
            "nested",
            r"
return function(message)
  local view = message.items[1]:slice(8, 8)
  assert(view:readf64(0) == 1)
  return { nested = { view } }
end
",
        ))
        .unwrap();
    domain
        .send_to_worker(worker, Message::map([("items", Message::List(vec![message.clone()]))]))
        .unwrap();
    let reply = domain.receive_from_worker(worker, WAIT).unwrap().unwrap();
    let Some(Message::List(nested)) = reply.get("nested") else {
        panic!("expected a list, got {reply:?}");
    };
    assert!(original.shares_storage(shared(&nested[0])));
    drop(reply);
    assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Stopped);
    assert_owners(original, baseline);
}

#[test]
fn a_worker_releases_what_it_kept_when_it_stops() {
    let harness = support::domain("kept", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    let worker = domain
        .spawn_worker(WorkerSpec::new(
            "kept",
            "return function(message) kept = message return 'kept' end",
        ))
        .unwrap();
    domain.send_to_worker(worker, message.clone()).unwrap();
    assert!(domain.receive_from_worker(worker, WAIT).unwrap().is_some());
    assert_owners(original, baseline + 1);
    assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Stopped);
    assert_owners(original, baseline);
}

#[test]
fn collection_in_a_live_worker_releases_a_dropped_handle() {
    let harness = support::domain("collected", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    let worker = domain
        .spawn_worker(WorkerSpec::new(
            "collected",
            r"
return function(message)
  if message == 'collect' then
    kept = nil
    -- This runtime has no collectgarbage, so allocate until the collector has swept.
    for _ = 1, 2000000 do
      local _ = {}
    end

    return 'collected'
  end

  kept = message
  return 'kept'
end
",
        ))
        .unwrap();
    domain.send_to_worker(worker, message.clone()).unwrap();
    assert!(domain.receive_from_worker(worker, WAIT).unwrap().is_some());
    assert_owners(original, baseline + 1);
    domain.send_to_worker(worker, Message::text("collect")).unwrap();
    assert!(domain.receive_from_worker(worker, WAIT).unwrap().is_some());
    assert_owners(original, baseline);
    assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Stopped);
}

#[test]
fn a_failed_worker_releases_its_queue() {
    let rendezvous = Rendezvous::new(2);
    let harness = support::domain_from(support::factory_with(Arc::clone(&rendezvous)), "failing", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    let worker = domain
        .spawn_worker(WorkerSpec::new(
            "failing",
            r"
local rendezvous = require('@test/rendezvous')

return function(_)
  rendezvous.meet()
  error('failed on purpose')
end
",
        ))
        .unwrap();
    for _ in 0..4 {
        domain.send_to_worker(worker, message.clone()).unwrap();
    }

    assert!(original.owners() >= baseline + 3);
    assert!(rendezvous.meet(), "the handler started");
    assert!(domain.receive_from_worker(worker, WAIT).is_err());
    assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Failed);
    assert_owners(original, baseline);
}

#[test]
fn a_failed_send_releases_its_message() {
    let harness = support::domain("failed send", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    let worker = domain
        .spawn_worker(WorkerSpec::new("ended", "return function() end"))
        .unwrap();
    assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Stopped);
    assert!(domain.send_to_worker(worker, message.clone()).is_err());
    assert_owners(original, baseline);
}

#[test]
fn a_refused_message_releases_what_it_had_read() {
    let harness = support::domain("refused", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    domain
        .with_runtime(|runtime| {
            runtime.exec("refused = { payload, function() end }")?;
            let value = runtime.global("refused")?;
            runtime.stack().with_frame(|frame| {
                value.with_value(frame, |frame, view| {
                    assert!(Message::read(frame, view, "refused").is_err());
                    Ok(())
                })
            })
        })
        .unwrap()
        .unwrap();
    assert_owners(original, baseline);
}

#[test]
fn stopping_a_domain_releases_replies_nobody_received() {
    let harness = support::domain("unread", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let worker = domain
        .spawn_worker(WorkerSpec::new(
            "unread",
            r"
local worker = require('@jess/worker')

return function(message)
  worker.send(message)
  worker.send(message)
  return message
end
",
        ))
        .unwrap();
    domain.send_to_worker(worker, message.clone()).unwrap();
    domain.stop().unwrap();
    assert_owners(original, 1);
}

#[test]
fn concurrent_workers_read_the_same_allocation() {
    let harness = support::domain("readers", &[]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let original = shared(&message);
    let baseline = original.owners();
    let workers: Vec<_> = (0..8)
        .map(|_| {
            domain
                .spawn_worker(WorkerSpec::new(
                    "reader",
                    r"
return function(view)
  local window, total = buffer.create(256), 0
  for _ = 1, 50 do
    for offset = 0, view.length - 256, 256 do
      view:copyInto(window, 0, offset, 256)
      total += buffer.readf64(window, 0) + view:readf64(offset + 8)
    end
  end

  return total
end
",
                ))
                .unwrap()
        })
        .collect();
    for &worker in &workers {
        domain.send_to_worker(worker, message.clone()).unwrap();
    }

    // Each 256-byte window starts on f64 index 32 * row, and the scalar read is the next one.
    let expected: f64 = (0..256u32).map(|row| f64::from(2 * 32 * row + 1)).sum::<f64>() * 50.0;
    for &worker in &workers {
        let reply = domain.receive_from_worker(worker, WAIT).unwrap().unwrap();
        assert_eq!(reply.as_number(), Some(expected));
    }

    for worker in workers {
        assert_eq!(domain.join_worker(worker, WAIT).unwrap(), WorkerState::Stopped);
    }

    assert_owners(original, baseline);
}

#[test]
fn allocating_needs_the_capability_and_receiving_does_not() {
    let harness = support::domain("allocation", &[SHARED_BUFFER_CREATE]);
    let domain = &harness.domain;
    let message = sealed(domain);
    let source = r"
return function(message)
  if message == 'create' then
    return require('@jess/sharedBuffer').create(8):seal().length
  end

  return message:readf64(8)
end
";
    let holder = domain.spawn_worker(WorkerSpec::new("holder", source)).unwrap();
    domain.send_to_worker(holder, Message::text("create")).unwrap();
    assert_eq!(
        domain.receive_from_worker(holder, WAIT).unwrap().unwrap().as_number(),
        Some(8.0)
    );

    let bare = domain
        .spawn_worker(WorkerSpec::new("bare", source).capabilities(CapabilitySet::empty()))
        .unwrap();
    domain.send_to_worker(bare, message).unwrap();
    assert_eq!(
        domain.receive_from_worker(bare, WAIT).unwrap().unwrap().as_number(),
        Some(1.0)
    );

    domain.send_to_worker(bare, Message::text("create")).unwrap();
    let refused = domain.receive_from_worker(bare, WAIT).unwrap_err().to_string();
    assert!(refused.contains(SHARED_BUFFER_CREATE), "{refused}");
}
