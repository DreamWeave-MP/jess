---
title: 'Scripting'
description: 'Write Luau scripts against the whole jess API: over your load order, over one file, or standalone.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 40

extra:
  kind: guide
---

jess is a Luau runtime with plugins, saves, NIFs, and OpenMW configs a call away. These pages teach it in order; the [API reference](@/docs/api/_index.md) is where you look things up afterwards.

A script runs one of three ways:

- **Standalone**: `jess run script.luau args...`. Anything goes.
- **Over your load order**: a module returning a function of `(state, openmw)`, called by a script that loaded your `openmw.cfg` with `jess.openmw.load`. This is how patchers work, including the [load-order tools](@/docs/bundled-scripts/_index.md).
- **Over one file**: `jess plugin modify --script` or `jess nif modify --script`, which hand your script one plugin or mesh and save it if you changed it.

Start with [Your First Script](@/docs/scripting/intro.md), then [Records and Views](@/docs/scripting/records-and-views.md) and [Selections](@/docs/scripting/selections.md): almost every script is built from those. The rest you can read when you need it. Meshes have their own section, the [NIF deep dive](@/docs/nif/_index.md).
