---
title: '@jess/json'
description: 'Read and write JSON: RFC 8259 in, serde-shaped documents out, and a plain-table writer for what people read and diff.'
weight: 130
extra:
  kind: api
---

{{ api_signature(value="require('@jess/json'): { decode, encode, encodeTable, null }") }}

`@jess/json` is the JSON codec, Luau in the built-in package. It reads [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259) and nothing looser: no comments, no trailing commas, no single quotes, no unquoted keys. It needs no capability; getting the text is your business.

There are two writers because there are two reasons to write JSON. `encode` takes a value built with [`@jess/data/value`](@/docs/api/value.md) and writes exactly what a Rust program's serializer would, key order and all. `encodeTable` takes an ordinary Luau table and writes something stable enough to diff. Pick by whether the shape matters more than the convenience.

A JSON `null` decodes to [`jess.null`](@/docs/api/jess.md#jess-null), which `json.null` is another name for, and encodes back from it. It's the one null every codec in jess shares, so a null read from JSON is one `@jess/yaml` writes.

## json.decode

{{ api_signature(value="json.decode(text: string, options: { maxDepth: number? }?) -> unknown") }}

Parses one JSON text and returns its value.

- **Parameters:** `text`, the whole document as UTF-8 (a leading byte-order mark is skipped, as the RFC allows a parser to). `options.maxDepth`: how deep arrays and objects may nest, 512 by default.
- **Returns:** the value. Objects are tables with string keys, arrays are tables from 1, strings are strings, numbers are numbers, `true` and `false` are booleans, and `null` is `jess.null`. A repeated key keeps the last value.
- **Errors:** `json: <what's wrong> at line <n>, column <n>` for anything the grammar doesn't allow (`json: trailing comma in array at line 1, column 7`), a text that isn't valid UTF-8, or nesting past `maxDepth` (`json: nesting exceeds maxDepth (1) at line 1, column 3`). `json: maxDepth must be a positive integer` for an option that isn't one.

The result is `unknown`: it is someone else's document. In `--!strict` code, cast it to the shape your program expects and check with `typeof` whatever you can't afford to be wrong about.

```luau
--!strict
local json = require '@jess/json'

type Manifest = { id: string, version: string, tags: { string | JessNull } }

local document = json.decode '{"id": "com.example.greeter", "version": "1.0.0", "tags": ["cli", null]}'
local manifest = document :: Manifest
print(manifest.id, manifest.version, #manifest.tags, manifest.tags[2] == jess.null)
```

```text
com.example.greeter	1.0.0	2	true
```

## json.encode

{{ api_signature(value="json.encode(value: Value) -> string") }}

Writes a value built with [`@jess/data/value`](@/docs/api/value.md) as compact JSON on one line, the way `serde_json` writes it: keys in the order the map was given them, a forced float as `1.0`, an absent field as `null`, a variant as a one-key object.

- **Parameters:** `value`, from `value.struct`, `value.map`, `value.array`, `value.variant`, `value.float`, or a scalar.
- **Returns:** the document, with no trailing newline.
- **Errors:** `data: a bare table is neither a list nor a map: build it with array, map, or struct` for a Luau table that wasn't built with the value module. A Luau table says neither whether it is a list or a map nor what order its keys come in, and a writer that guessed would write a different document from the same data on another day.

```luau
--!strict
local json = require '@jess/json'
local value = require '@jess/data/value'

local record = value.struct {
  { 'id', 'com.example.greeter' },
  { 'ratio', value.float(1) },
  { 'layers', value.array { 'example.greetings' } },
}

print(json.encode(record))
```

```text
{"id":"com.example.greeter","ratio":1.0,"layers":["example.greetings"]}
```

## json.encodeTable

{{ api_signature(value="json.encodeTable(value: unknown, options: { indent: number?, latin1: boolean? }?) -> string") }}

Writes a plain Luau table as JSON, for tools that write what people read and diff. The rules are fixed so the output is too: a table whose keys are exactly 1 to some *n* is an array; a table with string keys is an object, its keys sorted; an empty table is `[]`; `nil` and `jess.null` are `null`; a field that is `nil` is absent.

- **Parameters:** `value`, a string, number, boolean, `jess.null`, or table of those. `options.indent`: spaces per level, over several lines; without it, one line. `options.latin1`: escape every byte outside printable ASCII as the Latin-1 code point it is, for a reader that can't take UTF-8.
- **Returns:** the document, with no trailing newline.
- **Errors:** `json: a number that is NaN or infinite has no JSON`; `json: a <type> has no JSON` for a function, buffer, or other value JSON can't hold; `json: a table needs string keys or dense array indexes` for a table with a hole in its array part, since a hole makes it an object and an object needs string keys.

```luau
--!strict
local json = require '@jess/json'

print(json.encodeTable({ id = 'com.example.greeter', targets = { 'greet', 'version' } }, { indent = 2 }))
```

```text
{
  "id": "com.example.greeter",
  "targets": [
    "greet",
    "version"
  ]
}
```

## Type mapping

| JSON | Luau |
| --- | --- |
| String | `string`, with escapes decoded, `\uXXXX` pairs included |
| Number | `number`. JSON has one number type and so does the decoder: an integer past 2^53 is read as the nearest float. |
| `true`, `false` | `boolean` |
| `null` | `jess.null` |
| Array | A table from 1, with `jess.null` where the array had `null`, so positions hold |
| Object | A table with string keys |

Related: [`@jess/data/value`](@/docs/api/value.md), [`@jess/yaml`](@/docs/api/yaml.md), [`@jess/data/toml`](@/docs/api/toml.md).
