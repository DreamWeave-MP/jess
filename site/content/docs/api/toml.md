---
title: '@jess/data/toml'
description: 'Read and write TOML: tables with key order, float and datetime markers, and exact 64-bit integers.'
weight: 140
extra:
  kind: api
---

{{ api_signature(value="require('@jess/data/toml'): { decode, encode, version }") }}

`@jess/data/toml` reads [TOML 1.1](https://toml.io) documents into Luau tables and writes them
back. It's jess's one TOML codec, Luau in the built-in package: package manifests, `jess.lock`
and `jess-workspace.toml` are read with the same decoder. It needs no capability; reading the file
is yours.

`toml.version` is the TOML specification version the decoder implements.

## decode

{{ api_signature(value="toml.decode(text: string, options: { positions: boolean? }?) -> (toml.Table, toml.Layout)") }}

Parses a TOML document, returning its tables and a layout table for what Luau tables forget.

- **Parameters:** `text`, the whole document, as UTF-8. `options.positions`: also record where
  each key sits in the text.
- **Returns:** two values:
  1. The document's top-level table.
  2. The layout, keyed by each decoded table and array (by identity). For a table, the entry has
     `keys`, its keys in document order; `floats`, which marks keys written as floats (`1.0`)
     rather than integers (`1`); and `datetimes`, which marks TOML datetimes, read as strings. For
     an array, `floats` and `datetimes` are keyed by 1-based position. With `positions`, a table's
     entry also has `positions`: for each key written as `key = value` (not inside an inline
     table) or first named by a header, `{ key, value, valueEnd }`, 1-based byte offsets of the
     key and of its value as written, quotes included (for a header, its `[`). It's what a tool
     needs to put an error of its own on the right line.
- **Errors:** `toml: <what's wrong> at line <n>, column <n>`; a duplicate key is placed at the
  repeated key.

```luau
--!strict
local toml = require('@jess/data/toml')
local text = 'name = "greeter"\n'
local document, layout = toml.decode(text, { positions = true })
local positions = layout[document].positions or error 'decoded without positions'
local where = positions.name
print(where.key, string.sub(text, where.value, where.valueEnd)) -- 1	"greeter"
```

```luau
--!strict
local toml = require('@jess/data/toml')
local settings, layout = toml.decode [[
title = "Site build"
ratio = 1.0
count = 3
when = 1979-05-27

[owner]
name = "Tom"
]]
print(settings.title, settings.count) -- Site build  3
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

A decoded value is `unknown`: it's someone else's file, and it could be any TOML type. In `--!strict` code, cast a document to the shape your program expects, and check with `typeof` what it can't count on. A cast is a claim the checker takes on trust, so check anything you can't afford to be wrong about:

```luau
--!strict
local toml = require('@jess/data/toml')

type Settings = { owner: { name: string, scores: { number } } }

local document, layout = toml.decode '[owner]\nname = "Tom"\nscores = [1, 2.5]\n'
local settings = document :: Settings
print(settings.owner.name) -- Tom
print(layout[settings.owner.scores].floats[2]) -- true

local version = document.version
if typeof(version) == 'string' then print(`version {version}`) end
```

Related: [Files, TOML, and Regex](@/docs/scripting/files-toml-regex.md).
