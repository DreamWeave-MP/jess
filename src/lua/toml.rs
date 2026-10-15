//! `jess.toml`: TOML documents as Luau tables.
//!
//! `decode` also returns a layout table, since Luau tables forget what TOML remembers: for each
//! decoded table or array (by identity), `keys` lists its keys in document order, `floats`
//! marks which values were written as floats rather than integers, and `datetimes` marks TOML
//! datetimes that are exposed as strings.

use std::rc::Rc;

use l3i::Runtime;
use l3i::bind::Call;
use l3i::stack::{Frame, TableView};
use l3i::value::Table;

use crate::lua::plan::Functions;
use crate::lua::{Jess, Text, error, lua};

pub fn module(runtime: &Runtime, _jess: &Rc<Jess>) -> l3i::Result<Table> {
    let module = Functions::module(runtime, "toml")?;
    module.function("decode", |call: &Call<'_>, text: Text| decode(call, &text.0))?;
    module.finish()
}

/// `text` as a Luau table, and its layout table.
pub fn decode(call: &Call<'_>, text: &str) -> l3i::Result<(Table, Table)> {
    let document = toml::from_str::<toml::Table>(text).map_err(|source| error(source.to_string()))?;
    let layout = Table::new(call, 0, 8)?;
    let value = lua::stored_table(call, |frame| push_table(frame, &document, &layout))?;
    Ok((value, layout))
}

/// Records `entry` (on top of `frame`, popped) as the layout of `table` (below it).
fn record_layout(frame: &Frame<'_>, layout: &Table, table: &TableView<'_>) -> l3i::Result<()> {
    frame.with_frame(|frame| {
        let layout = layout.push_to(frame)?;
        frame.push_value(table.value())?;
        frame.push_value(frame.at(layout.index() - 1))?;
        lua::raw_set_pair(frame, &layout);
        Ok(())
    })
}

fn push_table(frame: &Frame<'_>, document: &toml::Table, layout: &Table) -> l3i::Result<()> {
    let table = frame.push_table(0, document.len())?;
    frame.with_frame(|frame| {
        let entry = frame.push_table(0, 3)?;
        let keys = frame.push_table(document.len(), 0)?;
        for (index, key) in document.keys().enumerate() {
            frame.push_string(key)?;
            keys.raw_set_index(frame, index as i64 + 1)?;
        }
        entry.raw_set(frame, "keys")?;
        let floats = frame.push_table(0, 0)?;
        for (key, value) in document {
            if matches!(value, toml::Value::Float(_)) {
                floats.raw_set_value(frame, key, &true)?;
            }
        }
        entry.raw_set(frame, "floats")?;
        let datetimes = frame.push_table(0, 0)?;
        for (key, value) in document {
            if matches!(value, toml::Value::Datetime(_)) {
                datetimes.raw_set_value(frame, key, &true)?;
            }
        }
        entry.raw_set(frame, "datetimes")?;
        record_layout(frame, layout, &table)
    })?;
    for (key, value) in document {
        push_value(frame, value, layout)?;
        table.raw_set(frame, key)?;
    }
    Ok(())
}

fn push_value(frame: &Frame<'_>, value: &toml::Value, layout: &Table) -> l3i::Result<()> {
    match value {
        toml::Value::String(text) => {
            frame.push_string(text)?;
        }
        // Luau numbers are floats; integers past 2^53 lose precision.
        #[allow(clippy::cast_precision_loss)]
        toml::Value::Integer(number) => {
            frame.push_number(*number as f64)?;
        }
        toml::Value::Float(number) => {
            frame.push_number(*number)?;
        }
        toml::Value::Boolean(flag) => {
            frame.push_boolean(*flag)?;
        }
        toml::Value::Datetime(datetime) => {
            frame.push_string(&datetime.to_string())?;
        }
        toml::Value::Array(items) => {
            let array = frame.push_table(items.len(), 0)?;
            frame.with_frame(|frame| {
                let entry = frame.push_table(0, 2)?;
                let floats = frame.push_table(0, 0)?;
                for (index, item) in items.iter().enumerate() {
                    if matches!(item, toml::Value::Float(_)) {
                        frame.push_boolean(true)?;
                        floats.raw_set_index(frame, index as i64 + 1)?;
                    }
                }
                entry.raw_set(frame, "floats")?;
                let datetimes = frame.push_table(0, 0)?;
                for (index, item) in items.iter().enumerate() {
                    if matches!(item, toml::Value::Datetime(_)) {
                        frame.push_boolean(true)?;
                        datetimes.raw_set_index(frame, index as i64 + 1)?;
                    }
                }
                entry.raw_set(frame, "datetimes")?;
                record_layout(frame, layout, &array)
            })?;
            for (index, item) in items.iter().enumerate() {
                push_value(frame, item, layout)?;
                array.raw_set_index(frame, index as i64 + 1)?;
            }
        }
        toml::Value::Table(table) => push_table(frame, table, layout)?,
    }
    Ok(())
}
