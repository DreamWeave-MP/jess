---
title: '@jess/data/value'
description: 'Build a value to write as JSON, YAML, or TOML: ordered maps, arrays, variants, forced floats, and the one null.'
weight: 145
extra:
  kind: api
---

{{ api_signature(value="require('@jess/data/value'): { array, map, set, struct, variant, float, kindOf, encode, shortest, display, Null, None }") }}

`@jess/data/value` is the value model behind `encode` on [`@jess/json`](@/docs/api/json.md), [`@jess/yaml`](@/docs/api/yaml.md), and [`@jess/data/toml`](@/docs/api/toml.md), Luau in the built-in package. It needs no capability.

A Luau table says neither whether it is a list or a map, nor what order its keys come in, nor whether a missing field is null or absent. A writer that guessed would write a different document from the same data on another day, and a document a tool writes gets diffed, hashed, and read back by other tools. So a value is built, not inferred: lists come from `array`, maps from `map` and `set` or from `struct`, and a bare table is refused by name.

The text the writers produce is byte for byte what a Rust program's serializer writes for the same value: `serde_json`'s compact JSON, `serde_yaml`'s block style, and the `toml` crate's pretty documents. That is what jess's own commands print, so a document a script writes and a document `jess package inspect` writes look the same.

## The model

| Kind | Built by | JSON | YAML | TOML |
| --- | --- | --- | --- | --- |
| `null` | [`jess.null`](@/docs/api/jess.md#jess-null), also `value.Null` | `null` | `null` | an error: TOML has no null |
| `none` | `value.None`, or `nil` in a `struct` field | `null` | `null` | left out of a table, an error inside an array |
| `boolean`, `string` | the Luau value | as themselves | as themselves, quoted when a reader would take them for something else | as themselves |
| `int` | a whole `number` below 2^53, or an `integer` | `3` | `3` | `3` |
| `float` | a fractional `number`, or `value.float(n)` | `1.0` | `1.0` | `1.0` |
| `array` | `value.array { ... }` | `[...]` | one item per line | `[...]` |
| `map` | `value.map()` and `value.set`, or `value.struct` | `{...}`, keys in order | keys in order | values first, then tables |
| `variant` | `value.variant(tag, value)` | `{"tag": value}` | `!tag value` | `[tag]` table |

## value.array, value.map, value.set, value.struct

{{ api_signature(value="value.array(items: { Value }) -> { Value }; value.map() -> Map; value.set(target: Map, key: string, value: Value) -> Map; value.struct(fields: { { unknown } }) -> Map") }}

`array` marks a table as a list and returns it. `map` makes an empty ordered map, `{ keys, values }`, and `set` adds a key to it, keeping the first position a key was given, so setting a key again changes its value and not its place. `struct` makes a map from `{ key, value }` pairs in order, the way a struct serializes its fields; a pair whose value is `nil` becomes `None`, an absent field.

These builders do not make copies: `array` marks the supplied table, and `set` mutates and returns the supplied map. Changing an array, a map, or a nested value before encoding changes the output. A `nil` passed to `set` becomes `None`, just as in a `struct` field; replacing it later keeps the key's original position.

```luau
--!strict
local value = require '@jess/data/value'

local options = value.map()
value.set(options, 'verbose', true)
value.set(options, 'name', 'greeter')
value.set(options, 'verbose', false)
print(table.concat(options.keys, ', '), options.values.verbose)
print(value.encode(options, 'json'))
```

```text
verbose, name	false
{"verbose":false,"name":"greeter"}
```

## value.variant, value.float

{{ api_signature(value="value.variant(tag: string, value: Value) -> Value; value.float(value: number) -> Value") }}

`variant` is an enum variant with a payload: a tagged value, written as one-key object, `!tag`, or a table by format. `float` is a number that is always written as a float, so `1` comes out as `1.0`; without it a whole number is an integer.

## value.kindOf

{{ api_signature(value="value.kindOf(value: Value) -> string") }}

Which of the kinds above a value is: `'null'`, `'none'`, `'boolean'`, `'string'`, `'int'`, `'float'`, `'array'`, `'map'`, or `'variant'`.

- **Errors:** `data: a bare table is neither a list nor a map: build it with array, map, or struct` for a table none of the builders made; `data: a <type> can't be written` for a function, buffer, or anything else no format holds.

```luau
--!strict
local value = require '@jess/data/value'

print(value.kindOf(1), value.kindOf(1.5), value.kindOf(value.float(1)), value.kindOf(jess.null), value.kindOf(value.None))
print(value.kindOf(value.array {}), value.kindOf(value.map()), value.kindOf(value.variant('a', 1)))
```

```text
int	float	float	null	none
array	map	variant
```

## value.encode

{{ api_signature(value="value.encode(value: Value, format: 'json' | 'yaml' | 'toml') -> string") }}

Writes a value in one of the three formats. The codecs' own `encode` functions call this with their format; it's here for code that picks the format at run time.

Non-finite floats are written as `null` in JSON, `.nan` or `.inf` (with a minus sign for negative infinity) in YAML, and `nan` or `inf` in TOML. TOML requires a map or variant at the document root; other root kinds raise an unsupported-type error.

- **Errors:** `data: unknown format '<name>' (expected json, yaml, or toml)`, plus whatever the format refuses: a null in TOML (`toml: unsupported unit type`), a `None` inside a TOML array (`toml: unsupported None value`), a bare table anywhere.

```luau
--!strict
local json = require '@jess/json'
local toml = require '@jess/data/toml'
local value = require '@jess/data/value'
local yaml = require '@jess/yaml'

local record = value.struct {
  { 'id', 'com.example.greeter' },
  { 'ratio', value.float(1) },
  { 'note', nil },
  { 'layers', value.array { 'example.greetings' } },
  { 'kind', value.variant('command', 'greet') },
}

print(json.encode(record))
print(yaml.encode(record))
print(toml.encode(record))
```

```text
{"id":"com.example.greeter","ratio":1.0,"note":null,"layers":["example.greetings"],"kind":{"command":"greet"}}
id: com.example.greeter
ratio: 1.0
note: null
layers:
- example.greetings
kind: !command greet

id = "com.example.greeter"
ratio = 1.0
layers = ["example.greetings"]

[kind]
command = "greet"
```

The same record, three documents: `note` is `null` twice and absent once, and the variant is an object, a tag, and a table.

## value.shortest, value.display

{{ api_signature(value="value.shortest(value: number, plus: boolean) -> string; value.display(value: number) -> string") }}

The two float spellings the writers use, for code that wants to match them. `shortest` takes a finite number and returns the shortest round-tripping decimal in Ryū's layout, fixed notation for decimal exponents from -5 to 15 and scientific past them, with `plus` writing the `+` on a positive exponent as `serde_json` does. `display` is Rust's `Display` for a float: never an exponent; it also handles NaN and infinities.

```luau
--!strict
local value = require '@jess/data/value'

print(value.shortest(1, false), value.shortest(1e20, true), value.display(1e20), value.display(0.1))
```

```text
1.0	1e+20	100000000000000000000	0.1
```

Related: [`@jess/json`](@/docs/api/json.md), [`@jess/yaml`](@/docs/api/yaml.md), [`@jess/data/toml`](@/docs/api/toml.md).
