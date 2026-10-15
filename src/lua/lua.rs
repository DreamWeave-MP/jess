//! The shapes jess's bindings share: option tables, coerced text, and pushing tables and values
//! onto a call's stack.

use l3i::bind::Binding;
use l3i::convert::{FromView, Integer};
use l3i::stack::{Frame, Scope, TableView, Type, ValueView};
use l3i::value::{Function, Table, Value};

use crate::lua::error;

/// A value's type as Luau's `type()` names it, for messages: a whole number is a `number` and
/// `jess.null` a `userdata`, which is what scripts see.
pub fn kind(value: ValueView<'_>) -> &'static str {
    match value.type_of() {
        Type::Integer | Type::Number => "number",
        Type::LightUserdata | Type::Userdata => "userdata",
        Type::None | Type::Nil => "nil",
        other => other.name(),
    }
}

/// A number as Luau's `tostring` spells it (`%.14g`): whole numbers without a fraction, and
/// 14 significant digits otherwise.
pub fn number_text(number: f64) -> String {
    if number.is_nan() {
        return "nan".to_owned();
    }
    if number.is_infinite() {
        return if number > 0.0 { "inf".to_owned() } else { "-inf".to_owned() };
    }
    if number == 0.0 {
        return if number.is_sign_negative() {
            "-0".to_owned()
        } else {
            "0".to_owned()
        };
    }
    // `%.14g`: exponential notation when the exponent is below -4 or at least 14, else plain,
    // both with trailing zeros trimmed.
    let scientific = format!("{number:.13e}");
    let (mantissa, exponent) = scientific.split_once('e').expect("Rust's `e` format has an exponent");
    let exponent: i32 = exponent.parse().expect("Rust's `e` format has a numeric exponent");
    let trim = |digits: &str| {
        if digits.contains('.') {
            digits.trim_end_matches('0').trim_end_matches('.').to_owned()
        } else {
            digits.to_owned()
        }
    };
    if !(-4..14).contains(&exponent) {
        let sign = if exponent < 0 { '-' } else { '+' };
        return format!("{}e{sign}{:02}", trim(mantissa), exponent.abs());
    }
    let decimals = usize::try_from(13 - exponent).unwrap_or(0);
    trim(&format!("{number:.decimals$}"))
}

/// A string argument the way Luau's string functions take one: a string, or a number as its text.
pub struct Text(pub String);

impl<'v> FromView<'v> for Text {
    const EXPECTED: &'static str = "string";

    fn from_view(view: ValueView<'v>) -> l3i::Result<Self> {
        if view.is_string() {
            return view.read::<&str>().map(|text| Self(text.to_owned()));
        }
        if view.is_integer() {
            return view.read::<Integer>().map(|integer| Self(integer.0.to_string()));
        }
        if view.is_number() {
            return view.read::<f64>().map(|number| Self(number_text(number)));
        }
        Err(view.type_error(Type::String))
    }

    fn matches(view: ValueView<'v>) -> bool {
        view.is_string() || view.is_number() || view.is_integer()
    }
}

impl l3i::bind::Param for Text {
    type Item<'c> = Text;
}

impl<'c> l3i::bind::ParamItem<'c> for Text {
    const KIND: l3i::bind::ParamKind = l3i::bind::ParamKind::Regular;
    const EXPECTED: &'static str = <Text as FromView<'c>>::EXPECTED;

    fn read_slot(view: ValueView<'c>) -> l3i::Result<Self> {
        <Text as FromView<'c>>::from_view(view)
    }

    fn matches(view: ValueView<'c>) -> bool {
        <Text as FromView<'c>>::matches(view)
    }
}

/// Bytes from a buffer or a string, in jess's own words when it's neither.
pub struct Bytes(pub Vec<u8>);

impl<'v> FromView<'v> for Bytes {
    const EXPECTED: &'static str = "buffer or string";

    fn from_view(view: ValueView<'v>) -> l3i::Result<Self> {
        if view.is_buffer() {
            return Ok(Self(view.read::<l3i::convert::BufferView<'_>>()?.to_vec()));
        }
        if view.is_string() {
            return Ok(Self(view.read::<&[u8]>()?.to_vec()));
        }
        Err(error(format!("expected a buffer or string, not {}", kind(view))))
    }

    fn matches(view: ValueView<'v>) -> bool {
        view.is_buffer() || view.is_string()
    }
}

impl l3i::bind::Param for Bytes {
    type Item<'c> = Bytes;
}

impl<'c> l3i::bind::ParamItem<'c> for Bytes {
    const KIND: l3i::bind::ParamKind = l3i::bind::ParamKind::Regular;
    const EXPECTED: &'static str = <Bytes as FromView<'c>>::EXPECTED;

    fn read_slot(view: ValueView<'c>) -> l3i::Result<Self> {
        <Bytes as FromView<'c>>::from_view(view)
    }

    fn matches(view: ValueView<'c>) -> bool {
        <Bytes as FromView<'c>>::matches(view)
    }
}

/// An option table argument, or none.
pub struct Opts(Option<Table>);

impl<'v> FromView<'v> for Opts {
    const EXPECTED: &'static str = "table";

    fn from_view(view: ValueView<'v>) -> l3i::Result<Self> {
        if view.is_nil() {
            return Ok(Self(None));
        }
        Table::from_value(Value::store(view)?).map(|table| Self(Some(table)))
    }

    fn matches(view: ValueView<'v>) -> bool {
        view.is_nil() || view.is_table()
    }
}

impl l3i::bind::Param for Opts {
    type Item<'c> = Opts;
}

/// An option table is optional: left out, or `nil`, it's no options.
impl<'c> l3i::bind::ParamItem<'c> for Opts {
    const KIND: l3i::bind::ParamKind = l3i::bind::ParamKind::Optional;
    const EXPECTED: &'static str = <Opts as FromView<'c>>::EXPECTED;

    fn read_slot(view: ValueView<'c>) -> l3i::Result<Self> {
        <Opts as FromView<'c>>::from_view(view)
    }

    fn matches(view: ValueView<'c>) -> bool {
        <Opts as FromView<'c>>::matches(view)
    }

    fn probe(call: &'c l3i::bind::Call<'c>, cursor: &mut std::ffi::c_int, top: std::ffi::c_int, _: bool) -> bool {
        if *cursor > top {
            return true;
        }
        if !<Self as FromView<'c>>::matches(call.arg(*cursor)) {
            return false;
        }
        *cursor += 1;
        true
    }

    fn materialize(
        call: &'c l3i::bind::Call<'c>,
        cursor: &mut std::ffi::c_int,
        top: std::ffi::c_int,
        position: &mut std::ffi::c_int,
        debug_name: &str,
        _: bool,
    ) -> l3i::Result<Self> {
        if *cursor > top {
            return Ok(Self(None));
        }
        *position += 1;
        let value = call.arg(*cursor);
        let options = <Self as FromView<'c>>::from_view(value)
            .map_err(|cause| error(format!("{debug_name}: bad argument #{position} (expected table): {cause}")))?;
        *cursor += 1;
        Ok(options)
    }
}

impl From<Table> for Opts {
    fn from(table: Table) -> Self {
        Self(Some(table))
    }
}

impl Opts {
    /// Rejects keys `function` doesn't take, naming the ones it does.
    pub(crate) fn check(&self, scope: &impl Scope, function: &str, keys: &[&str]) -> l3i::Result<()> {
        let Some(table) = &self.0 else {
            return Ok(());
        };
        scope.with_frame(|frame| {
            let view = table.push_to(frame)?;
            crate::lua::check_options(frame, view.value(), function, keys)
        })
    }

    /// Runs `body` with the option `key`'s value on a fresh frame, `nil` when the table or the key
    /// is missing.
    pub(crate) fn with<R>(
        &self,
        scope: &impl Scope,
        key: &str,
        body: impl FnOnce(&Frame<'_>, ValueView<'_>) -> l3i::Result<R>,
    ) -> l3i::Result<R> {
        scope.with_frame(|frame| match &self.0 {
            Some(table) => {
                let view = table.push_to(frame)?;
                let value = view.raw_get(frame, key)?;
                body(frame, value)
            }
            None => body(frame, frame.push_nil()?),
        })
    }

    /// The option `key` converted to `T`, `None` when it's missing.
    pub(crate) fn get<T: for<'a> FromView<'a>>(&self, scope: &impl Scope, key: &str) -> l3i::Result<Option<T>> {
        self.with(scope, key, |_, value| {
            if value.is_nil() {
                return Ok(None);
            }
            value.read::<T>().map(Some)
        })
    }
}

/// The strings of a list argument, each a string or a number as its text; `what` names the list
/// in errors.
pub fn string_list(frame: &Frame<'_>, list: ValueView<'_>, what: &str) -> l3i::Result<Vec<String>> {
    if !list.is_table() {
        return Err(error(format!("{what} must be a list of strings, not {}", kind(list))));
    }
    let mut strings = Vec::new();
    list.as_table()?.for_each_array(frame, |_, _, item| {
        if !Text::matches(item) {
            return Err(error(format!(
                "{what} must be a list of strings, not a list with a {}",
                kind(item)
            )));
        }
        strings.push(Text::from_view(item)?.0);
        Ok(())
    })?;
    Ok(strings)
}

/// Runs `build`, which leaves one value on top of a fresh frame, and keeps that value on `scope`:
/// a bound function's result, or the next value of a table under construction.
pub fn keep_top(scope: &impl Scope, build: impl FnOnce(&Frame<'_>) -> l3i::Result<()>) -> l3i::Result<()> {
    let mut frame = scope.frame();
    build(&frame)?;
    frame.preserve_top_and_release();
    Ok(())
}

/// Pushes a new table onto `scope`, filled by `fill` with the table on a frame of its own.
pub fn push_table(
    scope: &impl Scope,
    array_capacity: usize,
    hash_capacity: usize,
    fill: impl FnOnce(&Frame<'_>, &TableView<'_>) -> l3i::Result<()>,
) -> l3i::Result<()> {
    keep_top(scope, |frame| {
        let table = frame.push_table(array_capacity, hash_capacity)?;
        fill(frame, &table)
    })
}

/// Pushes a list of `items` onto `scope`, each pushed by `push`.
pub fn push_list<T>(
    scope: &impl Scope,
    items: impl ExactSizeIterator<Item = T>,
    mut push: impl FnMut(&Frame<'_>, T) -> l3i::Result<()>,
) -> l3i::Result<()> {
    push_table(scope, items.len(), 0, |frame, table| {
        for (index, item) in items.enumerate() {
            push(frame, item)?;
            table.raw_set_index(frame, index as i64 + 1)?;
        }
        Ok(())
    })
}

/// A table `build` pushed onto a frame, pinned.
pub fn stored_table(scope: &impl Scope, build: impl FnOnce(&Frame<'_>) -> l3i::Result<()>) -> l3i::Result<Table> {
    Table::from_value(crate::lua::stored(scope, build)?)
}

/// A Lua function made inside a call, for iterators: `next` is called by `for` with the state and
/// control it ignores.
pub fn function<F: Binding<M>, M>(scope: &impl Scope, name: &str, callable: F) -> l3i::Result<Function> {
    l3i::bind::function(scope, &["jess"], name, callable)
}

/// Sets `metatable` (on top of `frame`, popped) as the metatable of the table at `table`.
#[allow(ffi_unwind_calls)] // Neither call can raise: the table and metatable are on the stack.
pub fn set_metatable(frame: &Frame<'_>, table: &TableView<'_>) {
    // SAFETY: the table slot exists on this frame, and the metatable is the value on top, which
    // `lua_setmetatable` pops; neither can raise.
    unsafe {
        l3i::ffi::lua_setmetatable(Scope::state(frame), table.index());
    }
}

/// Whether the table at `table` has `marker` as its metatable.
#[allow(ffi_unwind_calls)] // Reads two existing slots; nothing can raise.
pub fn has_metatable(frame: &Frame<'_>, table: &TableView<'_>, marker: &Table) -> l3i::Result<bool> {
    frame.with_frame(|frame| {
        let marker = marker.push_to(frame)?;
        // SAFETY: the table slot exists; `lua_getmetatable` pushes the metatable or nothing and
        // says which, and the comparison reads two slots on this frame.
        unsafe {
            let state = Scope::state(frame);
            if l3i::ffi::lua_getmetatable(state, table.index()) == 0 {
                return Ok(false);
            }
            Ok(l3i::ffi::lua_rawequal(state, marker.index(), -1) != 0)
        }
    })
}

/// `table[key] = value` with the key and value the two values on top of `frame` (popped), for
/// keys that aren't strings or numbers.
#[allow(ffi_unwind_calls)] // A raw set into a table jess just made can't raise.
pub fn raw_set_pair(frame: &Frame<'_>, table: &TableView<'_>) {
    // SAFETY: the table slot exists below the key and value `lua_rawset` pops; the table isn't
    // read-only, since jess just made it.
    unsafe {
        l3i::ffi::lua_rawset(Scope::state(frame), table.index());
    }
}
