---
title: 'Load-Order Scripts'
description: 'Patchers that read a whole load order and write a plugin back: the (state, openmw) shape, a driver to run them, and a complete example.'
weight: 40
extra:
  kind: guide
---

A load-order script is a patcher you rerun whenever your mod list changes. It reads the whole load order and writes a plugin back. The shape jess uses for them is a module that returns a function taking `(state, openmw)`, and it's the shape the [load-order tools](@/docs/bundled-scripts/_index.md) in Dreamwind have, so yours runs beside them without ceremony.

The CLI doesn't read `openmw.cfg`. A small driver script does: it loads the config, requires the patchers, and calls them in order.

## The shape

```luau
--!strict
-- lightbooks.jsl
return function(state: { [string]: unknown }, openmw: OpenMwContext)
  print(`light books: {#openmw:content()} content files`)
end
```

- `state` is one table shared by everything the driver calls. Patchers leave results there for the ones after them, and read their options from it.
- `openmw` is the loaded config: its content list, data folders, `data-local`, and VFS. [OpenMW and the VFS](@/docs/scripting/openmw-and-vfs.md) covers all of it.

## The driver

```luau
--!strict
-- patch.jsl <openmw.cfg>
local config: string = ...
local openmw = jess.openmw.load(config)
local lightBooks = require './lightbooks'
local tes3merge = require '@dreamwind/tools/tes3merge'
local state = {}

lightBooks(state, openmw)
tes3merge(state, openmw)
```

```sh
jess run --package jess/jpk/dreamwind patch.jsl ~/.config/openmw/openmw.cfg
```

`require('./lightbooks')` loads `lightbooks.jsl` from beside the driver; `--package` makes Dreamwind's tools `require`-able by name. If one call raises an error, the run stops there, and anything earlier calls wrote stays written.

## A complete patcher

Here's a whole patcher, end to end. **Light Books** makes sure no book in the load order weighs more than 1, by writing a plugin that overrides every book that does. Whatever mods add or change books, the winning version gets patched.

```luau
--!strict
-- Light Books: no book in the load order weighs more than MaxWeight.
local MaxWeight = 1
local Output = 'Light Books.omwaddon'

type Winner = { book: BookRecord, source: string }

return function(state: { [string]: unknown }, openmw: OpenMwContext)
  -- Every active plugin but our own output, in load order.
  local names: { string }, paths: { string } = {}, {}

  for _, content in openmw:content() do
    local isPlugin = not string.find(string.lower(content.name), '%.omwscripts$')
    if not content.path or not isPlugin or content.name == Output then continue end

    table.insert(names, content.name)
    table.insert(paths, content.path)
  end

  -- Only books: everything else is skipped while reading, which is most of the file.
  local plugins, failures = jess.plugin.openAll(paths, { types = { 'BOOK' }, openmw = false })

  -- The last plugin to define a book wins, as in the game.
  local winners: { [string]: Winner }, order: { string } = {}, {}
  local opened = 1

  for index, path in paths do
    if failures[path] then
      print(`light books: skipping {names[index]}: {failures[path]}`)
      continue
    end

    local plugin = plugins[opened]
    opened += 1

    for record in plugin:select({ type = 'BOOK' }):records() do
      local book = record :: BookRecord
      local key = string.lower(book.id)
      if not winners[key] then table.insert(order, key) end

      winners[key] = { book = book, source = names[index] }
    end
  end

  local patch =
    jess.plugin.new { author = 'Light Books', description = 'Nothing weighs more than a feather.' }
  local used: { [string]: boolean } = {}

  for _, key in order do
    local winner = winners[key]
    local value = winner.book:decode()
    if not value or value.type ~= 'Book' or value.data.weight <= MaxWeight then continue end

    value.data.weight = MaxWeight
    patch:insert(value)
    used[winner.source] = true
  end

  local changed = patch:select({ type = 'BOOK' }):count()
  if changed == 0 then
    print 'light books: nothing to patch'
    return
  end

  -- Masters in load order, by name: their sizes come from the load order's files.
  local masters: { string } = {}

  for _, name in names do
    if used[name] then table.insert(masters, name) end
  end

  patch:setMasters(masters, { openmw = openmw })

  local folder = openmw.writableDataDir
    or error 'light books: openmw.cfg has no data-local= or data= folder'
  local written = patch:save(jess.fs.join(folder, Output), { backup = false })
  print(`light books: {changed} books lightened, written to {written.path}`)

  state.lightBooks = { path = written.path, changed = changed }
end
```

Over the three vanilla masters:

```text
$ jess run --package jess/jpk/dreamwind patch.jsl ~/.config/openmw/openmw.cfg
light books: 315 books lightened, written to /home/you/.local/share/openmw/data/Light Books.omwaddon
tes3merge: wrote 22 merged records to /home/you/.local/share/openmw/data/Merged Objects.esp
```

The moving parts, in order:

1. **The content list.** `openmw:content()` is every active content file in load order, each with its `name` and the `path` it resolved to (`nil` when it isn't in any data folder). `.omwscripts` files are content too, so skip them. Skip your own output, or the second run patches the first run's patch.
2. **Opening fast.** `jess.plugin.openAll` opens every plugin in parallel, and `types = { "BOOK" }` keeps only books, skipping everything else while reading. It returns the plugins that opened, in order, and a table of why the others didn't, by path. `openmw = false` because these are already paths; there's nothing to look up.
3. **Winners.** A later plugin's record replaces an earlier one with the same id, so the last definition seen is the one the game uses. Ids are case-insensitive in Morrowind: key them lowercase. The `order` list keeps the output in a stable order; iterating a table with `pairs` wouldn't.
4. **Copying records.** `book:decode()` returns the record as a plain table, including its `type`, which is what `insert` needs. Narrowing on `value.type == "Book"` tells the type checker which record it is, and `insert` takes the decoded record as it is.
5. **Masters.** The patch needs every plugin whose records it overrides as a master, in load order, with the size of the file it was made against. Give `setMasters` the names and the load order's context, and it looks each file up there for its size.
6. **Writing.** The output goes to `writableDataDir`: `data-local`, else the first `data=` folder, the same place the load-order tools write. Set `data-local=` if you can: OpenMW loads it after every `data=` folder, and it keeps generated files out of Morrowind's own `Data Files` and out of any mod's folder, where the mod's next update would clobber them.

## Passing results along: `state`

`state` is how patchers talk to each other. Light Books leaves a summary in `state.lightBooks`, and anything the driver calls after it can read it:

```luau
--!strict
return function(state: { [string]: unknown }, _openmw: OpenMwContext)
  type LightBooks = { path: string, changed: number }

  local result = state.lightBooks :: LightBooks?
  if result then print(`the previous script changed {result.changed} books`) end
end
```

The same table carries options *into* patchers. Dreamwind's tools read theirs from a table named after them (`state.tes3merge`, `state.s3lightfixes`, `state.mergedlands`), so the driver configures them by filling it before the call:

```luau
local state = {
  tes3merge = { ignoreRecords = { 'STAT' }, activate = true },
}
```

Afterwards each leaves a report in the same table (`state.tes3merge.report`), for whatever runs next. Pick a key named after your patcher for your own entries, so patchers don't trample each other.

## Turning the output on

Writing the plugin doesn't make OpenMW load it: it has to be in the content list. A patcher can activate it itself:

```luau
--!strict
return function(_state: { [string]: unknown }, openmw: OpenMwContext)
  if not openmw:activate 'Light Books.omwaddon' then return end

  local written = openmw:save()
  print(`light books: activated, backed up openmw.cfg to {written.backup}`)
end
```

`activate` returns whether the content list changed, and `save` writes it to the user's `openmw.cfg`, after backing the old one up as `openmw.cfg.bak` (then `.bak1`, ...), and returns `{ path, backup }`. `activate` refuses a file that isn't in one of the data folders, read afresh, so the file you wrote a moment ago counts and a typo doesn't end up in the load order. `{ force = true }` activates it anyway, for when you know better than the data folders. Make this opt-in: plenty of people manage their content list with a mod manager or the launcher and won't thank you for editing it behind their back. That's why the load-order tools only do it when asked (`activate = true`, `autoEnable = true`).

## Shipping a patcher with a mod

Put the module under your mod's data folder, say `scripts/mymod/patch.jsl`. A driver that makes the config its default can `require` it by its VFS name, so it can sit in a loose folder or inside a BSA, and a later mod can replace it the same way it would replace a mesh:

```luau
--!strict
local config: string = ...
jess.openmw.configPath(config)

local openmw = jess.openmw.load(config)
local patch =
  require 'scripts.mymod.patch' :: (state: { [string]: unknown }, openmw: OpenMwContext) -> ()

patch({}, openmw)
```

Split a big patcher into modules and load them with relative `require`s; see [OpenMW and the VFS](@/docs/scripting/openmw-and-vfs.md) for how `require` finds them.

## Pitfalls

- **Testing against your real setup.** A test config that chains to your real one with `config=` still counts your real `openmw.cfg` as the user's config: `openmw:save()` writes it, and `userConfigDir` points at its folder. For a safe test, copy the `data=` and `content=` lines into a standalone config in a scratch folder with its own `data-local=`, and hand the driver that.
- **Order matters.** A patcher sees `state` as the ones before it left it, and the content list as the context has it. A plugin written by an earlier patcher isn't in `openmw:content()` unless that patcher activated it.
- **One run, one state.** `state` doesn't persist between runs. Anything that should, write to a file.

## Related

- [Performance](@/docs/scripting/performance.md): making scripts like this fast on 500-plugin load orders.
- [Creating Plugins](@/docs/scripting/creating-plugins.md): more on `jess.plugin.new`, masters, and copying records.
- [`jess run`](@/docs/cli/run.md) and the [OpenMW context reference](@/docs/api/openmw.md).
