# Save strip and save check

`jess save strip` removes content files from an OpenMW save; `jess save check` reports whether a save loads cleanly with the current load order. Both are built on the same rules: what the engine does when a save names a record that no longer exists. Citations are to the local OpenMW checkout (`openmw/`, gitignored).

## Using them

```sh
jess save check MySave.omwsave                     # damage, missing content, broken state
jess save strip MySave.omwsave Bloodmoon.esm Tribunal.esm   # edits in place, keeps MySave.omwsave.bak
jess save strip --dry-run MySave.omwsave Tribunal.esm       # shows what would change
jess save strip --output Vanilla.omwsave MySave.omwsave Tribunal.esm
```

- Several content files can be stripped at once, in any order: dependents are removed before their masters.
- Editing in place keeps every earlier backup (`.bak`, `.bak1`, …). A blocked strip writes nothing and exits non-zero.
- Removing a master that other content in the save still needs is blocked, naming the dependents.
- The content file being removed doesn't have to be installed. When it's gone (the usual case: the mod was uninstalled mid-playthrough), its records are taken to be the ones the save refers to that no remaining content file and none of the save's own dynamic records define, which is what `save check` reports missing. That needs every remaining content file readable through the config; if one isn't (or there's no config), those ids are `target_only_unknown` and the strip is blocked, unless the unreadable ones are being removed in the same strip. Objects the removed file placed are found by their RefNum's content file index either way.
- Anything that stops a strip shows in `blockers.hard`: unsupported save formats, Lua data jess can't read, and the fields below.
- `save check` exits non-zero only when the save can't load (damaged, or a missing birthsign/race/class).

## Why only some fields matter

OpenMW tolerates a missing plugin: it warns, then drops every RefNum into it (`esmreader.cpp:90-100`), every object whose base record is gone (`cellstore.cpp:1058-1070`), and most dangling ids. A field needs handling only if the engine **keeps** a dangling id and that breaks something. Strip therefore:

1. Rewrites the dependency lists (`TES3.MAST`/`DATA`, `SAVE.DEPE`) and remaps RefNums after the removed index.
2. Fixes, or refuses, the fields below that break a save.
3. Doesn't block on anything the engine already handles.

## Fields that break a save

| Field | Where | Engine outcome | Evidence | Strip | Check |
|---|---|---|---|---|---|
| `SIGN` birthsign | `PLAY` | save fails to load | `mwworld/player.cpp:372-375` | blocks | won't load |
| race `RNAM`, class `CNAM` | saved player `NPC_` | save fails to load (checked when `PLAY` is read) | `mwworld/esmstore.cpp:805`, `worldimp.cpp:456` | blocks | won't load |
| head `BNAM`, hair `KNAM` | saved player `NPC_` | player renders without them | `mwrender/npcanimation.cpp:478-494` | blocks | broken |
| `TNAM` trap spell | object references | activation throws; can never be opened | `mwworld/actiontrap.cpp:26-34` | deletes (disarms) | broken |
| `BNAM` ownership global | object references | every ownership check throws | `mwmechanics/mechanicsmanagerimp.cpp:95-96` | deletes | broken |
| `DNAM` door destination | object references | door does nothing | `mwworld/actionteleport.cpp:37` | blocks (clearing it would make an exterior teleport) | broken |
| `ANAM` owner, `CNAM` faction (+`INDX`) | object references | always theft | `mechanicsmanagerimp.cpp:82-93` | deletes | broken |
| `KNAM` key | object references | key-only lock can't be opened | `mwmechanics/security.cpp:34`, `components/esm3/cellref.cpp:153-158` | deletes from pickable locks; blocks key-only locks | broken (key-only) |
| AI `TARG`/`CELL` | Follow/Escort packages | NPC idles forever | `mwmechanics/aifollow.cpp:99-104`, `aiescort.cpp:93-94` | deletes the package | broken |
| player's current cell | `PLAY` | player moved to exterior (0, 0) | `player.cpp:366-368` | blocks | warning |

Deleting inventory items also renumbers the inventory's `EQIP` and `SELE` item indices (`inventorystate.cpp:149-163`).

## Fields the engine handles

Deleted where strip can, otherwise left for the engine; none of these block.

| Field | Outcome | Evidence |
|---|---|---|
| object / inventory base `NAME` | dropped | `cellstore.cpp:1058-1070` |
| interior / exterior `CSTA` | dropped / blank generated cell | `worldmodel.cpp:528-554` |
| `MVRF` moved references | dropped, object returns home | `cellstore.cpp:1094-1119` |
| spells, used powers, selected spell | dropped | `mwmechanics/spells.cpp:223-249` |
| active spells (`ID__`, `SPID`, `MGEF`, `ARG_`) | dropped on update | `activespells.cpp:243-257` |
| soul `XSOL`, NpcStats `FACT`/`USED` | dropped | `livecellref.cpp:94-99`, `npcstats.cpp:469-502` |
| journal, quests, topics, quick keys, selected spell | dropped | `journalimp.cpp:56-70`, `dialoguemanagerimp.cpp:694-696`, `quickkeysmenu.cpp:646-669` |
| globals, global scripts, death counts, weather regions | dropped | `globals.cpp:101-102`, `globalscripts.cpp:254`, `actors.cpp:2340`, `weather.cpp:1112-1122` |
| last hit, bound items, class name, faction reactions, stolen owners, custom markers | harmless | `creaturestats.cpp:589-590`, `spelleffects.cpp:262-273`, `savegamedialog.cpp:220-226` |
| player faction in the saved `NPC_` | cleared | `esmstore.cpp:115-124` |

## RefNums and Lua

Every RefNum OpenMW writes into saves is classified. `TARG` is a RefNum only in AI Combat/Pursue packages; elsewhere it's a RefId. `LUAD` Lua data is decoded and its object references remapped; unknown userdata blocks the strip.

## Save formats

Strip edits formats `MIN_MUTATION_FORMAT_VERSION`..=`CURRENT_FORMAT_VERSION` (37..=40); `FORMAT_HISTORY` in `libs/plugin/src/lib.rs` records what each version changed. `cargo test -p jess-plugin --test plugin openmw_save::` fails when the local OpenMW checkout moves to a newer format, so a bump can't slip by.
