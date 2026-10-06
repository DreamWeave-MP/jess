---
title: 'Luau Style'
description: 'How the bundled scripts, the templates, and every Luau example in these docs are written, and @jess/style, the checker that holds them to it.'
weight: 50
extra:
  kind: guide
---

The built-in modules and templates are some of the first real Jess code people read, and whatever they find there is what they copy. So the Luau owned by this repo, the `jess init` template, the test fixtures, and every block on this site follows one style, and a test fails when it drifts.

[`@jess/style`](#the-checker) is the repository's enforced Luau style gate: a checker written in Luau, over Luau's own parser, in the built-in package jess carries. It owns the structural rules below and runs as part of the Rust test suite.

## Formatting

Canonical formatting matters, but it belongs in Jess's built-in Luau tooling rather than an external formatter. Until that formatter lands, there is no separate formatter-specific validation pass: write source in the canonical shape used by the surrounding code and let `@jess/style` enforce the rules it can prove.

## The style

**Blank lines are paragraphs.** One logical thought per paragraph. Setup and the action it sets up stay together; a new phase gets a blank line.

**Compound statements get room.** A multi-line `if`, `for`, `while`, `repeat`, `do`, `function`, or table constructor is always set apart by a blank line from a compound statement next to it, one-line or not. Two multi-line `if`s in a row, a loop after an `if`, a table after a loop: blank line, every time.

```luau
local function hasFlag(flags: string?, flag: string): boolean
  if not flags then return false end

  for name in string.gmatch(flags, '[^|%s]+') do
    if name == flag then return true end
  end

  return false
end
```

**Guard clauses keep the happy path flat.** Return early, `continue` early, and keep the interesting code at the lowest indentation it can have.

```luau
for record in plugin:records() do
  local meta = jess.meta(record)
  if not mergeTypes[meta.tag] then continue end

  local id = identity(record, meta)
  if not id or id == '' then continue end

  table.insert(groups[id], record)
end
```

**Declare things where they're used.** No `local Next, Type, Format = next, type, string.format` at the top of the file. That's a LuaJIT habit, and Luau doesn't need it. Constants that belong to the module are PascalCase (`PluginName`, `FlagBits`); everything else is camelCase.

**Truthiness, unless false means something.** `if not plugin then`, `value or default`. Write `== nil` only where `false` is a real value that has to survive, like a TOML boolean or an option that turns a setting off, and say so in a comment when it isn't obvious.

**Group declarations.** Same-kind locals initialize on one line; each multi-line table gets its own paragraph.

```luau
local report, summary

local merged, usedMasters, failed = {}, {}, {}

local defaults = {
  fileName = 'Merged Objects.esp',
  patches = { 'all' },
}
```

**Names say what they are.** Full words: `record`, not `rec`; `left` and `right`, not `a` and `b`. No single letters anywhere, `i`, `x`, and `k` included: a loop runs over an `index`, a grid over a `column` and a `row`. A name that needs a comment to explain it is the wrong name.

**Comments are for things code can't say.** External contracts, surprising behavior, the reason something weird is weird, licenses. Not narration. If a comment restates the next line, delete the comment and fix the line.

## Errors

Script code that breaks an invariant calls `error` and stops. It doesn't wrap its own functions in `pcall` to rewrap the message; pass the context in and raise the final message where the problem is found.

`pcall` goes around a call into a Rust API that can legitimately throw, and nowhere else: `jess.regex.new` on a user's pattern, `record:decode()` on a malformed record, `openmw:activate` on a file outside the data folders. Every one gets a comment naming what throws and why the script carries on.

```luau
-- `jess.regex.new` throws on a pattern Rust's regex crate rejects. Every bad pattern is reported
-- before the run stops, so each is caught here and collected.
local compiled, regex = pcall(jess.regex.new, pattern, CaseInsensitive)
```

A `pcall` without that comment is a bug report waiting to happen, and the checker treats it as one: it's an error, in capitals. The comment goes right above the statement holding the call, or above the statement that holds that one from its first line (a function whose first statement is the `pcall`, an `if` whose body is). One comment above a run of `pcall`s with no blank line between them speaks for the whole run, which is how a test that throws on purpose five times in a row reads. A `---@type` annotation or a `--!` directive isn't a comment for this purpose; it says nothing about what throws.

## Types

Bundled scripts start with `--!strict` and `--!native`, and the templates with `--!strict`. Everything is typed, and nothing is `any`. `unknown` is fine where the API really hands you an unknown (a `jess run` script's `state` table), and nowhere else. `LUAU_ANALYZE` makes the test suite type-check the bundled scripts, the `jess init` template, and the Luau test corpus against the generated definitions.

## The checker

`@jess/style` is the style above, as code. It's the first module of jess's built-in package, [`jpk/builtin`](@/docs/scripting/packages.md#the-built-in-package), and it's Luau end to end: one parse with [`@dream/luau`](@/docs/api/downstream-globals.md), Luau's own parser, the tree and its token stream as tables; one pass over the tokens for the layout of every line; one walk over the tree for everything else. No regexes over source text, no guessing where a long string ends.

```sh
jess run check-style.luau libs jpk tools
```

```luau
--!strict
-- check-style.luau: every finding under the folders given, and a count.
local style = require '@jess/style'
local errors, warnings = style.run { ... }

print(`{errors} errors, {warnings} warnings`)
```

`style.run` prints one line per finding, `path:line:column: severity[rule]: message`. `style.check(source, options)` returns the findings as tables (`rule`, `severity`, `message`, and the span: `line`, `column`, `endLine`, `endColumn`), and `style.checkFile` reads a file first, checking a Markdown page's ```` ```luau ```` blocks one by one. The [API page](@/docs/api/style.md) has the rest.

| Rule | Severity | What it wants |
| --- | --- | --- |
| `syntax` | error | Luau that parses. |
| `pcall.comment` | error | A comment right above every `pcall` and `xpcall`, naming what throws. |
| `layout.indentation` | error | Spaces, two per level. Text inside a long string or comment keeps its own layout. |
| `layout.compound` | error | A blank line between adjacent compound statements when either spans several lines. A statement holding a multi-line table or function counts as compound. |
| `quotes.double` | error | Single quotes, unless the string holds one. |
| `quotes.backtick` | error | Backticks only around interpolation. |
| `hoisting` | error | No local that only renames a Luau global or library function. `math.huge` and friends are values, not functions, so naming one is fine. |
| `naming.case` | error | camelCase locals; PascalCase only for module constants. `IDENTITY` and `light_count` are neither. |
| `naming.constant` | warning | A module-level local holding a literal that nothing reassigns (or, for a table, nothing changes or hands to a function) is a constant, and PascalCase. |
| `naming.words` | warning | Full words. No single-letter name in any position: locals, parameters, loop variables, and the keys you write in a table constructor; `_` is the only one-character name. No abbreviations either: `record`, not `rec`. |
| `truthiness` | warning | `if not name` for a value that can't be false. Only when the checker can prove it: a local annotated with a type that has no `boolean` in it (aliases in the file followed), a literal, or a library call like `string.find`. |
| `locals.group` | warning | Adjacent same-kind locals (empty tables of the same type) on one line. |
| `locals.mixed` | warning | No local starting `nil` in a statement that initializes others. |
| `guard.else` | warning | No `else` after a branch that returns, breaks, continues, or errors. |
| `guard.nested` | warning | A function or loop body that's one condition wrapped around three or more statements becomes a guard clause and the work. |
| `directives.strict`, `directives.native` | error | `--!strict` and `--!native` at the top, where the caller requires them. |
| `types.any` | error | No `any`, where the caller requires it. |

It costs more than the line-based check it replaced, and checks more. Over the repo's Luau (535 sources, the site's blocks included, 48,455 lines) on a verified release build, counted in retired instructions: the old Rust check took 5,627 per line (32 ms), parsing alone 17,990 per line (100 ms), and the whole check 28,151 per line (153 ms). Two thirds of it is Luau's parser building the tree as tables; the rules are the other third.

There's no comment that turns a rule off, for a line or a file. The style says to fix the code, not to quiet the checker; when the checker is wrong, fix the checker.

Options choose what a file is held to. `directives = { 'strict', 'native' }` and `noAny = true` are for the bundled scripts; `only = { ... }` limits a check to some rules; `declarations` says whether a file is a definitions file, which the checker otherwise works out; and `profile = 'lua'` is s3stash's Lua, where hoisting globals into PascalCase locals (`local Next, Type = next, type`) is the style rather than a mistake and LuaLS `---@param` types feed the truthiness rule.

## The style test

```sh
cargo test --test lua_luau_style
```

`tests/lua/luau_style.rs` activates the built-in package from its source, which also checks that it's the copy the binary embeds, and runs `@jess/style` over every repository-owned `.luau` file under `src/` and `tools/`, over every ```` ```luau ```` block on this site, and over `jess luau-types`' definitions. Any finding, error or warning, fails it. The bundled scripts and built-in modules are held to `--!strict`, `--!native`, and no `any`; the `jess init` template to `--!strict`.

The same test runs the checker's own suite (`jess package test jpk/builtin`, from the source and packed) and type-checks the package against the types folder. Canonical formatting will join this gate once Jess's built-in Luau formatter owns it.
