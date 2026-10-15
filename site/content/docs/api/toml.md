---
title: '@jess/data/toml'
description: 'Read and write TOML: tables with key order, float and datetime markers, and exact 64-bit integers.'
weight: 110
extra:
  kind: api
---

{{ api_signature(value="require('@jess/data/toml'): { decode, encode, version }") }}

`@jess/data/toml` reads [TOML 1.1](https://toml.io) documents into Luau tables and writes them
back. It's jess's one TOML codec, Luau in the built-in package: package manifests, `jess.lock`
and `jess-workspace.toml` are read with the same decoder. It needs no capability; reading the file
is yours.

## decode

{{ api_signature(value="toml.decode(text: string) -> (toml.Table, toml.Layout)") }}

Parses a TOML document, returning its tables and a layout table for what Luau tables forget.

- **Parameters:** `text`, the whole document, as UTF-8.
- **Returns:** two values:
  1. The document's top-level table.
  2. The layout, keyed by each decoded table and array (by identity). For a table, the entry has
     `keys`, its keys in document order; `floats`, which marks keys written as floats (`1.0`)
     rather than integers (`1`); and `datetimes`, which marks TOML datetimes, read as strings. For
     an array, `floats` and `datetimes` are keyed by 1-based position.
- **Errors:** `toml: <what's wrong> at line <n>, column <n>`.

```luau
--!strict
local toml = require('@jess/data/toml')
local settings, layout = toml.decode [[
title = "My load order"
ratio = 1.0
count = 3
when = 1979-05-27

[owner]
name = "Tom"
]]
print(settings.title, settings.count) -- My load order  3
local top = layout[settings]
print(table.concat(top.keys or {}, ', ')) -- title, ratio, count, when, owner
print(top.floats.ratio, top.floats.count, top.datetimes.when) -- true  nil  true
```

## encode

{{ api_signature(value="toml.encode(value: Value) -> string") }}

Writes a value built with `@jess/data/value` (its `map`, `struct`, `array` and `float`) as a TOML
document, laid out the way Rust's `toml` crate pretty-prints one: values before tables, a table's
keys in the order the map was given them. An absent field (`None`) is left out of a table, and
raises inside a list, where TOML has nothing to write for it.

## Type mapping

| TOML | Luau |
| --- | --- |
| String | `string` |
| Integer | `number` within ±2^53, beyond it an exact `integer` (`typeof(value) == 'integer'`) |
| Float | `number`. `layout[table].floats[key]` is `true`, so you can tell `1.0` from `1`. |
| Boolean | `boolean` |
| Offset date-time, local date-time, local date, local time | `string`, in RFC 3339 form (`"1979-05-27T07:32Z"`). `layout[table].datetimes[key]` is `true`. |
| Array | An array table, with its own layout entry (`floats` and `datetimes`). |
| Table, inline table, array of tables | Tables, each with its own layout entry. |

Decoded values could be any TOML type, so narrow them before use in `--!strict` code:

```luau
--!strict
local toml = require('@jess/data/toml')
local settings, layout = toml.decode '[owner]\nname = "Tom"\nscores = [1, 2.5]\n'
local owner = settings.owner
if typeof(owner) == 'table' then
  print(owner.name) -- Tom
  local scores = owner.scores
  if typeof(scores) == 'table' then
    print(layout[scores].floats[2]) -- true
  end
end
```

Related: [Files, TOML, and Regex](@/docs/scripting/files-toml-regex.md).
