---
title: 'jess.toml'
description: 'Read TOML documents into tables, with key order, float markers, and datetime markers.'
weight: 110
extra:
  kind: api
---

{{ api_signature(value="jess.toml: { decode }") }}

`jess.toml` reads [TOML](https://toml.io) documents, such as a script's settings file, into Luau tables. It reads only: there's no encoder.

## jess.toml.decode

{{ api_signature(value="jess.toml.decode(text: string) -> (TomlTable, TomlLayout)") }}

Parses a TOML document, returning its tables and a layout table for what Luau tables forget.

- **Parameters:** `text`, the whole document.
- **Returns:** two values:
  1. The document's top-level table (`TomlTable`).
  2. A `TomlLayout`, keyed by each decoded table and array (by identity). For a table, the entry has `keys`, its keys in document order; `floats`, which marks keys written as floats (`1.0`) rather than integers (`1`); and `datetimes`, which marks TOML datetimes exposed as strings. For an array, `floats` and `datetimes` are keyed by 1-based position.
- **Errors:** `TOML parse error at line <n>, column <n>` with the offending line and what was expected.

```luau
--!strict
local settings, layout = jess.toml.decode [[
title = "My load order"
ratio = 1.0
count = 3

[owner]
name = "Tom"
]]
print(settings.title, settings.count) -- My load order  3
local top = layout[settings]
print(table.concat(top.keys or {}, ', ')) -- title, ratio, count, owner
print(top.floats.ratio, top.floats.count) -- true  nil
```

## Type mapping

{{ api_signature(value="type TomlValue = boolean | number | string | TomlTable | { TomlValue }") }}

| TOML | Luau |
| --- | --- |
| String | `string` |
| Integer | `number`. Integers beyond 2^53 lose precision, like any Luau number. |
| Float | `number`. `layout[table].floats[key]` is `true`, so you can tell `1.0` from `1`. |
| Boolean | `boolean` |
| Offset date-time, local date-time, local date, local time | `string`, as written in RFC 3339 form (`"1979-05-27T07:32:00Z"`). `layout[table].datetimes[key]` is `true`. |
| Array | An array table, with its own layout entry (`floats` and `datetimes`). |
| Table, inline table, array of tables | Tables, each with its own layout entry. |

Values are `TomlValue`s, so narrow them before use in `--!strict` code:

```luau
--!strict
local settings, layout = jess.toml.decode '[owner]\nname = "Tom"\nscores = [1, 2.5]\n'
local owner = settings.owner
if typeof(owner) == 'table' then
  local ownerTable = owner :: TomlTable
  print(ownerTable.name) -- Tom
  local scores = ownerTable.scores
  if typeof(scores) == 'table' then
    print(layout[scores :: { TomlValue }].floats[2]) -- true
  end
end
```

Related: [Files, TOML, and Regex](@/docs/scripting/files-toml-regex.md), [jess.fs.read](@/docs/api/fs.md#jess-fs-read).
