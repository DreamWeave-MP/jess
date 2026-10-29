---
title: 'Getting Started'
description: 'Install jess, write a script, make it a package, and set up an editor that knows the types.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 10

extra:
  kind: guide
---

Start with a script, then make an application you can distribute. Each journey builds on the one before it; the [API](@/docs/api/_index.md) and [CLI](@/docs/cli/_index.md) references are there when you need a precise contract.

## First experience

1. [Install Jess](@/docs/getting-started/install.md) and check its version.
2. Write and check [your first script](@/docs/scripting/intro.md). If Luau is new, read [Enough Luau to Start](@/docs/getting-started/luau-basics.md) alongside it.
3. Learn what Jess adds in [JSL and L3i](@/docs/scripting/jsl-and-l3i.md), and [set up your editor](@/docs/getting-started/editor-setup.md) with the generated types.
4. Read [The Jess Mental Model](@/docs/getting-started/mental-model.md) and [Capabilities and Trust](@/docs/getting-started/authority.md) before granting code access to your machine.

## Build and distribute

1. Make [your first package](@/docs/getting-started/first-package.md), including a module, a test, a command target, and a JPK.
2. Build [your first application](@/docs/getting-started/first-application.md) and learn [its lifecycle and state](@/docs/getting-started/application-lifecycle.md).
3. Take it [from workspace to distribution](@/docs/getting-started/first-workspace.md), then use [the two-package recipe](@/docs/cookbook/multi-package.md) to resolve and lock a dependency offline.
4. Use [Checking and Testing](@/docs/getting-started/testing.md) and [Deployment and Operations](@/docs/reference/deployment.md) to verify and run the result on its destination.

## Go deeper

[Building a Larger Application](@/docs/scripting/learning-path.md) follows state ownership, events, workers, I/O, graphics, audio, and profiling. [Modules, Layers, and Dependencies](@/docs/getting-started/modules-and-layers.md) explains composition; [Read Another Package](@/docs/getting-started/reading-a-package.md) shows how to inspect one. The [cookbook](@/docs/cookbook/_index.md) has complete small jobs, and [Debugging Jess Code](@/docs/getting-started/debugging.md) starts from common failures. [Choose Your Next Project](@/docs/getting-started/next-steps.md) points to further routes.
