---
title: 'Linting Plugins'
description: 'Find evil GMSTs, suspicious overrides, and other problems before your users do.'
weight: 30
extra:
  kind: guide
---

Goal: before you upload a mod, find the problems your players would otherwise find for you. `jess plugin lint` reads the plugin, runs checks, and lists what they found. It never changes anything.

## Lint it

```sh
jess plugin lint BalmoraTweaks.esp
```

```yaml
input: BalmoraTweaks.esp
path: BalmoraTweaks.esp
checks:
- junk_cells
- missing_version
- fogbug
- fogsync
- bloodmoon_dependency
- deprecated_lists
- dialogue_teleports
- expansion_dependency
- modified_info_id
summary:
  errors: 0
  warnings: 1
  info: 1
findings:
- check: missing_version
  severity: info
  message: TES3 description has no Version line
- check: junk_cells
  severity: warning
  message: CELL Balmora at record 6
```

With no checks named, lint runs the **recommended** set: problems that break or change the game for players and rarely happen on purpose. `checks` says exactly which ran, so an empty `findings` is a real all-clear, not a misconfigured command.

Each finding carries its check's severity. `error` means the plugin is broken for players (a missing master, or expansion functions without the expansion as a master); `warning` is dirt or a likely mistake worth fixing (junk cells, evil GMSTs, fog mismatches); `info` is worth a look but often on purpose (a missing version line, overrides). The table below lists each check's.

Two problems here. The description has no "Version" line, so nobody can tell which version they have. And the plugin carries a copy of one of Balmora's exterior cells with nothing in it but its name and grid: a junk cell that will override any mod that does change it. [Clean it](@/docs/guides/cleaning-plugins.md) and add a version:

```sh
jess plugin clean --junk-cells BalmoraTweaks.esp
jess plugin header --description '+\nVersion 1.0' BalmoraTweaks.esp
```

## Pick the checks

```sh
jess plugin lint --all MyMod.esp                            # everything
jess plugin lint --evil-gmsts --overrides MyMod.esp         # just these
jess plugin lint --recommended --overrides MyMod.esp        # recommended, plus overrides
```

Named checks and presets add up. `--all` is noisy by design: it includes checks like `overrides` that list things you almost certainly did on purpose.

## Every check

✓ marks the recommended ones. "Masters" means the check reads the plugin's masters, found as paths or beside the plugin.

| Check | Finds | Why it matters | Severity | ✓ |
| --- | --- | --- | --- | --- |
| `--junk-cells` | Cells with only a name and grid data: no references, lighting, region, or changes. | CS dirt. It overrides every other mod's version of the cell's header for nothing. | warning | ✓ |
| `--missing-version` | A description with no line starting `Version`. | Users and tools can't tell which version they have. | info | ✓ |
| `--fogbug` | Interior cells with fog density 0. | Morrowind.exe renders them as a black void on many cards. OpenMW is fine, your Morrowind.exe players aren't. | warning | ✓ |
| `--fogsync` | Cells whose two fog densities (`DATA` and `AMBI`) disagree. | The cell stores fog density twice; when the copies disagree, different tools and engines can see different fog. | warning | ✓ |
| `--bloodmoon-dependency` | Bloodmoon-only functions in scripts without `Bloodmoon.esm` as a master. | Breaks for players without Bloodmoon, or loads before it. | error | ✓ |
| `--expansion-dependency` | Tribunal or Bloodmoon functions in scripts without either expansion as a master. | The same, for either expansion. | error | ✓ |
| `--deprecated-lists` | Scripts that `AddItem`/`RemoveItem` on leveled lists. | Changes standard leveled lists at runtime, where list mergers can't see or merge it. | warning | ✓ |
| `--dialogue-teleports` | Dialogue results that `Position` or `PositionCell`. | Teleporting from dialogue is a classic source of broken quests and stuck players. Worth a second look. | warning | ✓ |
| `--modified-info-id` | The same dialogue text under two different `INFO` IDs. | Usually a line copied and re-IDed by accident, so it shows up twice. | warning | ✓ |
| `--duplicate-records` | Two records with the same type and ID in one plugin. | The later one wins; the first is dead weight or a mistake. | warning | |
| `--evil-gmsts` | Evil GMSTs: tes3cmd's list of 11 Tribunal and 61 Bloodmoon GMSTs (`sMaxSale`, `fWerewolfHealth`, ...) still at the expansion's value. IDs match ignoring case; tes3cmd's `--gmsts` works too. | CS dirt: the CS copies them into plugins made with the expansions loaded, and they override other mods' settings. | warning | |
| `--overrides` | Every record that overrides a master's record. Masters. | An inventory of what you changed in the base game. Expect a lot. | info | |
| `--modified-info` | `INFO`s that change a master's `INFO`. Masters. | Editing vanilla dialogue is how mods break each other's quests. | warning | |
| `--duplicate-info` | Duplicate `INFO` IDs. | Only one of them will be used. | warning | |
| `--missing-author` | An empty author field. | Credit where it's due. | info | |
| `--missing-description` | An empty description. | Nobody knows what the plugin is for. | info | |
| `--missing-master` | Masters that can't be found. Masters. | The plugin won't load without them. | error | |
| `--master-sync` | Master sizes that don't match the files. Masters. | Morrowind.exe warns about changed masters; fix with [`header --synchronize`](@/docs/guides/masters-and-headers.md). | warning | |
| `--record-count` | A header record count that doesn't match the records. | Cosmetic for OpenMW; some older tools trust it. | warning | |
| `--cell-params` | Repeated cell header subrecords before the references. | CS quirk; harmless but dirty. | warning | |
| `--autocalc-spells` | Spells flagged autocalc. | NPCs get them automatically; make sure that's what you meant. | info | |
| `--cell-00` | Exterior cells at grid 0,0. | Often an accidental copy of the cell the CS opens first. | warning | |
| `--getsoundplaying` | Scripts using `GetSoundPlaying`. | Commonly misused; check each use. | info | |
| `--menumode` | Scripts that never mention `MenuMode`. | Such scripts keep running while menus are open, which is rarely intended. | info | |
| `--scripted-doors` | Doors with scripts. | Door scripts often take over activation; check they still let the player through. | info | |
| `--no-bloodmoon-functions` | `Bloodmoon.esm` as a master, with no Bloodmoon functions found. | A master dependency you might not need. | info | |
| `--no-tribunal-functions` | `Tribunal.esm` as a master, with no Tribunal functions found. | The same for Tribunal. | info | |

Lint's function detection is text-based and conservative. It looks for a handful of telltale functions (`ToggleCollisionGrid` and `GetSoundPlaying` for Bloodmoon, `StartScript` and `ModRegion` for Tribunal), so the dependency checks are hints, not proof.

## Lint in CI

`plugin lint` exits `1` when it finds an `error`, after printing the report (`error: 1 error was found`), and `0` otherwise. So the plain command fails a build on a plugin that's broken for players, and not on a missing version line:

```sh
jess plugin lint MyMod.esp
```

`--fail-on` moves the line: `--fail-on warning` also fails on dirt and likely mistakes, `--fail-on any` on every finding, and `--fail-on never` never fails, for a report you only read. Choosing the checks still decides what's looked for. To allow particular findings, filter the document instead; the pipeline's status is then `jq`'s:

```sh
jess plugin lint --all --fail-on never --format json MyMod.esp \
  | jq -e '[.findings[] | select(.check != "overrides")] == []'
```

Each finding has a `check` code (snake_case, like `missing_version`), a `severity`, and a `message` naming the record and its index; `summary` counts `errors`, `warnings`, and `info`.

## lint vs validate

`plugin lint` asks "is this plugin *sensible*?". [`plugin validate`](@/docs/cli/plugin.md#validate) asks "is this plugin *in the right format*?":

```sh
jess plugin validate --profile tes3 MyMod.omwaddon
```

Every profile checks the file's structure: a readable `TES3` header first, master entries in pairs, record tags that are text. The `tes3` profile adds every record type and subrecord that Morrowind.exe and the Construction Set don't know: OpenMW-only records and extensions. Use it when you want a plugin to work in both engines. The default, `omwaddon`, adds what OpenMW's loader refuses or skips: record types it doesn't load, `INFO`s with no `DIAL` before them, and records it can't read. The report's `checks` names what the profile checked, and `validate` exits `1` when the plugin doesn't fit.

## Related

- [`plugin lint`](@/docs/cli/plugin.md#lint): the flags.
- [Cleaning Plugins](@/docs/guides/cleaning-plugins.md): removing what lint finds.
- In Luau, `plugin:lint()` runs the same checks and returns the findings as a table; see the [plugin API](@/docs/api/plugin.md).
