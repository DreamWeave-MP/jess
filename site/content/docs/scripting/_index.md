---
title: 'Scripting'
description: 'Write Luau against the jess API: scripts, require, the built-in modules, packages, and applications.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 40

extra:
  kind: guide
---

These pages teach the runtime in order; the [API reference](@/docs/api/_index.md) is where you look things up afterwards.

Luau runs under jess in three shapes:

- **A script**: `jess run script.jsl args...`. One VM, the `jess` global, your terminal, and whatever capabilities you grant on the command line.
- **A command target**: a script with a package around it. The manifest names the entry, the layers it activates, and the capabilities it needs; `jess package run` grants exactly those.
- **An application target**: packages loaded into one script domain, driven by frames until it says it is done.

Start with [Your First Script](@/docs/scripting/intro.md), then [Packages](@/docs/scripting/packages.md), which is where almost everything else lives. [Applications](@/docs/scripting/applications.md) is for things that keep running. The rest you can read when you need it.
