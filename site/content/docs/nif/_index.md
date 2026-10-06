---
title: 'NIF Deep Dive'
description: 'For people who open meshes in a hex editor for fun: the NIF model, editing, building, optimizing, and every check rule.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 50

extra:
  kind: guide
---

jess reads NIFs from every game OpenMW supports, and reads and writes Morrowind's. This section is for scripters doing mesh work in bulk: thousands of files, one script, no NifSkope. If you only want to know whether your meshes load, the [NIF guide](@/docs/guides/nif-checking-and-fixing.md) gets you there from the command line without any of this.

Read the pages in order the first time. Each one assumes the one before it:

1. [The NIF Model](@/docs/nif/nif-model.md): records, links, and roots, and why only Morrowind's NIFs are editable.
2. [Editing NIFs](@/docs/nif/editing.md): record handles, live views, links, and saving without surprises.
3. [Building From Scratch](@/docs/nif/building-from-scratch.md): a textured, colliding mesh from nothing but Luau.
4. [Optimization Cookbook](@/docs/nif/optimization-cookbook.md): batch recipes over a folder, a BSA, or the whole load order, with real numbers from vanilla Morrowind.
5. [Check Rules](@/docs/nif/checking-rules.md): every rule `jess nif check` enforces, the OpenMW function behind it, and what `nif fix` repairs.
