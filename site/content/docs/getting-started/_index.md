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

Four steps, each one short. Nothing here writes outside the folder you point it at.

1. [Install](@/docs/getting-started/install.md) the binary.
2. Write [your first script](@/docs/scripting/intro.md): `jess init`, one file, `jess run`. New to the language? Read [Enough Luau to Start](@/docs/getting-started/luau-basics.md) alongside it.
3. Build [your first package](@/docs/getting-started/first-package.md): a manifest, a module, a test, a command target, packed and run.
4. [Set up your editor](@/docs/getting-started/editor-setup.md) with the types, in VS Code, Neovim, Zed, or Helix.

Before diving further, read [The Jess Mental Model](@/docs/getting-started/mental-model.md) for the distinctions between packages, layers, targets, capabilities, and domains. Then [Scripting](@/docs/scripting/_index.md) explains how the pieces fit, and the [API](@/docs/api/_index.md) and [CLI](@/docs/cli/_index.md) references are for looking things up.

Once you have a package, build [your first application](@/docs/getting-started/first-application.md) and take it [from workspace to distribution](@/docs/getting-started/first-workspace.md). Learn how to [read another package](@/docs/getting-started/reading-a-package.md), [check and test your work](@/docs/getting-started/testing.md), and [decide what to grant](@/docs/getting-started/authority.md). [Choose Your Next Project](@/docs/getting-started/next-steps.md) points from there to commands, libraries, applications, workspaces, and distribution.

For the concepts that tend to get tangled, read [Modules, Layers, and Dependencies](@/docs/getting-started/modules-and-layers.md) and [Application Lifecycle and State](@/docs/getting-started/application-lifecycle.md). When something goes wrong, start with [Debugging Jess Code](@/docs/getting-started/debugging.md).
