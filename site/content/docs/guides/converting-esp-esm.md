---
title: 'Converting ESP and ESM'
description: 'Flip a plugin to a master and back.'
weight: 60
extra:
  kind: guide
---

Goal: turn `BalmoraTweaks.esp` into `BalmoraTweaks.esm`, or back again.

```sh
jess plugin esm BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
file_type: master
output: BalmoraTweaks.esm
written:
  path: BalmoraTweaks.esm
```

```sh
jess plugin esp BalmoraTweaks.esm
```

```yaml
input: BalmoraTweaks.esm
path: BalmoraTweaks.esm
file_type: plugin
output: BalmoraTweaks.esp
written:
  path: BalmoraTweaks.esp
```

The original stays where it was; the converted copy is a new file beside it. That also means converting back needs the old file out of the way: with `BalmoraTweaks.esp` still there, `plugin esp BalmoraTweaks.esm` refuses (`BalmoraTweaks.esp already exists; use --overwrite to replace it`). `--dry-run` reports the `output` it would write without writing it.

## What changes

One field: the header's file type (plugin or master). Every record, the author, the description, and the masters are copied byte for byte. `jess plugin compare` on the two files shows the first difference at byte 28, inside the header (and exits `1` with `error: files differ`, because they do):

```yaml
left: BalmoraTweaks.esp
right: BalmoraTweaks.esm
identical: false
left_bytes: 940
right_bytes: 940
first_difference: 28
```

The extension is part of the job too: `plugin esm` only takes a `.esp` file and `plugin esp` only a `.esm` (`error: input must be a .esp file`). Neither takes an `.omwaddon`, which has no master twin.

## Where the output goes

Without `--output`, the converted file is written **next to the input**, with the other extension, as tes3cmd does. Convert a plugin where it's installed and the `.esm` lands in the mod's folder beside it; `written.path` names it.

`--output` puts it anywhere else, under any name:

```sh
jess plugin esm --output ~/Mods/BalmoraTweaks/BalmoraTweaks.esm BalmoraTweaks.esp
```

If the output already exists, jess refuses (`BalmoraTweaks.esm already exists; use --overwrite to replace it`). `--overwrite` replaces it, after backing it up (`BalmoraTweaks.esm.bak`, reported as `written.backup`). The output can't be the input.

## Why you'd want to

- **The CS and masters.** The Construction Set only lets a plugin depend on masters. To build a plugin on top of someone else's `.esp`, convert it to `.esm` for the CS session, then set your plugin's master back to the `.esp` name (see [Masters and Headers](@/docs/guides/masters-and-headers.md)).
- **Editing a master in the CS.** The CS won't open a master as the active file. Convert to `.esp`, edit, convert back.
- **Load order in Morrowind.exe.** The original engine loads masters before plugins, whatever order you list them. ESM-ifying a plugin forces it earlier.

## OpenMW caveats

OpenMW loads content in exactly the order `openmw.cfg` lists it; the master flag doesn't move anything. Converting doesn't change your load order there, and renaming the file changes its `content=` name, so update that line, in the launcher or from a script (`openmw:deactivate('X.esp')`, `openmw:activate('X.esm')`, `openmw:save()`).

Other plugins name their masters by file name. Rename `Foo.esp` to `Foo.esm` and everything that depended on `Foo.esp` is now missing a master. Convert the dependents' master entries too, or keep both files around while you work.

## Related

- [`plugin esp`, `plugin esm`](@/docs/cli/plugin.md#esp): the flags.
- In Luau: `plugin:setHeader { fileType = "master" }`; see the [plugin API](@/docs/api/plugin.md).
