//! Rust values to Luau tables and back.
//!
//! Values cross as a [`serde_value`] tree, so every float survives, NaN and infinities included.
//! Service field names are renamed reversibly between `snake_case` and camelCase. A missing value
//! is `nil` in a table's fields but `jess.null` inside an array, so arrays keep their positions.
//! Tables that are sequences (`1..n` keys), or carry the array marker jess's own lists get, become
//! arrays; an empty table is an array unless the value it replaces was an object.

use std::collections::BTreeMap;

use l3i::convert::Vector3;
use l3i::memory::LightUserdata;
use l3i::stack::{Frame, Scope, TableView, ValueView};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;
use serde_value::Value as Tree;

use crate::lua::{Jess, error, kind, lua};

/// `jess.null`: a null light userdata, equal to itself and to nothing else, and a `userdata` to
/// `type()`.
pub const NULL: LightUserdata = LightUserdata {
    pointer: std::ptr::null_mut(),
    tag: 0,
};

/// Whether `value` is `jess.null`.
pub fn is_null(value: ValueView<'_>) -> bool {
    value.is_light_userdata() && value.read::<LightUserdata>().is_ok_and(|light| light.pointer.is_null())
}

/// Whether a tree value stands for a missing value: `None`, or nothing.
fn is_missing(value: &Tree) -> bool {
    matches!(value, Tree::Option(None) | Tree::Unit)
}

/// Pushes `value` onto `scope` as a Luau value with camelCase field names and codes. A value
/// that is one scalar (a field read's number or string) is pushed straight from its `Serialize`;
/// only a table goes through a tree.
pub fn to_lua(scope: &impl Scope, jess: &Jess, value: &impl Serialize) -> l3i::Result<()> {
    lua::keep_top(scope, |frame| {
        if push_leaf(frame, value)? {
            return Ok(());
        }
        let tree = serde_value::to_value(value).map_err(error)?;
        push_tree(frame, jess, &tree)
    })
}

/// Pushes a value that serializes to one Luau scalar (a number, boolean, string, bytes, or a
/// missing value as `jess.null`) without building a tree; `Ok(false)`, having pushed nothing, for
/// anything else.
fn push_leaf(frame: &Frame<'_>, value: &(impl Serialize + ?Sized)) -> l3i::Result<bool> {
    match value.serialize(LeafPusher { frame }) {
        Ok(()) => Ok(true),
        Err(NotLeaf::Compound) => Ok(false),
        Err(NotLeaf::Failed(message)) => Err(error(message)),
    }
}

/// Why a value didn't push as a leaf: it's a table, or its `Serialize` failed.
#[derive(Debug)]
enum NotLeaf {
    Compound,
    Failed(String),
}

impl std::fmt::Display for NotLeaf {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Compound => formatter.write_str("not a leaf"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for NotLeaf {}

impl serde::ser::Error for NotLeaf {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self::Failed(message.to_string())
    }
}

/// A serializer that pushes one scalar onto a frame and refuses every compound value before
/// pushing anything, so a refusal leaves the frame as it was.
struct LeafPusher<'f, 'p> {
    frame: &'f Frame<'p>,
}

#[allow(clippy::cast_precision_loss)] // Luau numbers are doubles; wide integers round as they always did.
impl serde::Serializer for LeafPusher<'_, '_> {
    type Ok = ();
    type Error = NotLeaf;
    type SerializeSeq = serde::ser::Impossible<(), NotLeaf>;
    type SerializeTuple = serde::ser::Impossible<(), NotLeaf>;
    type SerializeTupleStruct = serde::ser::Impossible<(), NotLeaf>;
    type SerializeTupleVariant = serde::ser::Impossible<(), NotLeaf>;
    type SerializeMap = serde::ser::Impossible<(), NotLeaf>;
    type SerializeStruct = serde::ser::Impossible<(), NotLeaf>;
    type SerializeStructVariant = serde::ser::Impossible<(), NotLeaf>;

    fn serialize_bool(self, value: bool) -> Result<(), NotLeaf> {
        self.frame.push_boolean(value);
        Ok(())
    }

    fn serialize_i8(self, value: i8) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_i16(self, value: i16) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_i32(self, value: i32) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_i64(self, value: i64) -> Result<(), NotLeaf> {
        self.serialize_f64(value as f64)
    }

    fn serialize_u8(self, value: u8) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_u16(self, value: u16) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_u32(self, value: u32) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_u64(self, value: u64) -> Result<(), NotLeaf> {
        self.serialize_f64(value as f64)
    }

    fn serialize_f32(self, value: f32) -> Result<(), NotLeaf> {
        self.serialize_f64(f64::from(value))
    }

    fn serialize_f64(self, value: f64) -> Result<(), NotLeaf> {
        self.frame.push_number(value);
        Ok(())
    }

    fn serialize_char(self, value: char) -> Result<(), NotLeaf> {
        self.frame.push_string(&value.to_string());
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<(), NotLeaf> {
        self.frame.push_string(value);
        Ok(())
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<(), NotLeaf> {
        self.frame
            .push(value)
            .map(|_| ())
            .map_err(|problem| NotLeaf::Failed(problem.to_string()))
    }

    fn serialize_none(self) -> Result<(), NotLeaf> {
        self.serialize_unit()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), NotLeaf> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), NotLeaf> {
        self.frame
            .push(&NULL)
            .map(|_| ())
            .map_err(|problem| NotLeaf::Failed(problem.to_string()))
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), NotLeaf> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(self, _: &'static str, _: u32, variant: &'static str) -> Result<(), NotLeaf> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _: &'static str, value: &T) -> Result<(), NotLeaf> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: &T,
    ) -> Result<(), NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeTupleStruct, NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Self::SerializeStruct, NotLeaf> {
        Err(NotLeaf::Compound)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, NotLeaf> {
        Err(NotLeaf::Compound)
    }
}

/// A field name renamed; variant names, which start uppercase, are kept.
fn rename(key: &str, case: fn(&str) -> String) -> String {
    if key.starts_with(|first: char| first.is_ascii_uppercase()) {
        key.to_owned()
    } else {
        case(key)
    }
}

/// Pushes a tree onto `frame`: tables for maps and sequences (the latter marked as jess's arrays),
/// `jess.null` for a missing value in an array or as a table's only field (an enum variant holding
/// nothing, like `{ internal = jess.null }`), and nothing at all for other missing fields.
#[allow(clippy::cast_precision_loss)] // Luau numbers are doubles; wide integers round as they always did.
pub fn push_tree(frame: &Frame<'_>, jess: &Jess, tree: &Tree) -> l3i::Result<()> {
    match tree {
        Tree::Bool(value) => {
            frame.push_boolean(*value);
        }
        Tree::U8(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::U16(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::U32(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::U64(value) => {
            frame.push_number(*value as f64);
        }
        Tree::I8(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::I16(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::I32(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::I64(value) => {
            frame.push_number(*value as f64);
        }
        Tree::F32(value) => {
            frame.push_number(f64::from(*value));
        }
        Tree::F64(value) => {
            frame.push_number(*value);
        }
        Tree::Char(value) => {
            frame.push_string(&value.to_string());
        }
        Tree::String(value) => {
            frame.push_string(value);
        }
        Tree::Bytes(bytes) => {
            frame.push(bytes.as_slice())?;
        }
        Tree::Unit | Tree::Option(None) => {
            frame.push(&NULL)?;
        }
        Tree::Option(Some(inner)) | Tree::Newtype(inner) => push_tree(frame, jess, inner)?,
        Tree::Seq(items) => {
            let table = frame.push_table(items.len(), 0)?;
            for (index, item) in items.iter().enumerate() {
                push_tree(frame, jess, item)?;
                table.raw_set_index(frame, index as i64 + 1)?;
            }
            jess.array_marker.push_to(frame)?;
            lua::set_metatable(frame, &table);
        }
        Tree::Map(entries) => {
            let table = frame.push_table(0, entries.len())?;
            let keep_null = entries.len() == 1;
            for (key, value) in entries {
                if !keep_null && is_missing(value) {
                    continue;
                }
                match key {
                    Tree::String(name) => {
                        push_tree(frame, jess, value)?;
                        table.raw_set(frame, &rename(name, camel_case))?;
                    }
                    other => {
                        push_tree(frame, jess, other)?;
                        push_tree(frame, jess, value)?;
                        lua::raw_set_pair(frame, &table);
                    }
                }
            }
        }
    }
    Ok(())
}

/// Pushes JSON service data with camelCase field names.
pub fn display_to_lua(scope: &impl Scope, json: &Json) -> l3i::Result<()> {
    lua::keep_top(scope, |frame| push_json(frame, json, true))
}

/// Pushes a JSON value, keys exactly as they are (display values, whose keys can be Lua table
/// keys or record tags), or camelCase when `rename_keys`.
fn push_json(frame: &Frame<'_>, json: &Json, rename_keys: bool) -> l3i::Result<()> {
    match json {
        Json::Null => {
            frame.push_nil();
        }
        Json::Bool(value) => {
            frame.push_boolean(*value);
        }
        Json::Number(number) => {
            frame.push_number(number.as_f64().unwrap_or(f64::NAN));
        }
        Json::String(text) => {
            frame.push_string(text);
        }
        Json::Array(items) => {
            let table = frame.push_table(items.len(), 0)?;
            for (index, item) in items.iter().enumerate() {
                push_json(frame, item, rename_keys)?;
                table.raw_set_index(frame, index as i64 + 1)?;
            }
        }
        Json::Object(fields) => {
            let table = frame.push_table(0, fields.len())?;
            for (key, value) in fields {
                push_json(frame, value, rename_keys)?;
                if rename_keys {
                    table.raw_set(frame, &rename(key, camel_case))?;
                } else {
                    table.raw_set(frame, key)?;
                }
            }
        }
    }
    Ok(())
}

/// Reads a script's value as a tree ready to deserialize: field names `snake_case`, whole numbers
/// as integers so integer fields accept them, arrays as sequences, and `jess.null` as a missing
/// value. `shape` is the value it replaces, if any, which decides whether empty tables are arrays
/// or objects.
pub fn read_tree(frame: &Frame<'_>, jess: &Jess, value: ValueView<'_>, shape: Option<&Json>) -> l3i::Result<Tree> {
    if value.is_nil() {
        return Ok(Tree::Unit);
    }
    if is_null(value) {
        return Ok(Tree::Option(None));
    }
    if value.is_boolean() {
        return Ok(Tree::Bool(value.read::<bool>()?));
    }
    if value.is_integer() {
        return Ok(Tree::I64(value.read::<l3i::convert::Integer>()?.0));
    }
    if value.is_number() {
        return Ok(number_tree(value.read::<f64>()?));
    }
    if value.is_string() {
        return Ok(match value.read::<&str>() {
            Ok(text) => Tree::String(text.to_owned()),
            Err(_) => Tree::Bytes(value.read::<&[u8]>()?.to_vec()),
        });
    }
    if value.is_buffer() {
        return Ok(Tree::Bytes(value.read::<l3i::convert::BufferView<'_>>()?.to_vec()));
    }
    if value.is_vector() {
        let vector = value.read::<Vector3>()?;
        return Ok(Tree::Seq(
            [vector.x, vector.y, vector.z]
                .into_iter()
                .map(|component| number_tree(f64::from(component)))
                .collect(),
        ));
    }
    if value.is_table() {
        return table_tree(frame, jess, &value.as_table()?, shape);
    }
    Err(error(format!("a {} can't be converted to a service value", kind(value))))
}

fn table_tree(frame: &Frame<'_>, jess: &Jess, table: &TableView<'_>, shape: Option<&Json>) -> l3i::Result<Tree> {
    let length = table.raw_len();
    let mut count = 0usize;
    table.for_each(frame, |_, _, _| {
        count += 1;
        Ok(())
    })?;
    let array = if count == 0 {
        !matches!(shape, Some(Json::Object(_)))
    } else {
        count == length || lua::has_metatable(frame, table, &jess.array_marker)?
    };
    if array {
        let element = match shape {
            Some(Json::Array(items)) => items.first(),
            _ => None,
        };
        let mut items = Vec::with_capacity(length);
        table.for_each_array(frame, |frame, _, item| {
            items.push(read_tree(frame, jess, item, element)?);
            Ok(())
        })?;
        return Ok(Tree::Seq(items));
    }
    let mut entries = BTreeMap::new();
    table.for_each(frame, |frame, key, value| {
        let key = if key.is_string() {
            rename(key.read::<&str>()?, snake_case)
        } else if key.is_integer() {
            key.read::<l3i::convert::Integer>()?.0.to_string()
        } else if key.is_number() {
            lua::number_text(key.read::<f64>()?)
        } else {
            return Err(error(format!("table keys must be strings, not {}", kind(key))));
        };
        let field_shape = match shape {
            Some(Json::Object(shape)) => shape.get(&key),
            _ => None,
        };
        let value = read_tree(frame, jess, value, field_shape)?;
        entries.insert(Tree::String(key), value);
        Ok(())
    })?;
    Ok(Tree::Map(entries))
}

/// `number` as an integer when it's a whole number an `f64` holds exactly, else as it is. Negative
/// zero stays a float, so float fields keep its sign.
fn number_tree(number: f64) -> Tree {
    const MAX_EXACT: f64 = 9_007_199_254_740_992.0;
    #[allow(clippy::cast_possible_truncation)]
    if number.fract() == 0.0 && number.abs() <= MAX_EXACT && !(number == 0.0 && number.is_sign_negative()) {
        Tree::I64(number as i64)
    } else {
        Tree::F64(number)
    }
}

/// Deserializes a tree read from a script. A field the type doesn't have is an error.
pub fn from_tree<T: DeserializeOwned + Serialize>(tree: &Tree) -> l3i::Result<T> {
    let result: T = T::deserialize(deserializer(tree)).map_err(field_error)?;
    check_fields(tree, &serde_json::to_value(&result).map_err(error)?)?;
    Ok(result)
}

/// Deserializes a script's value with camelCase field names. `shape` is the value it replaces, if
/// any, which decides whether empty tables are arrays or objects. A field the type doesn't have
/// is an error.
pub fn from_lua<T: DeserializeOwned + Serialize>(
    frame: &Frame<'_>,
    jess: &Jess,
    value: ValueView<'_>,
    shape: Option<&Json>,
) -> l3i::Result<T> {
    from_tree(&read_tree(frame, jess, value, shape)?)
}

/// Errors for the fields of `input`, a tree read from a script, that `output` (what it
/// deserialized to, serialized) doesn't have: names the type doesn't know, which serde would skip.
pub fn check_fields(input: &Tree, output: &Json) -> l3i::Result<()> {
    check_fields_at(input, output, &mut Vec::new())
}

fn check_fields_at(input: &Tree, output: &Json, path: &mut Vec<String>) -> l3i::Result<()> {
    match (input, output) {
        (Tree::Map(entries), Json::Object(fields)) => {
            for (key, value) in entries {
                // `jess.null` leaves an optional field out, so it isn't in the output either.
                let (Tree::String(key), false) = (key, is_missing(value)) else {
                    continue;
                };
                path.push(rename(key, camel_case));
                match fields.get(key) {
                    Some(field) => check_fields_at(value, field, path)?,
                    None => return Err(error(format!("unknown field '{}'", field_path(path)))),
                }
                path.pop();
            }
        }
        (Tree::Seq(items), Json::Array(fields)) => {
            for (index, (item, field)) in items.iter().zip(fields).enumerate() {
                path.push(format!("[{}]", index + 1));
                check_fields_at(item, field, path)?;
                path.pop();
            }
        }
        (Tree::Option(Some(inner)) | Tree::Newtype(inner), output) => check_fields_at(inner, output, path)?,
        _ => {}
    }
    Ok(())
}

/// `data.color[2]` from its parts.
fn field_path(path: &[String]) -> String {
    let mut out = String::new();
    for part in path {
        if !out.is_empty() && !part.starts_with('[') {
            out.push('.');
        }
        out.push_str(part);
    }
    out
}

/// What deserializing a script's value reports, in serde's own words (such as: missing field
/// \`weight\`): the error type the tree deserializer raises.
#[derive(Debug)]
struct DeserializeError(String);

impl std::fmt::Display for DeserializeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DeserializeError {}

impl serde::de::Error for DeserializeError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

/// A deserializer over a tree read from a script.
fn deserializer(tree: &Tree) -> serde_value::ValueDeserializer<DeserializeError> {
    serde_value::ValueDeserializer::new(tree.clone())
}

/// A deserialize error as a script sees it: field names camelCase.
#[allow(clippy::needless_pass_by_value)] // Used as `map_err(field_error)`.
fn field_error(problem: DeserializeError) -> l3i::Error {
    error(camel_case_message(&problem.0))
}

/// serde's messages name fields in backticks, `snake_case` like the Rust fields; scripts spell
/// them camelCase. Variant names, which start uppercase, stay, and a `deserialize error: ` prefix
/// goes.
pub fn camel_case_message(message: &str) -> String {
    let mut message = message;
    while let Some(rest) = message.strip_prefix("deserialize error: ") {
        message = rest;
    }
    let mut out = String::with_capacity(message.len());
    let mut parts = message.split('`');
    out.push_str(parts.next().unwrap_or_default());
    for (index, part) in parts.enumerate() {
        out.push('`');
        if index % 2 == 0 && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            out.push_str(&rename(part, camel_case));
        } else {
            out.push_str(part);
        }
    }
    out
}

fn camel_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    while let Some(character) = chars.next() {
        match chars.peek() {
            Some(next) if matches!(character, '_' | '-') && next.is_ascii_lowercase() && !out.is_empty() => {
                out.push(next.to_ascii_uppercase());
                chars.next();
            }
            _ => out.push(character),
        }
    }
    out
}

fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for character in name.chars() {
        if character.is_ascii_uppercase() && !out.is_empty() {
            out.push('_');
            out.push(character.to_ascii_lowercase());
        } else {
            out.push(character);
        }
    }
    out
}
