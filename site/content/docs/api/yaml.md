---
title: '@jess/yaml'
description: 'Read and write YAML 1.2.2: the Core schema, anchors and aliases, explicit tags, several documents in one text, and block-style output.'
weight: 135
extra:
  kind: api
---

{{ api_signature(value="require('@jess/yaml'): { decode, decodeAll, encode, null, version }") }}

`@jess/yaml` is the YAML codec, Luau in the built-in package. It loads [YAML 1.2.2](https://yaml.org/spec/1.2.2/) with the Core schema, and writes one document in block style. It needs no capability.

YAML is a built-in data format, not a privileged native path, and the loader is self-contained on purpose: `@dream/bytes` for the UTF-8 work, nothing else native. Presentation trivia (comments, quoting style, indentation) is discarded. Anchors and aliases keep graph identity: an alias is the same Luau table as its anchor, not a copy. A tag the schema doesn't know is kept, explicitly, as `{ tag, value }`.

A YAML null (`~`, `null`, or nothing) decodes to [`jess.null`](@/docs/api/jess.md#jess-null), which `yaml.null` is another name for, and `encode` writes `jess.null` back as `null`. It's the one null every codec in jess shares.

## yaml.decode

{{ api_signature(value="yaml.decode(text: string, options: Options?) -> Value") }}

Parses a text holding one document and returns its value. `yaml.version` is the spec version the loader follows, `'1.2.2'`.

`Options` is `{ maxDepth: number? }`; the exported `Value` type is `unknown`.

- **Parameters:** `text`, the whole document as UTF-8. `options.maxDepth`: how deep collections may nest, 512 by default; it must be a positive integer.
- **Returns:** the document's value, or `jess.null` for a text with no document in it. Mappings are tables, sequences are tables from 1, and scalars resolve by the Core schema: see the table below.
- **Errors:** `YAML: <what's wrong> at line <n>, column <n>` for a text the grammar refuses (`YAML: expected ',' or ']' in flow sequence at line 1, column 9`), a duplicate mapping key, a NaN mapping key, an undefined tag handle, an invalid `!!int` or `!!bool` scalar, an integer outside the signed 64-bit range, or nesting past `maxDepth` (`YAML: maximum nesting depth exceeded at line 1, column 4`). An invalid depth raises `YAML: maxDepth must be a positive integer`. `YAML: expected one document, got <n>; use decodeAll` means the text contains several documents.

The result is `unknown`. Cast it to the shape your program expects in `--!strict` code, and check what you can't count on.

```luau
--!strict
local yaml = require '@jess/yaml'

type Manifest = {
  id: string,
  version: string,
  count: number,
  empty: JessNull,
  tags: { string },
  first: string,
  custom: { tag: string, value: string },
}

local document = yaml.decode [[
id: com.example.greeter
version: 1.0.0
count: 0x10
empty: ~
tags: [cli, &tool tool]
first: *tool
custom: !flag enabled
]]
local manifest = document :: Manifest
print(manifest.id, typeof(manifest.version), manifest.count, manifest.empty == jess.null)
print(manifest.tags[2], manifest.first, manifest.custom.tag, manifest.custom.value)
```

```text
com.example.greeter	string	16	true
tool	tool	!flag	enabled
```

`1.0.0` is a string because the Core schema says so: two dots make it no number. `0x10` is an integer because the Core schema says that too.

## yaml.decodeAll

{{ api_signature(value="yaml.decodeAll(text: string, options: Options?) -> { Value }") }}

Parses a text holding any number of documents, separated by `---`, and returns their values in order. A text with none gives an empty table. It takes the same depth option and reports the same parsing errors as `decode`.

```luau
--!strict
local yaml = require '@jess/yaml'

local documents = yaml.decodeAll '---\nid: one\n---\nid: two\n'
print(#documents, (documents[1] :: { id: string }).id, (documents[2] :: { id: string }).id)
```

```text
2	one	two
```

## yaml.encode

{{ api_signature(value="yaml.encode(value: Values.Value) -> string") }}

`Values.Value` is the exported `Value` type of [`@jess/data/value`](@/docs/api/value.md).

Writes a value built with [`@jess/data/value`](@/docs/api/value.md) as one document in block style, as `serde_yaml` writes it: plain scalars where a reader would take them back as what they were, quoted where it wouldn't (a string that looks like a number, a boolean, or a null stays a string), sequences one item per line, keys in the order the map was given them, a variant as a `!tag` on its value, an absent field as `null`.

- **Parameters:** `value`, from `value.struct`, `value.map`, `value.array`, `value.variant`, `value.float`, or a scalar.
- **Returns:** the document, ending in a newline.
- **Errors:** `data: a bare table is neither a list nor a map: build it with array, map, or struct` for a Luau table the value module didn't build.

```luau
--!strict
local yaml = require '@jess/yaml'
local value = require '@jess/data/value'

local record = value.struct {
  { 'id', 'com.example.greeter' },
  { 'version', '1.0.0' },
  { 'layers', value.array { 'example.greetings' } },
  { 'note', nil },
}

print(yaml.encode(record))
```

```text
id: com.example.greeter
version: 1.0.0
layers:
- example.greetings
note: null
```

## Type mapping

| YAML | Luau |
| --- | --- |
| Plain `~`, `null`, `Null`, `NULL`, or an empty value | `jess.null` |
| Plain `true`, `false` (and `True`, `TRUE`, ...) | `boolean` |
| Plain decimal, `0o` octal, `0x` hexadecimal integer | `number` within ±2^53, beyond it an exact `integer` (`typeof(value) == 'integer'`) |
| Plain float, `.inf`, `-.inf`, `.nan` | `number` |
| Any other plain scalar, and every quoted scalar | `string` |
| `!!str`, `!!null`, `!!bool`, `!!int`, `!!float`, `!!seq`, `!!map` | The Core type, checked: `!!int abc` is an error, not a string |
| Any other tag, `!flag` or `!<tag:example.com,2026:thing>` | `{ tag: string, value: unknown }`, with the tag as written, or resolved through its `%TAG` handle |
| Sequence | A table from 1 |
| Mapping | A table keyed by the resolved keys |
| Alias | The anchored value itself, by identity |

Related: [`@jess/data/value`](@/docs/api/value.md), [`@jess/json`](@/docs/api/json.md), [`@jess/data/toml`](@/docs/api/toml.md).
