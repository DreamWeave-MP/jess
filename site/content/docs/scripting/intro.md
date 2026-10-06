---
title: 'Your First Script'
description: 'Write, type-check, and run a jess script in five minutes.'
weight: 10
extra:
  kind: guide
---

A jess script is a [Luau](https://luau.org) file with the whole jess API already loaded. No imports, no project setup, no build step: write a file, run it. (There is a project setup, `jess init`, and you'll want it for the types. It's one command.)

## Count some books

Save this as `hello.luau`:

```luau
--!strict
local name: string = ... or 'Morrowind.esm'

local plugin = jess.plugin.open(name)
local books = plugin:select({ type = 'BOOK' }):count()

print(`{name} has {books} books`)
```

Run it:

```sh
$ cd ~/Games/Morrowind/Data\ Files
$ jess run ~/my-scripts/hello.luau
Morrowind.esm has 574 books
$ jess run ~/my-scripts/hello.luau Tribunal.esm
Tribunal.esm has 49 books
```

That's a complete script. Three things happened:

- `jess` is a global. Everything lives under it: `jess.plugin`, `jess.save`, `jess.nif`, `jess.openmw`, and friends.
- `"Morrowind.esm"` is a path, relative to where you ran jess, the same as on the command line; hence the `cd`. A script that loads your `openmw.cfg` and makes it the default (`jess.openmw.configPath`) can use bare names the way OpenMW finds them. [Paths, Configs, and Encodings](@/docs/getting-started/openmw-config.md#in-scripts) covers the rules.
- `...` holds the arguments after the script's name, as strings. The global `arg` table has them too, with the script's path at `arg[0]`.

## Turn on the types

That `--!strict` on the first line isn't decoration. It tells Luau to type-check the whole file, and jess ships types for every function, report, record, and NIF block. Strict mode catches the misspelled field before you run the script over 300 plugins, not after, when the only evidence is a patch that quietly did nothing.

The quickest way to get them is a script folder that's already set up:

```sh
jess init my-scripts                    # VS Code; or --editor neovim, zed, helix, none
```

That writes `types/` (the types, from the jess you have installed: `jess.d.luau` and a stub module for each `@jess/` and `@dream/` name), a strict `.luaurc` aliasing those names into `types/`, a `main.luau` to start from, and the settings that point [luau-lsp](https://github.com/JohnnyMorganz/luau-lsp) at the types in your editor. To do it by hand, `jess luau-types types` writes the folder and prints the aliases for your `.luaurc`, and in VS Code:

```json
{
  "luau-lsp.platform.type": "standard",
  "luau-lsp.fflags.enableNewSolver": true,
  "luau-lsp.types.definitionFiles": { "@jess": "types/jess.d.luau" }
}
```

Other editors running luau-lsp take the file as `luau-lsp lsp --definitions:@jess=types/jess.d.luau --flag:LuauSolverV2=true`. The flag turns on Luau's new type solver, which some of jess's types need. Now you get completion on `jess.`, hover docs, and red squiggles where your script is wrong.

{% usage_note(title="Regenerate after upgrading") %}
`types/` describes one jess version. After you upgrade jess, run `jess init` in the folder again (it only refreshes `types/`, leaving your files alone) or `jess luau-types types` again, or the editor will happily approve calls the new version doesn't have.
{% end %}

## Records are handles

`select` hands back record *handles*. A handle is a live window onto one record in the plugin: read a field and you get the plugin's value, write one and the plugin changes. Every name on a handle is a field of its record or one of its methods; what the handle knows about the record itself (its number, tag, and plugin) is `jess.meta(record)`. The record's fields depend on its type, so in typed code you cast the handle to say which type it is:

```luau
--!strict
local plugin = jess.plugin.open 'Morrowind.esm'

local heaviest: BookRecord? = nil
for record in plugin:select({ type = 'BOOK' }):records() do
  local book = record :: BookRecord
  if not heaviest or book.data.weight > heaviest.data.weight then heaviest = book end
end

if heaviest then
  local number = jess.meta(heaviest).index
  print(`{heaviest.name} ({heaviest.id}, record {number}) weighs {heaviest.data.weight}`)
end
```

```
Boethiah's Pillow Book (bk_BoethiahPillowBook, record 11189) weighs 10
```

Every record type has a `...Record` type: `BookRecord`, `LightRecord`, `NpcRecord`, `CellRecord`, and so on. The cast is only for the type checker; it doesn't check or convert anything at runtime, so cast to the type you selected. [Records and Views](@/docs/scripting/records-and-views.md) goes deeper.

## House rules

A handful of conventions hold across the whole API. Learn them once.

- **camelCase everywhere.** Functions, methods, fields, and report codes: `plugin:syncMasters()`, `check.verdict == "willNotLoad"`. Types are PascalCase.
- **Errors raise.** A missing file, a bad regex, a field name the record type doesn't have, or a record that can't be encoded throws. When failure is an outcome you expect, like a file that may not be installed, catch it with `pcall` and carry on:

  ```luau
  --!strict
  -- `open` throws for a plugin that isn't there.
  local ok, problem = pcall(function() return jess.plugin.open 'Not A Real Plugin.esp' end)

  if not ok then print(`couldn't open it: {problem}`) end
  ```

  A caught error is its message, as a string, ready to print or match. An error nothing catches stops the script and prints with a traceback.

  Don't wrap code in `pcall` to find out whether a field exists or a call works. Know the type and use the documented field. A script that swallows its own bugs isn't resilient; it's lying to you.
- **Outcomes aren't errors.** A save that won't load or a strip that's blocked comes back as a report with a verdict, not an exception. Errors mean jess couldn't do what you asked; reports tell you what it found.
- **Record numbers are 0-based; lists are 1-based.** `jess.meta(record).index` matches the numbers `jess plugin dump` prints, and record 0 is the file's `TES3` header, so the first real record is 1. Every list jess returns is an ordinary Luau array starting at 1.
- **Bytes are buffers.** `plugin:bytes()`, `record:bytes()`, and `openmw:readFile(path)` return Luau `buffer`s.
- **Vectors are vectors.** OpenMW Lua data's `util.vector3` comes back as a native Luau `vector`.
- **Holes are `jess.null`.** Where an array has a missing value, like an empty link in a NIF node's children, it holds `jess.null`, because `nil` would shift everything after it.

## Where next

- [Records and Views](@/docs/scripting/records-and-views.md) and [Selections](@/docs/scripting/selections.md): the core of almost every script.
- [Load-Order Scripts](@/docs/scripting/load-order-scripts.md): scripts that run over your whole load order.
- [Per-File Scripts](@/docs/scripting/modify-scripts.md): fix one plugin or mesh with a script, tes3cmd-style.
- The [Luau API Reference](@/docs/api/_index.md), for every signature. The [`jess run` reference](@/docs/cli/run.md) covers the command's flags.
