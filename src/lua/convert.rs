//! Luau tables to Rust values.
//!
//! Values cross as a [`serde_value`] tree, so every float survives, NaN and infinities included.
//! Field names are camelCase in Luau and `snake_case` in Rust. A missing value is `nil` in a
//! table's fields but `jess.null` inside an array, so arrays keep their positions. Tables that are
//! sequences (`1..n` keys) become arrays; an empty table is an array unless the value it replaces
//! was an object.

use std::collections::BTreeMap;

use l3i::convert::Vector3;
use l3i::memory::LightUserdata;
use l3i::stack::{Frame, TableView, ValueView};
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

/// A field name renamed; variant names, which start uppercase, are kept.
fn rename(key: &str, case: fn(&str) -> String) -> String {
    if key.starts_with(|first: char| first.is_ascii_uppercase()) {
        key.to_owned()
    } else {
        case(key)
    }
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
        count == length
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
