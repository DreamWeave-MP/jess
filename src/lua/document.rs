//! One document in YAML, JSON, or TOML, written by the built-in package's `@jess/data/value`: the
//! one writer of those formats, which the CLI prints through instead of serde writers of its own.
//!
//! A value arrives as a `serde_json::Value`, whose maps keep their keys in the order they were
//! serialized (the field order of the struct behind them), and becomes the value model's: an
//! object a `struct` in that order, an array an `array`, a fractional number a `float`. A `null`
//! that is an object's field is an absent field, which TOML leaves out and JSON and YAML print as
//! `null`, as serde's writers treat a `None`; a `null` in an array is the null itself.

use l3i::Runtime;
use l3i::stack::Scope;
use l3i::value::{Function, Table, Value};
use serde_json::Value as Json;

use crate::lua::{Options, runtime, stored};

/// The largest integer every Luau number holds exactly.
const EXACT: i64 = 1 << 53;

/// `value` as one document in `format` (`json`, `yaml`, or `toml`).
///
/// # Errors
///
/// Returns an error if the runtime can't be made, `format` isn't one of the three, or `value`
/// holds what the format can't (TOML has no null in an array).
pub fn encode(value: &Json, format: &str) -> l3i::Result<String> {
    let runtime = runtime(Options::default())?;
    let module: Table = runtime.eval("return require('@jess/data/value')")?;
    let builder = Builder::new(&runtime, &module)?;
    let document = builder.build(&runtime.stack(), value)?;
    let encode: Function = module.get(&runtime.stack(), "encode")?;
    encode.invoke(&runtime.stack(), (&document, format))
}

/// The value model's constructors, taken from the module once.
struct Builder {
    structure: Function,
    array: Function,
    float: Function,
    null: Value,
}

impl Builder {
    fn new(runtime: &Runtime, module: &Table) -> l3i::Result<Builder> {
        let stack = runtime.stack();
        Ok(Builder {
            structure: module.get(&stack, "struct")?,
            array: module.get(&stack, "array")?,
            float: module.get(&stack, "float")?,
            null: module.get(&stack, "Null")?,
        })
    }

    fn build(&self, scope: &impl Scope, value: &Json) -> l3i::Result<Value> {
        match value {
            Json::Null => Ok(self.null.clone()),
            Json::Bool(value) => stored(scope, |frame| frame.push(value).map(drop)),
            Json::String(value) => stored(scope, |frame| frame.push(value.as_str()).map(drop)),
            Json::Number(number) => self.number(scope, number),
            Json::Array(items) => {
                let list = stored(scope, |frame| {
                    let list = frame.push_table(items.len(), 0)?;
                    for (index, item) in items.iter().enumerate() {
                        self.build(frame, item)?.push_to(frame)?;
                        list.raw_set_index(frame, position(index))?;
                    }
                    Ok(())
                })?;
                self.array.invoke(scope, (&list,))
            }
            Json::Object(fields) => {
                let pairs = stored(scope, |frame| {
                    let pairs = frame.push_table(fields.len(), 0)?;
                    for (index, (name, field)) in fields.iter().enumerate() {
                        let pair = frame.push_table(2, 0)?;
                        frame.push(name.as_str())?;
                        pair.raw_set_index(frame, 1)?;
                        // A null field is left absent: `struct` reads it as the absent option.
                        if !field.is_null() {
                            self.build(frame, field)?.push_to(frame)?;
                            pair.raw_set_index(frame, 2)?;
                        }
                        pairs.raw_set_index(frame, position(index))?;
                    }
                    Ok(())
                })?;
                self.structure.invoke(scope, (&pairs,))
            }
        }
    }

    fn number(&self, scope: &impl Scope, number: &serde_json::Number) -> l3i::Result<Value> {
        if let Some(whole) = number.as_i64() {
            if (-EXACT..=EXACT).contains(&whole) {
                #[allow(clippy::cast_precision_loss)] // Within 2^53, exactly.
                return stored(scope, |frame| frame.push(&(whole as f64)).map(drop));
            }
            return stored(scope, |frame| frame.push(&l3i::convert::Integer(whole)).map(drop));
        }
        let value = number.as_f64().unwrap_or(f64::NAN);
        if number.is_u64() {
            return stored(scope, |frame| frame.push(&value).map(drop));
        }
        self.float.invoke(scope, (value,))
    }
}

/// The 1-based Luau index of the `index`th item.
#[allow(clippy::cast_possible_wrap)] // A document holds far fewer than 2^63 items.
fn position(index: usize) -> i64 {
    index as i64 + 1
}
