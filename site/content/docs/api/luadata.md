---
title: 'jess.luadata'
description: 'Read and write OpenMW Lua data (the storage in saves and omwaddons).'
weight: 80
extra:
  kind: api
---

{{ api_signature(value="jess.luadata: { decode, encode, objectRef, refIndex, refContentFile, objectList, vec2, vec4, color, quaternion, matrix, userdata }") }}

OpenMW serializes Lua values in its own binary format: script storage in saves (`LUAD` subrecords, which [save:luaData](@/docs/api/save.md#save-luadata) decodes for you) and the `initData` of scripts attached in `.omwaddon` files. `jess.luadata` converts that format to Luau values and back.

## LuaDataValue

{{ api_signature(value="type LuaDataValue = boolean | number | string | vector | ObjectRef | ObjectList | Vec2 | Vec4 | Color | Quaternion | Matrix | Userdata | { [LuaDataValue]: LuaDataValue }") }}

| OpenMW Lua value | Luau value |
| --- | --- |
| `nil`, booleans, numbers, strings, tables | The same. Strings are byte strings. |
| `util.vector3` | A Luau `vector` (`vector.create(x, y, z)`). OpenMW stores 32-bit floats. |
| A game object reference | `ObjectRef`: one Luau `integer` holding the refnum. `jess.luadata.refIndex` and `refContentFile` read it; `==` compares it. A save's thousands of references allocate nothing. |
| A list of object references | `ObjectList` userdata: `#list`, `list[i]` (1-based) giving `ObjectRef`s. |
| `util.vector2`, `util.vector4` | `Vec2` (`x`, `y`) and `Vec4` (`x`, `y`, `z`, `w`) userdata. |
| `util.color` | `Color` userdata: `r`, `g`, `b`, `a`. |
| `util.transform` (matrix or quaternion) | `Matrix` userdata (16 `values`) or `Quaternion` (`x`, `y`, `z`, `w`). |
| Anything else OpenMW stores as userdata | `Userdata`: `typeName` and raw `data` (a `buffer`). |

Every `Vec2`, `Vec4`, `Color`, `Quaternion`, and `Matrix` also has `kind` (its type name) and `values` (its components as an array). All of them compare with `==` by value, and `tostring` gives `Color(1, 0.5, 0, 1)` and the like. Engine values keep their type through a decode and encode, so data you don't touch goes back unchanged.

## jess.luadata.decode

{{ api_signature(value="jess.luadata.decode(bytes: buffer | string) -> LuaDataValue?") }}

Decodes serialized Lua data.

- **Errors:** `unsupported Lua serialization format version <n> at offset <n>`, or truncated or malformed data.

```luau
--!strict
local bytes = jess.luadata.encode { answer = 42, name = 'Fargoth' }
local value = jess.luadata.decode(bytes) :: { [string]: LuaDataValue }
print(value.answer, value.name) -- 42  Fargoth
-- Data that isn't serialized Lua throws; pcall hands back the message instead.
print(pcall(jess.luadata.decode, 'not lua data'))
```

## jess.luadata.encode

{{ api_signature(value="jess.luadata.encode(value: LuaDataValue?) -> buffer") }}

Serializes a value in OpenMW's format.

Encoding is deterministic: a table's array part goes first, in order, then its other keys sorted (booleans, then numbers, then strings), so the same value always gives the same bytes.

- **Errors:** `a function can't be stored as Lua data` (and the like for threads), or `this userdata can't be stored as Lua data` for userdata that isn't one of the types above.

```luau
--!strict
local bytes = jess.luadata.encode {
  target = jess.luadata.objectRef(12, 0),
  at = vector.create(1, 2, 3),
  tint = jess.luadata.color(1, 0.5, 0),
}
local value = jess.luadata.decode(bytes) :: { [string]: LuaDataValue }
print(jess.luadata.refIndex(value.target), value.at, value.tint) -- 12  1, 2, 3  Color(1, 0.5, 0, 1)
```

## Constructors

### jess.luadata.objectRef

{{ api_signature(value="jess.luadata.objectRef(index: number, contentFile: number) -> ObjectRef") }}

A reference to a game object: its `index` within its content file, and the `contentFile`'s position in the load order (as OpenMW numbers them), packed into one Luau `integer` (the index in the high 32 bits, the content file in the low 32). Integers compare with `==` only and never equal a `number`.

```luau
--!strict
local ref = jess.luadata.objectRef(12, 0)
print(type(ref), ref == jess.luadata.objectRef(12, 0)) -- integer  true
```

### jess.luadata.refIndex, refContentFile

{{ api_signature(value="jess.luadata.refIndex(ref: ObjectRef) -> number") }}
{{ api_signature(value="jess.luadata.refContentFile(ref: ObjectRef) -> number") }}

The two halves of a reference.

```luau
--!strict
local ref = jess.luadata.objectRef(12, 0)
print(jess.luadata.refIndex(ref), jess.luadata.refContentFile(ref)) -- 12  0
```

### jess.luadata.objectList

{{ api_signature(value="jess.luadata.objectList(refs: { ObjectRef }) -> ObjectList") }}

A list of object references.

```luau
--!strict
local list = jess.luadata.objectList { jess.luadata.objectRef(1, 0), jess.luadata.objectRef(2, 1) }
print(#list, jess.luadata.refIndex(list[2])) -- 2  2
```

### jess.luadata.vec2

{{ api_signature(value="jess.luadata.vec2(x: number, y: number) -> Vec2") }}

A `util.vector2`.

```luau
--!strict
local vector = jess.luadata.vec2(1, 2)
print(vector.x, vector.y, vector.kind) -- 1  2  Vec2
```

### jess.luadata.vec4

{{ api_signature(value="jess.luadata.vec4(x: number, y: number, z: number, w: number) -> Vec4") }}

A `util.vector4`.

```luau
--!strict
print(jess.luadata.vec4(1, 2, 3, 4).z) -- 3
```

### jess.luadata.color

{{ api_signature(value="jess.luadata.color(r: number, g: number, b: number, a: number?) -> Color") }}

A `util.color`; `a` defaults to 1. Components are stored as 32-bit floats.

```luau
--!strict
print(jess.luadata.color(1, 0.5, 0)) -- Color(1, 0.5, 0, 1)
```

### jess.luadata.quaternion

{{ api_signature(value="jess.luadata.quaternion(x: number, y: number, z: number, w: number) -> Quaternion") }}

A `util.transform` stored as a rotation quaternion.

```luau
--!strict
print(jess.luadata.quaternion(0, 0, 0, 1).w) -- 1
```

### jess.luadata.matrix

{{ api_signature(value="jess.luadata.matrix(values: { number }) -> Matrix") }}

A `util.transform` stored as a 4×4 matrix of 16 values.

- **Errors:** `a matrix has 16 values, not <n>`.

```luau
--!strict
local identity = jess.luadata.matrix { 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1 }
print(identity.kind, #identity.values) -- Matrix  16
```

### jess.luadata.userdata

{{ api_signature(value="jess.luadata.userdata(typeName: string, data: buffer | string) -> Userdata") }}

A userdata value of a type jess doesn't decode, kept as its type name and raw bytes.

```luau
--!strict
local custom = jess.luadata.userdata('MyType', 'abc')
print(custom.typeName, buffer.tostring(custom.data)) -- MyType  abc
```

Related: [Saves in scripts](@/docs/scripting/saves.md).
