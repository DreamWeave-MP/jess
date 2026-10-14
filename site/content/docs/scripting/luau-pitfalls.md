---
title: 'Luau Pitfalls'
description: 'Type checker and runtime surprises you will hit writing strict Luau for jess, each with a repro and the fix.'
weight: 105
extra:
  kind: guide
---

Porting tes3cmd, the OpenMW save tools, and jess's own plugin library to strict Luau ran into the same handful of surprises more than once. None of them is hard once you know about it. Every repro below is checked by the type checker jess itself uses: Luau's new solver, the same one `luau-lsp` runs over a [`jess init`](@/docs/cli/utility-commands.md) folder.

Some are bugs in Luau, and the section says so, with a repro small enough to file. Some are Luau working as designed. One is a jess decision.

## `table.create` without a value, and `string.unpack`, give you `unknown`

```luau
--!strict
local slots: { number } = table.create(4)
local count, size = string.unpack('<I4I4', 'abcdefgh')
local total: number = count + size
```

The first line fails with `Expected this to be '{number}', but got '{unknown}'`. With no fill value, `table.create`'s element type is never inferred, not even from the annotation it's being assigned to. `string.unpack` returns `unknown` for every value, so the arithmetic fails too.

Fix: pass a fill value when there is one, and cast what can't be inferred.

```luau
--!strict
local slots: { number } = table.create(4, 0)
local flags = table.create(4) :: { boolean }
local count, size = string.unpack('<I4I4', 'abcdefgh')
local total = (count :: number) + (size :: number)
```

Upstream: `table.create(n)` not taking the expected type is a Luau bug; the old solver accepts the first repro. `string.unpack` returning `unknown` is a missing feature rather than a bug: `string.format` already reads its format string to type its arguments, and `string.unpack` could read its format the same way. Both repros above are ready to file at [luau-lang/luau](https://github.com/luau-lang/luau/issues).

## A table literal passed through `pcall` isn't checked against the function

```luau
--!strict
type Options = { encoding: string?, profile: ('esp' | 'omwaddon')? }

local function open(bytes: string, options: Options?): number
  return #bytes
end

-- `open` may throw; keep the failure as a value so this protected-call typing repro can continue.
local ok, result = pcall(open, 'x', { encoding = 'win1252', profile = 'esp' })
```

That fails with `Expected this to be 'Options?', but got '{ encoding: string, profile: ("esp" | "omwaddon")? }'`. Called directly, `open('x', { ... })` checks the literal against `Options`, and the same table type-checks. Passed through `pcall`, the literal gets its own type first, a `string` where `Options` has a `string?`, and a mutable field's type has to match exactly.

Fix: call through a closure, which is also how you get typed results back, or give the table its type first.

```luau
--!strict
type Options = { encoding: string?, profile: ('esp' | 'omwaddon')? }

local function open(bytes: string, options: Options?): number
  return #bytes
end

-- `open` may throw; keep the failure as a value while the closure preserves argument context.
local ok, result = pcall(function(): number return open('x', { encoding = 'win1252' }) end)
local options: Options = { encoding = 'win1252', profile = 'esp' }
-- `open` may throw; keep the failure as a value while the typed options reach the call unchanged.
local again, size = pcall(open, 'x', options)
```

Upstream: a Luau bug. Generic functions like `pcall` should pass the expected parameter types through to their arguments the way a direct call does. The first repro is ready to file.

## A table that isn't a literal doesn't fit an options type

```luau
--!strict
local bytes = require '@dream/bytes'

local Layout = table.freeze { header = 16, lengthAt = 4 }
local spans, count = bytes.frame(buffer.create(0), Layout)
```

`bytes.frame` takes `{ header: number, lengthAt: number, lengthSize: number?, bigEndian: boolean?, inner: {...}? }`. The frozen table's fields type as read-only (`read header: number`), and read-only fields don't fit read-write ones. Drop the `table.freeze` and it still fails: a table bound to a local has no `lengthSize` at all, and the checker wants the optional fields present. The same table written as a literal in the call checks fine, because a literal is checked against the parameter's type.

Fix: write option tables at the call. Building one is cheap next to whatever the call does.

```luau
--!strict
local bytes = require '@dream/bytes'

local spans, count = bytes.frame(buffer.create(0), { header = 16, lengthAt = 4 })
local ascii = bytes.regex('(?i)menumode', { unicode = false })
```

Upstream: read-only fields from `table.freeze` are deliberate. Refusing a table only because it lacks an optional field is stricter than the old solver was. It's worth asking about, though it may well be intended.

## Two types with one name

```luau
--!strict
type Regex = { source: string }

local function wants(regex: Regex): string
  return regex.source
end

print(wants(jess.regex.new 'a'))
```

jess's definitions declare `Regex` (and `Plugin`, `Record`, and the rest) as global types. Declare a local one with the same name and the error reads `Expected this to be 'Regex' from 'main.jsl', but got 'Regex' from '@jess'`. Older Luau printed both as plain `Regex`, which is where "Regex is not Regex" came from. Today's message tells you which is which, but the clash is still there.

Fix: don't reuse the global type names for your own types. Call yours `Pattern`, or whatever it actually is.

Upstream: the confusing message is fixed in the Luau jess uses. There's nothing to file.

## An `if` expression is one value

```luau
--!strict
local function pair(): (number, string)
  return 1, 'one'
end

local count
local name
count, name = if math.random() > 0.5 then pair() else pair()
print(count, name)
```

That prints `1 nil`. An `if` expression is an expression, and an expression is one value, so `name` is always `nil`. The type checker only notices when you use `name` as a `string`. Luau's linter warns (`UnbalancedAssignment`), but a strict type check alone doesn't. This one cost a real bug: a save's screenshot read from a Morrowind save came back with no format, because the second return value was lost.

Fix: use an `if` statement when a branch returns more than one value.

```luau
--!strict
local function pair(): (number, string)
  return 1, 'one'
end

local count, name
if math.random() > 0.5 then
  count, name = pair()
else
  count, name = pair()
end

print(count, name)
```

Upstream: this is the language working as designed. Run `luau-lsp` or `luau-analyze` with linting on and you'll see the warning.

## `_G` is read-only

```luau
-- Writing `_G.answer` throws under Jess; capture the error so the example can print it.
local ok, problem = pcall(function() _G.answer = 42 end)
print(ok, problem)
```

Under `jess run` (and `jess test`, load-order scripts, and packages) that prints `false` and `attempt to modify a readonly table`, and so does a bare `answer = 42`. jess sandboxes its globals: scripts share one runtime with the jess API, and nothing one script does to the globals should leak into the next one. Reading works, so `(_G :: any).jess` is fine.

Fix: keep state in locals, or in a module that returns a table. `require` caches modules, so every script that requires one gets the same table.

```luau
--!strict
local State = {}
State.answer = 42
return State
```

Upstream: nothing to file. The sandbox is jess's decision, made through Luau's own sandboxing.

## Related

- [Luau Style](@/docs/contributing/luau-style.md): how jess's own Luau is written, and the checks it passes.
- [Editor Setup](@/docs/getting-started/editor-setup.md): `luau-lsp` with jess's types, which shows every one of these as you type.
