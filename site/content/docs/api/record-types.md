---
title: 'Record and NIF Types'
description: 'The generated record and NIF type definitions, and how to get them.'
weight: 130
extra:
  kind: api
---

{{ api_signature(value="jess luau-types <FOLDER>") }}

`jess luau-types` writes the Luau types for the whole API into a folder. Its definitions file, `jess.d.luau`, has two halves: the hand-written half (every module and handle on these pages) and a generated half with a type for every plugin record and every NIF record. This page explains the generated half so you can read it; the file itself is the reference.

## Getting the types

```sh
jess luau-types types
```

Point your editor at `types/jess.d.luau` (in VS Code with luau-lsp, add it to `luau-lsp.types.definitionFiles`), and alias `jess` and `dream` to `types/jess` and `types/dream` in your `.luaurc`, so `require('@jess/...')` finds its stub. Then start scripts with `--!strict`. `jess init` does all of this for a script folder: it writes `types/`, a strict `.luaurc` with the aliases, and the luau-lsp settings for VS Code, Neovim, Zed, or Helix.

Regenerate the folder after upgrading jess (rerunning `jess init` refreshes it), since it describes the jess that wrote it: the definitions' first line names the version. It isn't shipped with jess or kept in the repository, because a copy that can drift from the binary is worse than none.

The output has three parts, in order:

1. The API: `Plugin`, `Record`, `Selection`, `Nif`, `NifRecord`, the report types, `List<T>`, the [downstream globals](@/docs/api/downstream-globals.md), `arg`, `require`, and `declare jess: { ... }`.
2. `-- BEGIN GENERATED RECORD TYPES` to `-- END GENERATED RECORD TYPES`: plugin record types, generated from the Rust record types (every `#[esp_meta]` type in `jess-omwaddon`, rendered by `omwaddon::luau`).
3. `-- BEGIN GENERATED NIF TYPES` to `-- END GENERATED NIF TYPES`: NIF record types, generated from `jess-nif`'s record structs by the `LuauType` derive in `jess-macros` (rendered by `jess_nif::luau`).

Because the record types come from the same Rust types jess encodes and decodes with, they can't drift from what scripts actually see.

## Plugin record types

Every record type gets three types. For `LIGH`, whose type name is `Light`:

| Type | What it is | Where you meet it |
| --- | --- | --- |
| `Light` | The record as a plain table, tagged `type = "Light"`. | `record:decode()`, `record:toTable()`, `plugin:insert`, `record:set`. |
| `LightView` | The record's fields as live views, plus `toTable`. | Fields of a handle cast to `LightRecord`. |
| `LightRecord` | `RecordHandle & LightView`: the handle's methods, plus the record's fields. | `local light = record :: LightRecord`. |

```luau
type Light = {
  type: 'Light',
  flags: ObjectFlags,
  id: string,
  name: string,
  script: string,
  mesh: string,
  icon: string,
  sound: string,
  data: LightData,
}

type LightRecord = RecordHandle & LightView

type LightView = {
  flags: ObjectFlags,
  id: string,
  name: string,
  script: string,
  mesh: string,
  icon: string,
  sound: string,
  read data: LightDataView,
  write data: LightDataView | LightData,
  toTable: (self: LightView) -> Light,
}

type LightData = {
  weight: number,
  value: number,
  time: number,
  radius: number,
  color: { number },
  flags: LightFlags,
}

type LightDataView = {
  weight: number,
  value: number,
  time: number,
  radius: number,
  read color: List<number>,
  write color: List<number> | { read [number]: number },
  flags: LightFlags,
  toTable: (self: LightDataView) -> LightData,
} & ViewIteration

type LightFlags = string -- flag names (or hex for unnamed bits) joined by " | ": DYNAMIC, CAN_CARRY, NEGATIVE, FLICKER, FIRE, OFF_BY_DEFAULT, FLICKER_SLOW, PULSE, PULSE_SLOW
```

Nested structs follow the same pattern (`LightData` and `LightDataView`), and a list in a view is a [List](#list-t).

- **Reading and writing.** A struct or list field reads as its view, and takes a whole table too: `read data: LightDataView` and `write data: (LightDataView | LightData)`, so `light.data = { ... }` type-checks. A list takes any array of fitting items (`{ read [number]: number }`).
- **Iterating.** `ViewIteration` gives a struct view inside a record (`LightDataView`) `for name, value in view do` and `#view`, as it has at runtime; a `List` iterates by its indexer. A record handle's own view (`LightView`) has neither, since a handle doesn't iterate: copy it with `toTable()` first.
- **Handles and `toTable`.** `RecordHandle` is what every handle has, its methods; `Record` adds a `toTable` returning [RecordValue](#recordvalue-and-the-tag-names), and a `LightRecord` gets `LightView`'s, returning `Light`. A `LightRecord` still goes wherever a `Record` does.

A `CELL` shows the exceptions, the fields that stay plain tables even in a view:

```luau
type CellView = {
  flags: ObjectFlags,
  name: string,
  read data: CellDataView,
  write data: CellDataView | CellData,
  region: string?,
  read mapColor: List<number>?,
  write mapColor: (List<number> | { read [number]: number })?,
  waterHeight: number?,
  read atmosphereData: AtmosphereDataView?,
  write atmosphereData: (AtmosphereDataView | AtmosphereData)?,
  references: { Reference },
  toTable: (self: CellView) -> Cell,
}
```

`references` is `{ Reference }`, not a `List`: it's a copy (see [Copied fields](@/docs/api/record.md#copied-fields)).

### Shapes of values

| Rust value | Luau type | Example |
| --- | --- | --- |
| Optional field | `T?` | `region: string?` |
| Bitflags | `string`, flag names joined by `" \| "`; bits without a name are written in hex | `"DYNAMIC \| FIRE \| 0x1000"` |
| C-like enum | a union of string literals | `type DialogueType2 = "Topic" \| "Voice" \| "Greeting" \| "Persuasion" \| "Journal"` |
| Other enums | a union of tables tagged by `type` | `type AiPackage = AiTravelPackage \| AiWanderPackage \| ...`, with `AiTravelPackage = { type: "Travel", ... }` |
| An item missing from an array | `JessNull` ([jess.null](@/docs/api/jess.md#jess-null)) | `children: { number \| JessNull }` |
| Raw byte blobs | `string`, base64 | a script's `bytecode` and `variables` |
| Fixed arrays, vectors | `{ number }` (a `List<number>` in a view) | a light's `color` |

Flags are case-sensitive names; an unknown name is an error (`unrecognized named flag`).

Text comes in two kinds. Names, ids, and paths end at their first NUL, and write back with one. Free text keeps every byte: a book's `text`, the `description` of a class, magic effect, race, or skill, a string game setting's `value.data`, a dialogue response's `text` and `scriptText`, and a script's `text`. The Construction Set writes those without a NUL terminator and plenty of other tools write one, so a string from the second kind of plugin ends in `\0`. Leave it there and the record writes back byte for byte; `display()` trims it for reading.

### RecordValue and the tag names

{{ api_signature(value="type RecordValue = TES3Object") }}

`TES3Object` is the union of every record table, each a flat table with its `type` name as a field. It's what `record:decode()`, `selection:decode()`, and `record:toTable()` return, so narrow it by `type` before touching fields; a plain `for` loop over a list of them type-checks:

```luau
--!strict
local record = jess.plugin.open('MyMod.esp'):find('NPC_', 'test_npc') :: Record
local value = record:decode()
if value and value.type == 'Npc' then print(value.name, value.data.level) end
```

Record handles and selectors use four-letter tags; tables use type names. `plugin:insert` takes either.

| Tag | Type | Tag | Type | Tag | Type |
| --- | --- | --- | --- | --- | --- |
| `TES3` | `Header` | `STAT` | `Static` | `APPA` | `Apparatus` |
| `GMST` | `GameSetting` | `DOOR` | `Door` | `LOCK` | `Lockpick` |
| `GLOB` | `GlobalVariable` | `MISC` | `MiscItem` | `PROB` | `Probe` |
| `CLAS` | `Class` | `WEAP` | `Weapon` | `INGR` | `Ingredient` |
| `FACT` | `Faction` | `CONT` | `Container` | `BOOK` | `Book` |
| `RACE` | `Race` | `CREA` | `Creature` | `ALCH` | `Alchemy` |
| `SOUN` | `Sound` | `BODY` | `Bodypart` | `LEVI` | `LeveledItem` |
| `SNDG` | `SoundGen` | `LIGH` | `Light` | `LEVC` | `LeveledCreature` |
| `SKIL` | `Skill` | `ENCH` | `Enchanting` | `CELL` | `Cell` |
| `MGEF` | `MagicEffect` | `NPC_` | `Npc` | `LAND` | `Landscape` |
| `SCPT` | `Script` | `ARMO` | `Armor` | `PGRD` | `PathGrid` |
| `REGN` | `Region` | `CLOT` | `Clothing` | `DIAL` | `Dialogue` |
| `BSGN` | `Birthsign` | `REPA` | `RepairItem` | `INFO` | `DialogueInfo` |
| `SSCR` | `StartScript` | `ACTI` | `Activator` | `LUAL` | `ScriptConfigList` |
| `LTEX` | `LandscapeTexture` | `SPEL` | `Spell` | | |

`RecordFields` (what `insert` and `update` take) is looser: `{ type: string?, [string]: unknown }`. `insert` takes a `RecordValue` as well, so a decoded record goes straight in.

### RecordDisplay

{{ api_signature(value="type RecordDisplay = { record: number, type: string, [string]: unknown }") }}

What `record:display()`, `selection:display()`, and `save:values()` return: the readable shape `jess plugin dump` and `jess save dump` print. It isn't generated per type. Field names are camelCase, like the tables above, except in a save-only record's `subrecords`, which are keyed by tag, with `LUAD` Lua data keeping its own keys. Read it; don't write it back.

## List\<T\>

{{ api_signature(value="type List<T, Table = T> = { [number]: T, push, insert, remove, toTable: (self) -> { Table } }") }}

A live list inside a record view. `T` is what reading an item gives; `Table` is what `toTable()` copies each item to, which differs for lists of structs (`List<TravelDestinationView, TravelDestination>`) and of NIF links (`List<NifRecord?, number | JessNull>`: an empty link reads as `nil` from the view and copies as `jess.null`). `#list`, `list[i]`, and `for i, v in list do` all type-check. Its methods are on the [Record](@/docs/api/record.md#views-and-lists) page.

## NIF record types

NIF records follow the same scheme. A table has its base types' fields copied in, so it's one flat table; views build on their bases' `...ViewFields`. For `NiNode`:

```luau
type NiNode = {
  type: 'NiNode',
  index: number?,
  name: string,
  extraData: number?,
  controller: number?,
  flags: number,
  translation: { number },
  rotation: { number },
  scale: number,
  velocity: { number },
  properties: { number | JessNull },
  boundingVolume: NiBoundingVolume?,
  children: { number | JessNull },
  effects: { number | JessNull },
}

type NiNodeRecord = NifRecordHandle & NiNodeView

type NiNodeView = NiNodeViewFields & {
  toTable: (self: NiNodeView) -> NiNode,
}

type NiNodeViewFields = NiAVObjectViewFields & {
  read children: List<NifRecord?, number | JessNull>,
  write children: (
    List<NifRecord?, number | JessNull> | { read [number]: NifRecord | number | JessNull }
  ),
  read effects: List<NifRecord?, number | JessNull>,
  write effects: (
    List<NifRecord?, number | JessNull> | { read [number]: NifRecord | number | JessNull }
  ),
}
```

- **Tables** (`NiNode`) are what `decode()`, `toTable()`, and `jess.nif.dump` give. Links are record indices (`number?`), and an empty link inside an array is [jess.null](@/docs/api/jess.md#jess-null) (`JessNull`). `index` is set in dumps, which list each record with its index.
- **Views** (`NiNodeView`, built from `...ViewFields` so subtypes can extend them) read links as record handles (`NifRecord?`), and take handles or indices when written.
- **Handles**: `NiNodeRecord = NifRecordHandle & NiNodeView`. Cast with `local node = record :: NiNodeRecord`; `node:toTable()` is then a `NiNode`.
- **Names:** types whose names don't start with `Ni` get a `Nif` prefix for their table and view types (`AvoidNode` is `NifAvoidNode` and `NifAvoidNodeView`), while the handle keeps the plain name (`AvoidNodeRecord`).
- **Geometry:** `uvSets` stays a plain table even in a view (`NiGeometryDataView.uvSets: { { { number } } }`): it's copied when read, like a `CELL`'s references, so assign it back after changing it.

`NifRecordValue` is the union of every NIF record table, tagged by `type` (`"NiNode"`), and `NifRecordFields` (what `insert`, `set`, and `update` take) is `{ type: string?, [string]: unknown }`. `insert` and `set` take a `NifRecordValue` too.

```luau
--!strict
local nif = jess.nif.open('rock.nif', { openmw = false })
local root = nif:roots()[1] :: NiNodeRecord
for _, child in root.children do
  if child and jess.meta(child).typeName == 'NiTriShape' then
    local shape = child :: NiTriShapeRecord
    print(shape.name, shape.translation[1])
  end
end
```

## Casting handles

Iterating a plugin or selection yields `Record`, and NIF lookups yield `NifRecord`. Neither type knows the record's own fields, so cast after checking the type, which [jess.meta](@/docs/api/jess.md#jess-meta) knows:

```luau
--!strict
local plugin = jess.plugin.open 'MyMod.esp'
for record in plugin do
  local tag = jess.meta(record).tag
  if tag == 'LIGH' then
    local light = record :: LightRecord
    light.data.radius += 32
  elseif tag == 'NPC_' then
    local npc = record :: NpcRecord
    npc.aiData.fight = 0
  end
end
```

A cast is only a promise to the type checker: casting a `STAT` to `LightRecord` type-checks, and then reading `light.data` gives `nil` at runtime.

{% usage_note(title="The new type solver") %}
The views' `read` and `write` properties and read-only arrays need Luau's new type solver, which luau-analyze uses by default. luau-lsp needs it switched on; the settings `jess init` writes do that ([Editor Setup](@/docs/getting-started/editor-setup.md#the-new-type-solver)).
{% end %}

Related: [Record](@/docs/api/record.md), [jess.nif, Nif, and NifRecord](@/docs/api/nif.md), [Records and Views](@/docs/scripting/records-and-views.md), [The NIF Model](@/docs/nif/nif-model.md).
