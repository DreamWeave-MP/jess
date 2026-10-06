---
title: 'Format Notes'
description: 'Byte-identical round trips, OpenMW save format bumps, and other format rules.'
weight: 40
extra:
  kind: guide
---

The rules jess holds itself to about file formats, and the engine behavior behind the less obvious ones. OpenMW source paths here are relative to the OpenMW checkout jess keeps at `./openmw` (gitignored); clone OpenMW there if you're changing anything that predicts engine behavior.

## Byte-identical round trips

Read a file, save it without changes, get the same bytes back. Every byte. This is a hard requirement, not a goal: a change that breaks it doesn't merge.

Why be this strict:

- **Edits should be reviewable.** Change one NPC's name and diff the result, and you should see one NPC's name. A tool that re-encodes everything turns a one-field edit into a thousand-record diff and hides the one change that matters.
- **Vanilla data is weird, and the weirdness is load-bearing.** Strings without NUL terminators, junk in padding, empty subrecords, references in a particular order. Some of it the engine reads, some it doesn't, and you can't always tell which. Keeping it is always safe. "Normalizing" it isn't.
- **Other tools compare bytes.** Load-order tools and mod managers decide what conflicts, what's dirty, and what changed by comparing records. A tool that rewrites records it didn't touch shows up as a conflict everywhere.
- **Saves are the player's.** Stripping a mod out of a save should change what it has to and nothing else.

How it's kept:

- **Plugins and saves**: `jess-plugin` keeps the original buffer and copies untouched records straight out of it. Only edited records are rebuilt. See [Plugins in Rust](@/docs/rust/plugins.md).
- **Typed edits**: `PluginEditor::replace_record` re-encodes only the subrecords an edit changed (`libs/omwaddon/src/writeback.rs`), matches `CELL` references by `FRMR` index so untouched ones keep their bytes and order, and verifies that the result decodes to exactly the new value before using it.
- **NIFs**: Morrowind NIFs save back byte for byte through `NiStream`, and through Luau tables too.

How it's checked: `jess plugin test-codec` (debug builds), and the real-data tests against the retail masters, real saves, and real NIFs. All in [Building and Testing](@/docs/contributing/building-and-testing.md#real-data). If you touch a parser, run them.

What's allowed to normalize: `GeneratedPlugin`, which is for building plugins, not editing them, and the subrecords an edit actually changed. That's it.

## OpenMW save format bumps

OpenMW versions its save format with `CurrentSaveGameFormatVersion` in `components/esm3/formatversion.hpp`. jess tracks it in `jess_plugin::openmw_save` (`libs/plugin/src/openmw_save.rs`):

| Constant | Meaning |
| --- | --- |
| `CURRENT_FORMAT_VERSION` | The newest format jess knows. Must equal OpenMW's `CurrentSaveGameFormatVersion`. Currently 40. |
| `MIN_MUTATION_FORMAT_VERSION` | The oldest format save stripping has been checked against. Currently 37: format 34 and older store actor references as actor ids instead of `RefNum`s, and 36 and older store Lua scripts by path. |
| `FORMAT_HISTORY` | One `FormatChange` per version from `MIN_MUTATION_FORMAT_VERSION` to `CURRENT_FORMAT_VERSION`: the version, the OpenMW commit that introduced it, what changed, and whether it affects save editing. |

`jess save strip` edits formats 37 through 40 and refuses anything else. `jess save check` reports whether a save's format is in that range.

`cargo test -p jess-plugin --test plugin openmw_save::` compares `CURRENT_FORMAT_VERSION` against your `./openmw` checkout, and fails the moment OpenMW moves ahead. It also checks that `FORMAT_HISTORY` covers every version in the range. (Without a checkout, the comparison skips.)

When it fails, update in this order:

1. **Find the commit** that bumped the format:
   ```sh
   git -C openmw log -- components/esm3/formatversion.hpp
   ```
2. **Read what it changed.** Does it touch a subrecord that save check or strip reads or rewrites (references, `RefNum`s, dependencies, Lua script data, cell state, the player)? The code that cares lives in `libs/save`.
3. **Add a `FORMAT_HISTORY` entry**: version, the short commit hash, one line on what changed, and `affects_save_editing`.
4. **Raise `CURRENT_FORMAT_VERSION`.**
5. **If it affects save editing**, update the strip code to handle the new layout. If older layouts can no longer be edited safely by the same code, raise `MIN_MUTATION_FORMAT_VERSION` too, and explain why in its doc comment.

For reference, the current history:

| Version | OpenMW commit | Change | Affects editing |
| --- | --- | --- | --- |
| 37 | `fdc2e63152` | Lua scripts saved by id instead of path (`LUAS`) | no |
| 38 | `4f53438082` | Ingredient effect indices in dynamic `INGR` records | no |
| 39 | `021234fa35` | Projectile wind-up (`PROJ` `WIND` float) | no |
| 40 | `a4091dd052` | Skills and attributes saved by `RefId` (`SKIL`, `ATTR`, `INCR`) | no |

The rules strip follows for each field, with engine citations, are in [`docs/save-strip.md`](https://github.com/DreamWeave-MP/jess/blob/main/docs/save-strip.md).

## Moved references: `MVRF` and `CNDT`

A `CELL` record's references are runs of subrecords, each starting at a `FRMR`. A *moved* reference (one a later plugin relocated to another cell) is preceded by an `MVRF` (the moved reference's number) and optionally a `CNDT` (the exterior cell it moved to):

```text
... FRMR NAME DATA ...  MVRF [CNDT] FRMR NAME DATA ...  FRMR ...
    `-- reference --'   `------ moved reference -----'
```

The pair belongs to the reference **after** it. That's how OpenMW reads it: `ESM::Cell::getNextRef` in `components/esm3/loadcell.cpp` reads an `MVRF` (and `CNDT`, via `getNextMVRF`), then expects the `FRMR` it applies to.

`RawRecord::cell_groups` groups them the same way: a reference group starts at its `MVRF` when it has one. Deleting a moved reference takes its `MVRF`/`CNDT` with it. jess used to attach the pair to the reference *before* it, so deleting that earlier reference took the next reference's `MVRF` with it, and deleting a moved reference left its `MVRF` behind. If you write code that splits or reorders references, use `cell_groups`; don't scan for `FRMR` yourself.

## Why there's no `undelete`

tes3cmd's `undelete` "restores" deleted references by replacing each `DELE` with a `ZNAM` disabled marker and a zeroed `DATA` position, parking the reference at the origin, switched off. jess had a port. It's gone, because on OpenMW it destroys cells:

1. OpenMW's reference loader (`loadDataImpl` in `components/esm3/cellref.cpp`) reads known subrecords in a loop. Anything it doesn't recognize, `ZNAM` included, it takes as the end of the reference and leaves for the caller.
2. The caller, `ESM::Cell::getNextRef` (`components/esm3/loadcell.cpp`), then looks for the next `MVRF` or `FRMR`. It finds a `ZNAM`, so it returns false: no more references.
3. The cell stops loading right there. Every reference after the first `ZNAM` is silently gone.

(OpenMW's `.ess` importer does read `ZNAM`, in `apps/essimporter/importcellref.cpp`, but that's Morrowind's save format, not content files.)

A tool that quietly deletes half a cell's contents while claiming to restore some of them doesn't get to stay. If you want a deleted reference back, restore the reference properly, or delete the override that deleted it.

## `CAN_CARRY` on lights

`LIGH` flag `0x002` (`ESM::Light::Carry` in `components/esm3/loadligh.hpp`, `CAN_CARRY` in jess) doesn't just mean "can be picked up". In OpenMW it decides whether the light is an item at all (`apps/openmw/mwclass/light.cpp`):

- `Light::isItem` returns the flag.
- `Light::insertObjectPhysics` gives only non-carryable lights collision.
- `Light::activate`, `Light::canBeEquipped`, and `Light::showsInInventory` all refuse a light without it.

Flip it on an existing light and every copy already in an inventory, or already placed as scenery, is now the wrong kind of object. That's why `@dreamwind/tools/s3lightfixes` never adds or removes `CAN_CARRY`, even when a light override's `flag` asks for it: the rest of the override applies, and the carry bit stays what the plugin said. The original S3LightFixes would change it; this is one of the documented differences ([S3LightFixes](@/docs/bundled-scripts/s3lightfixes.md)).

## Text encodings

Morrowind content is in one of three Windows code pages: 1252 (English and Western European, the default), 1250 (Central European), or 1251 (Cyrillic). The files don't say which.

- **Raw operations never decode text.** Copying, comparing, deleting, and round-tripping work on bytes, so the encoding can't break them.
- **Typed strings decode with the plugin's `LegacyEncoding`**, set when it's parsed (`--encoding` in the CLI, `encoding` in Luau). Pick the wrong one and names come out as mojibake; save them back and you've written mojibake. Pick the right one.
- **Saves are different.** OpenMW writes ids in saves as typed `RefId`s, not code-page strings, so save ids don't depend on the encoding. The content files a save needs do: `save check` and `save strip` read them in the config's `encoding=`, as the engine does, or `--encoding` when the config doesn't set one, so their ids match the save's.
- **Lint reads masters in the plugin's encoding.** A load order has one code page, so a master's ids only match the plugin's when both are read in it.
- **No byte order marks.** A string that starts with `EF BB BF` (or `FF FE`, `FE FF`) is three (or two) characters in the code page, not a switch to UTF-8 or UTF-16: OpenMW's `ToUTF8` translates every byte through the code page's table, and jess writes those bytes back as they were.

## Record counts

The `TES3` header stores a record count. The OpenMW engine never reads it when loading content; OpenMW-CS reads it to size its loading progress (`CSMWorld::Data::startLoading` in `apps/opencs/model/world/data.cpp`), and other tools read it for their own reasons. Either way it should be right, and it should not change for no reason:

- Removing records (`omwaddon::edit::delete_matching`, `omwaddon::lint::clean`, Luau `plugin:delete`, `clean`, and `selection:delete()`) sets the count to match. Inserting in Luau does too.
- An edit that adds or removes nothing leaves the count alone, even if it was already wrong, so the plugin stays byte for byte. Fixing a count that was wrong before you arrived is its own decision: `jess plugin header --update-record-count`, or `updateRecordCount()` in Luau.
- `delete_matching_with` and `clean_with` take `RecordCount::Keep` for callers who need the old count left exactly as it was.

## Floats in text

Dumps and messages print floats in the shortest spelling that parses back to the same value (`omwaddon::text::float_text`). A value that is exactly an `f32`, which is every float a plugin stores, is spelled as that `f32`, so `0.1` reads `0.1` and not `0.10000000149011612`. Tiny and huge magnitudes switch to scientific notation (`3.140798e-39`), and NaN and infinities read `NaN`, `inf`, `-inf`.

Rounding to a fixed number of places would be tidier and wrong: a dump that rounds can't tell you why two records differ, and a denormal fog density would read as zero or as forty digits of zeros. Neither helps anyone debug a fog bug.

## Unknown flag bits

NPC and creature `FLAG` fields sometimes carry bits nobody has named. jess keeps them through edits, and serde and Luau spell them in hex next to the named ones (`"FEMALE | 0x100"`). Dropping bits you don't understand is how you break something you didn't know existed.
