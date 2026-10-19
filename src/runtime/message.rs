//! Transferable values: what may cross a worker or event boundary, and how it converts to and
//! from Luau.
//!
//! The set is deliberately small: nil, booleans, numbers, Luau integers, strings (bytes),
//! vectors, byte buffers, lists and maps of those, and an explicit [`ResourceHandle`]. Nothing
//! else converts, and nothing unsupported is dropped quietly: [`TransferError`] names the path to
//! the offending value and what it was.
//!
//! Bytes travel as [`Message::Bytes`], an `Arc<[u8]>`: one copy out of the sending VM and one
//! copy into a fresh `buffer` in the receiving one, and no copy at all between native parties
//! that share the `Arc`. A zero-copy buffer shared between two VMs would need Luau to alias
//! external memory as a `buffer`, which l3i does not expose; that is the documented follow-up.

use std::fmt;
use std::sync::Arc;

use l3i::convert::{BufferView, Integer, NewBuffer, Push, Vector3};
use l3i::stack::{Frame, Scope, Type, ValueView};
use l3i::userdata::Userdata;

/// How deep a message may nest. Deeper is refused rather than walked: a self-referential table
/// would otherwise never end.
pub const MAX_DEPTH: usize = 32;

/// An explicit handle to a native resource that is safe to name from any domain or worker. The
/// runtime never dereferences it; the service that minted it does, and checks the kind.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ResourceHandle {
    /// What kind of resource, in the minting service's vocabulary (`texture`, `file-watch`).
    pub kind: String,
    /// The service's id for it.
    pub id: u64,
}

// SAFETY: plain data; the destructor never touches the Lua API.
unsafe impl Userdata for ResourceHandle {
    const NAME: &'static str = "jess.runtime.Handle";
}

/// A key of a [`Message::Map`]: strings (bytes) or numbers, the keys a Luau table can hold that
/// mean the same thing in every VM.
#[derive(Clone, PartialEq, Debug)]
pub enum MessageKey {
    /// A string key, as bytes.
    String(Vec<u8>),
    /// A number key.
    Number(f64),
}

/// A value that may cross a worker or event boundary.
#[derive(Clone, PartialEq, Debug, Default)]
pub enum Message {
    /// `nil`.
    #[default]
    Nil,
    /// A boolean.
    Boolean(bool),
    /// A Luau number.
    Number(f64),
    /// A Luau integer (`42i`), distinct from a number as Luau keeps it.
    Integer(i64),
    /// A Luau string: bytes, not necessarily UTF-8.
    String(Vec<u8>),
    /// A Luau vector.
    Vector([f32; 3]),
    /// A Luau `buffer`; arrives as a fresh buffer of the same bytes.
    Bytes(Arc<[u8]>),
    /// A table whose keys are exactly `1..=n`.
    List(Vec<Message>),
    /// Any other table, with string or number keys, in key order.
    Map(Vec<(MessageKey, Message)>),
    /// An explicit cross-domain handle.
    Handle(ResourceHandle),
}

impl Message {
    /// A string message from text.
    #[must_use]
    pub fn text(text: &str) -> Message {
        Message::String(text.as_bytes().to_vec())
    }

    /// A map message from string keys.
    #[must_use]
    pub fn map<K: AsRef<str>>(entries: impl IntoIterator<Item = (K, Message)>) -> Message {
        let mut entries: Vec<(MessageKey, Message)> = entries
            .into_iter()
            .map(|(key, value)| (MessageKey::String(key.as_ref().as_bytes().to_vec()), value))
            .collect();
        sort_entries(&mut entries);
        Message::Map(entries)
    }

    /// The value under the string key `key` of a map, if this is a map holding it.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Message> {
        let Message::Map(entries) = self else { return None };
        entries.iter().find_map(|(entry, value)| match entry {
            MessageKey::String(name) if name == key.as_bytes() => Some(value),
            _ => None,
        })
    }

    /// The string's text, if this is a UTF-8 string.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Message::String(bytes) => std::str::from_utf8(bytes).ok(),
            _ => None,
        }
    }

    /// The number, if this is one (an integer converts when exact).
    #[must_use]
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Message::Number(number) => Some(*number),
            // Exactness is checked by the round trip: an integer that survives it is a number.
            #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            Message::Integer(integer) if (*integer as f64) as i64 == *integer => Some(*integer as f64),
            _ => None,
        }
    }

    /// An estimate of the message's size in bytes, for message and byte counters: the payload
    /// bytes of strings and buffers, eight per scalar, and the sum over containers.
    #[must_use]
    pub fn size_hint(&self) -> usize {
        match self {
            Message::Nil => 0,
            Message::Boolean(_) | Message::Number(_) | Message::Integer(_) => 8,
            Message::Vector(_) => 12,
            Message::String(bytes) => bytes.len(),
            Message::Bytes(bytes) => bytes.len(),
            Message::List(items) => items.iter().map(Message::size_hint).sum(),
            Message::Map(entries) => entries
                .iter()
                .map(|(key, value)| {
                    let key = match key {
                        MessageKey::String(bytes) => bytes.len(),
                        MessageKey::Number(_) => 8,
                    };
                    key + value.size_hint()
                })
                .sum(),
            Message::Handle(handle) => handle.kind.len() + 8,
        }
    }

    /// Whether a buffer is anywhere in the message: the one value a frozen table doesn't make
    /// immutable.
    #[must_use]
    pub fn holds_bytes(&self) -> bool {
        match self {
            Message::Bytes(_) => true,
            Message::List(items) => items.iter().any(Message::holds_bytes),
            Message::Map(entries) => entries.iter().any(|(_, value)| value.holds_bytes()),
            _ => false,
        }
    }

    /// What the message weighs in a queue: the storage it holds, not only its payload. Every
    /// value costs a [`Message`]'s size, every element of a list or map a key's too, and strings,
    /// buffers, string keys, and handle kinds their bytes; the sum saturates. A list of a million empty tables weighs
    /// what it takes to hold them, where [`Message::size_hint`], a payload count for profiling,
    /// says nothing.
    #[must_use]
    pub fn queue_weight(&self) -> usize {
        let held = match self {
            Message::Nil | Message::Boolean(_) | Message::Number(_) | Message::Integer(_) | Message::Vector(_) => 0,
            Message::String(bytes) => bytes.len(),
            Message::Bytes(bytes) => bytes.len(),
            Message::List(items) => items.iter().fold(0, |total: usize, item| {
                total.saturating_add(ENTRY).saturating_add(item.queue_weight())
            }),
            Message::Map(entries) => entries.iter().fold(0, |total: usize, (key, value)| {
                total
                    .saturating_add(ENTRY)
                    .saturating_add(key_bytes(key))
                    .saturating_add(value.queue_weight())
            }),
            Message::Handle(handle) => handle.kind.len(),
        };
        NODE.saturating_add(held)
    }

    /// Reads the Luau value `view` as a message; `root` names it in errors (`message`,
    /// `reply`).
    ///
    /// # Errors
    ///
    /// [`TransferError`] naming the path to the first value that cannot cross.
    pub fn read(frame: &Frame<'_>, view: ValueView<'_>, root: &str) -> Result<Message, TransferError> {
        Message::read_within(frame, view, root, usize::MAX)
    }

    /// [`Message::read`], refusing a value that would weigh more than `limit` in a queue as soon as
    /// the read passes it, before the rest is copied: a table of ten million empty tables is
    /// refused a few thousand in, not built and then measured. The read counts each value as
    /// [`Message::queue_weight`] does, so what it admits never weighs more than `limit`.
    ///
    /// # Errors
    ///
    /// As [`Message::read`], and [`TransferError`] at the value where the read passed `limit`.
    pub fn read_within(frame: &Frame<'_>, view: ValueView<'_>, root: &str, limit: usize) -> Result<Message, TransferError> {
        let mut budget = Budget { left: limit, limit };
        read_at(frame, view, 0, &mut budget).map_err(|mut error| {
            error.path.insert_str(0, root);
            error
        })
    }
}

fn sort_entries(entries: &mut [(MessageKey, Message)]) {
    entries.sort_by(|(left, _), (right, _)| match (left, right) {
        (MessageKey::Number(left), MessageKey::Number(right)) => left.total_cmp(right),
        (MessageKey::Number(_), MessageKey::String(_)) => std::cmp::Ordering::Less,
        (MessageKey::String(_), MessageKey::Number(_)) => std::cmp::Ordering::Greater,
        (MessageKey::String(left), MessageKey::String(right)) => left.cmp(right),
    });
}

/// A value that cannot cross a boundary: where it was and what it was.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TransferError {
    /// The path from the root value: `message.items[3].callback`.
    pub path: String,
    /// What was found there: a Luau type name, or why a table or key does not convert.
    pub found: String,
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} can't cross a worker or event boundary", self.path, self.found)
    }
}

impl std::error::Error for TransferError {}

/// An error at the value being read; each table it is inside puts its key in front of the path
/// as the error comes back up, so a read that succeeds never formats one.
fn unsupported(found: impl Into<String>) -> TransferError {
    TransferError {
        path: String::new(),
        found: found.into(),
    }
}

/// What a [`Message`] costs a queue for being there, before what it holds.
const NODE: usize = size_of::<Message>();

/// What a map entry costs a queue besides its value: its key, before the key's bytes.
const ENTRY: usize = size_of::<MessageKey>();

fn key_bytes(key: &MessageKey) -> usize {
    match key {
        MessageKey::String(bytes) => bytes.len(),
        MessageKey::Number(_) => 0,
    }
}

/// What a read may still copy, in [`Message::queue_weight`]'s units.
struct Budget {
    left: usize,
    limit: usize,
}

impl Budget {
    fn charge(&mut self, weight: usize) -> Result<(), TransferError> {
        if weight > self.left {
            return Err(unsupported(format!(
                "a value past the {} bytes a queue holds of one payload",
                self.limit
            )));
        }
        self.left -= weight;
        Ok(())
    }
}

fn read_at(frame: &Frame<'_>, view: ValueView<'_>, depth: usize, budget: &mut Budget) -> Result<Message, TransferError> {
    if depth > MAX_DEPTH {
        return Err(unsupported(format!("a table nested more than {MAX_DEPTH} deep")));
    }
    budget.charge(NODE)?;
    let read_failed = |error: l3i::Error| unsupported(format!("a value l3i couldn't read ({error})"));
    match view.type_of() {
        Type::None | Type::Nil => Ok(Message::Nil),
        Type::Boolean => Ok(Message::Boolean(view.read::<bool>().map_err(read_failed)?)),
        Type::Integer => Ok(Message::Integer(view.read::<Integer>().map_err(read_failed)?.0)),
        Type::Number => Ok(Message::Number(view.read::<f64>().map_err(read_failed)?)),
        Type::String => {
            let bytes = view.read::<&[u8]>().map_err(read_failed)?;
            budget.charge(bytes.len())?;
            Ok(Message::String(bytes.to_vec()))
        }
        Type::Vector => {
            let vector = view.read::<Vector3>().map_err(read_failed)?;
            Ok(Message::Vector([vector.x, vector.y, vector.z]))
        }
        Type::Buffer => {
            let bytes = view.read::<BufferView<'_>>().map_err(read_failed)?;
            budget.charge(bytes.len())?;
            Ok(Message::Bytes(Arc::from(bytes.to_vec())))
        }
        Type::Table => read_table(frame, view, depth, budget),
        Type::Userdata => l3i::userdata::receiver::<ResourceHandle>(view).map_or_else(
            || Err(unsupported(format!("a {}", l3i::diagnostics::object_type_name(view)))),
            |handle| {
                budget.charge(handle.kind.len())?;
                Ok(Message::Handle(handle.clone()))
            },
        ),
        Type::Function => Err(unsupported("a function")),
        Type::Thread => Err(unsupported("a thread")),
        Type::LightUserdata => Err(unsupported("a light userdata")),
    }
}

/// The path segment of `key`: `.name` or `[3]`.
fn segment(key: &MessageKey) -> String {
    match key {
        MessageKey::String(bytes) => format!(".{}", String::from_utf8_lossy(bytes)),
        MessageKey::Number(number) => format!("[{}]", format_number(*number)),
    }
}

#[cfg(test)]
thread_local! {
    /// The table entries reads on this thread have visited, so a test can see a refused read
    /// stop walking.
    static VISITED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn read_table(frame: &Frame<'_>, view: ValueView<'_>, depth: usize, budget: &mut Budget) -> Result<Message, TransferError> {
    let table = view
        .as_table()
        .map_err(|error| unsupported(format!("a table l3i couldn't view ({error})")))?;
    let length = table.raw_len();
    // No more room up front than the budget could pay for: a table's length is the script's to
    // choose.
    let mut entries: Vec<(MessageKey, Message)> = Vec::with_capacity(length.min(budget.left / (ENTRY + NODE)));
    let mut array_keys = 0usize;
    let mut failure: Option<TransferError> = None;
    let walked = table.for_each(frame, |frame, key, value| {
        #[cfg(test)]
        VISITED.with(|visited| visited.set(visited.get() + 1));
        match read_entry(frame, key, value, depth, budget) {
            Ok((key, value)) => {
                #[allow(clippy::cast_precision_loss)] // Array indexes are far below 2^53.
                if let MessageKey::Number(number) = key
                    && number.fract() == 0.0
                    && number >= 1.0
                    && number <= length as f64
                {
                    array_keys += 1;
                }
                entries.push((key, value));
                Ok(())
            }
            Err(error) => {
                failure = Some(error);
                // Ends the walk here: the rest of a table that can't cross isn't visited.
                Err(l3i::Error::runtime("the table can't cross"))
            }
        }
    });
    if let Some(error) = failure {
        return Err(error);
    }
    walked.map_err(|error| unsupported(format!("a table l3i couldn't walk ({error})")))?;
    if length > 0 && array_keys == length && entries.len() == length {
        sort_entries(&mut entries);
        return Ok(Message::List(entries.into_iter().map(|(_, value)| value).collect()));
    }
    sort_entries(&mut entries);
    Ok(Message::Map(entries))
}

/// One entry of a table being read: its key, charged before its bytes are copied, and its value.
fn read_entry(
    frame: &Frame<'_>,
    key: ValueView<'_>,
    value: ValueView<'_>,
    depth: usize,
    budget: &mut Budget,
) -> Result<(MessageKey, Message), TransferError> {
    let unreadable = |error: l3i::Error| unsupported(format!("a key l3i couldn't read ({error})"));
    let key = match key.type_of() {
        Type::String => {
            let bytes = key.read::<&[u8]>().map_err(unreadable)?;
            budget.charge(ENTRY.saturating_add(bytes.len()))?;
            MessageKey::String(bytes.to_vec())
        }
        Type::Number => {
            let number = key.read::<f64>().map_err(unreadable)?;
            let key = MessageKey::Number(number);
            // A number key is cheap to name in the path; a string key isn't copied just to name it.
            budget.charge(ENTRY).map_err(|mut error| {
                error.path.insert_str(0, &segment(&key));
                error
            })?;
            key
        }
        other => return Err(unsupported(format!("a table with a {} key", other.name()))),
    };
    let value = read_at(frame, value, depth + 1, budget).map_err(|mut error| {
        error.path.insert_str(0, &segment(&key));
        error
    })?;
    Ok((key, value))
}

fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 {
        format!("{number:.0}")
    } else {
        number.to_string()
    }
}

/// Pushes `message` as one Luau value on `frame`.
///
/// # Errors
///
/// An l3i error if the VM cannot allocate the value or the handle type is not registered in it.
pub fn push_message(frame: &Frame<'_>, message: &Message) -> l3i::Result<()> {
    push(frame, message, false)
}

/// Pushes `message` as one Luau value on `frame`, every table in it frozen: for one payload many
/// handlers share, so none of them can change what the next one reads.
///
/// # Errors
///
/// As [`push_message`].
pub fn push_frozen_message(frame: &Frame<'_>, message: &Message) -> l3i::Result<()> {
    push(frame, message, true)
}

fn push(frame: &Frame<'_>, message: &Message, frozen: bool) -> l3i::Result<()> {
    match message {
        Message::Nil => {
            frame.push_nil()?;
        }
        Message::Boolean(value) => {
            frame.push_boolean(*value)?;
        }
        Message::Number(value) => {
            frame.push_number(*value)?;
        }
        Message::Integer(value) => {
            frame.push(&Integer(*value))?;
        }
        Message::String(bytes) => {
            frame.push(bytes.as_slice())?;
        }
        Message::Vector([x, y, z]) => {
            frame.push(&Vector3 { x: *x, y: *y, z: *z })?;
        }
        Message::Bytes(bytes) => {
            frame.push(&NewBuffer(bytes.to_vec()))?;
        }
        Message::List(items) => {
            let table = frame.push_table(items.len(), 0)?;
            for (index, item) in items.iter().enumerate() {
                push(frame, item, frozen)?;
                table.raw_set_index(frame, i64::try_from(index).unwrap_or(i64::MAX).saturating_add(1))?;
            }
            if frozen {
                table.set_read_only(true)?;
            }
        }
        Message::Map(entries) => {
            let table = frame.push_table(0, entries.len())?;
            for (key, value) in entries {
                push(frame, value, frozen)?;
                match key {
                    MessageKey::String(bytes) => table.raw_set(frame, &String::from_utf8_lossy(bytes))?,
                    MessageKey::Number(number) => table.raw_set_number_key(frame, *number)?,
                }
            }
            if frozen {
                table.set_read_only(true)?;
            }
        }
        Message::Handle(handle) => {
            l3i::userdata::push_owned(frame, handle.clone())?;
        }
    }
    Ok(())
}

impl Push for Message {
    /// Pushes the message as one value; a table is built in a temporary frame and handed up.
    fn push_into<'s, S: Scope>(&self, scope: &'s S) -> l3i::Result<ValueView<'s>> {
        let mut frame = scope.frame();
        push_message(&frame, self)?;
        frame.preserve_top_and_release();
        Ok(scope.top_value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use l3i::Runtime;

    fn round_trip(runtime: &Runtime, source: &str) -> Result<Message, TransferError> {
        let stack = runtime.stack();
        stack
            .with_frame(|frame| {
                let options = runtime.compile_options();
                let chunk = runtime.load(frame, "=message", &format!("return {source}"), &options)?;
                let value: l3i::value::Value = chunk.as_function()?.invoke(frame, ())?;
                value.with_value(frame, |frame, view| Ok(Message::read(frame, view, "message")))
            })
            .unwrap()
    }

    #[test]
    fn supported_values_convert_both_ways() {
        let runtime = Runtime::new().unwrap();
        let message = round_trip(
            &runtime,
            "{ 1, 2.5, 'two', true, buffer.fromstring('abc'), vector.create(1, 2, 3), { a = 1, b = { 'x' } }, [10] = 42i }",
        )
        .unwrap();
        let Message::Map(entries) = &message else {
            panic!("mixed keys make a map, got {message:?}")
        };
        assert_eq!(entries.len(), 8);
        assert_eq!(entries[0].1, Message::Number(1.0));
        assert_eq!(entries[1].1, Message::Number(2.5));
        assert_eq!(entries[2].1, Message::text("two"));
        assert_eq!(entries[3].1, Message::Boolean(true));
        assert_eq!(entries[4].1, Message::Bytes(Arc::from(b"abc".to_vec())));
        assert_eq!(entries[5].1, Message::Vector([1.0, 2.0, 3.0]));
        assert_eq!(entries[6].1.get("a"), Some(&Message::Number(1.0)));
        assert_eq!(entries[6].1.get("b"), Some(&Message::List(vec![Message::text("x")])));
        assert_eq!(entries[7], (MessageKey::Number(10.0), Message::Integer(42)));

        // Pushed back, everything reads the same, with the buffer a fresh buffer.
        let again = runtime
            .stack()
            .with_frame(|frame| {
                push_message(frame, &message)?;
                let view = frame.top_value();
                assert!(view.is_table());
                Ok(Message::read(frame, view, "message"))
            })
            .unwrap()
            .unwrap();
        assert_eq!(again, message);
        assert_eq!(
            round_trip(&runtime, "{ 'a', 'b', 'c' }").unwrap(),
            Message::List(vec![Message::text("a"), Message::text("b"), Message::text("c")])
        );
        assert_eq!(round_trip(&runtime, "nil").unwrap(), Message::Nil);
        assert_eq!(round_trip(&runtime, "{}").unwrap(), Message::Map(Vec::new()));
    }

    #[test]
    fn unsupported_values_name_their_path() {
        let runtime = Runtime::new().unwrap();
        let error = round_trip(&runtime, "{ items = { 1, 2, { callback = function() end } } }").unwrap_err();
        assert_eq!(error.path, "message.items[3].callback");
        assert_eq!(error.found, "a function");
        assert_eq!(
            error.to_string(),
            "message.items[3].callback: a function can't cross a worker or event boundary"
        );
        let error = round_trip(&runtime, "{ [true] = 1 }").unwrap_err();
        assert_eq!(error.found, "a table with a boolean key");
        let error = round_trip(&runtime, "coroutine.create(function() end)").unwrap_err();
        assert_eq!((error.path.as_str(), error.found.as_str()), ("message", "a thread"));
        let cyclic = round_trip(&runtime, "(function() local t = {} t.again = t return t end)()").unwrap_err();
        assert!(cyclic.found.contains("nested more than"), "{cyclic}");
        assert!(cyclic.path.starts_with("message.again.again"), "{cyclic}");
    }

    #[test]
    fn size_hints_count_payload_bytes() {
        let message = Message::map([
            ("name", Message::text("Fargoth")),
            ("bytes", Message::Bytes(Arc::from(vec![0u8; 100]))),
        ]);
        assert_eq!(message.size_hint(), 5 + 100 + 4 + 7);
        assert_eq!(Message::List(vec![Message::Number(1.0); 3]).size_hint(), 24);
        assert_eq!(Message::Integer(3).as_number(), Some(3.0));
        assert_eq!(Message::text("x").as_text(), Some("x"));
    }

    /// The queue weight counts storage: containers of nothing weigh what holding them takes,
    /// where the profiling size hint sees no payload at all.
    #[test]
    fn queue_weight_counts_every_value_held() {
        let empties = Message::List(vec![Message::Map(Vec::new()); 3]);
        assert_eq!(empties.size_hint(), 0);
        assert_eq!(empties.queue_weight(), NODE + 3 * (ENTRY + NODE));
        let keyed = Message::map([("name", Message::text("abc"))]);
        assert_eq!(keyed.queue_weight(), NODE + ENTRY + 4 + NODE + 3);
        let nils = Message::List(vec![Message::Nil; 1_000]);
        assert!(nils.queue_weight() > 1_000 * NODE);
    }

    /// A budgeted read stops where it passes the budget, at that value's path, and what it
    /// admits weighs what the read charged.
    #[test]
    fn a_budgeted_read_stops_at_its_limit() {
        let runtime = Runtime::new().unwrap();
        let read = |source: &str, limit: usize| {
            let stack = runtime.stack();
            stack
                .with_frame(|frame| {
                    let options = runtime.compile_options();
                    let chunk = runtime.load(frame, "=message", &format!("return {source}"), &options)?;
                    let value: l3i::value::Value = chunk.as_function()?.invoke(frame, ())?;
                    value.with_value(frame, |frame, view| Ok(Message::read_within(frame, view, "payload", limit)))
                })
                .unwrap()
        };
        let source = "(function() local t = {} for n = 1, 100 do t[n] = {} end return t end)()";
        let full = read(source, usize::MAX).unwrap();
        let weight = full.queue_weight();
        assert_eq!(read(source, weight).unwrap(), full);
        let refused = read(source, weight - 1).unwrap_err();
        assert_eq!(refused.path, "payload[100]");
        assert!(refused.found.contains(&format!("past the {} bytes", weight - 1)), "{refused}");
        let text = read("string.rep('x', 1000)", 500).unwrap_err();
        assert_eq!(text.path, "payload");
    }

    /// A read that passes its budget stops walking the table there, and a string key is charged
    /// before its bytes are copied.
    #[test]
    fn a_refused_read_stops_walking() {
        let runtime = Runtime::new().unwrap();
        let read = |source: &str, limit: usize| {
            let stack = runtime.stack();
            stack
                .with_frame(|frame| {
                    let options = runtime.compile_options();
                    let chunk = runtime.load(frame, "=message", &format!("return {source}"), &options)?;
                    let value: l3i::value::Value = chunk.as_function()?.invoke(frame, ())?;
                    VISITED.with(|visited| visited.set(0));
                    value.with_value(frame, |frame, view| Ok(Message::read_within(frame, view, "payload", limit)))
                })
                .unwrap()
        };
        let vast = "(function() local t = {} for n = 1, 100000 do t[n] = {} end return t end)()";
        assert!(read(vast, 10 * (NODE + ENTRY)).is_err());
        let visited = VISITED.with(std::cell::Cell::get);
        assert!(visited < 20, "visited {visited} entries of a refused table");

        let keyed = "{ [string.rep('k', 100000)] = true }";
        let refused = read(keyed, 1_000).unwrap_err();
        assert_eq!(refused.path, "payload", "refused at the table, before the key is copied");
        assert!(refused.found.contains("past the 1000 bytes"), "{refused}");
    }
}
