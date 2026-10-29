---
title: 'Enough Luau to Start'
description: 'Read a small typed Luau program and learn where to go next.'
weight: 12
extra:
  kind: guide
---

Jess source is Luau in `.jsl` files. If Luau is new to you, start with a small program before learning the package model. Save this as `inventory.jsl`:

```luau
--!strict
type Entry = { name: string, count: number }

local function describe(entry: Entry): string
  return `{entry.name}: {entry.count}`
end

local Entries: { Entry } = {
  { name = 'apples', count = 3 },
  { name = 'pears', count = 2 },
}

for _, entry in Entries do
  print(describe(entry))
end
```

Run `jess check inventory.jsl` to check the types, then `jess run inventory.jsl`. The program prints:

```text
apples: 3
pears: 2
```

## Read it from the top

`--!strict` tells the checker to use strict mode for the file. `type Entry` describes a table with a string `name` and a numeric `count`. The type is for checking; it does not construct a value or validate arbitrary data at runtime.

`local function describe` defines a function visible in this file. Its parameter and return annotations describe what it accepts and returns. The backtick string interpolates the expressions in braces.

`Entries` is a table used as a list, with two entry tables in it. Luau lists conventionally start at index 1. The loop visits each entry; `_` is the index we do not use. `print` writes the resulting strings.

## What to learn next

Tables are used for both lists and named fields; they are references, so two variables can refer to the same table. `nil` represents absence, and only `nil` and `false` are falsey. Functions are values and modules return values through `require`. Types help the checker catch mistakes, but checking and running are separate operations in Jess.

The [Luau documentation](https://luau.org) covers the language. [Your First Script](@/docs/scripting/intro.md) adds Jess's arguments, modules, types, and capabilities; [Luau Pitfalls](@/docs/scripting/luau-pitfalls.md) covers details that tend to surprise people. The project's [style guide](@/docs/contributing/luau-style.md) explains the conventions used in its examples.
