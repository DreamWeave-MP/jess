---
title: 'Reading a Package'
description: 'Find a package’s public API, entrypoints, dependencies, tests, and requested authority before running it.'
weight: 30
extra:
  kind: guide
---

Someone hands you a package. Before running it, you can find out what it contains and what it asks to do. Opening and inspecting a package reads its description; it does not activate its layers or run its targets.

## Start with the manifest

In a Package Source, open `jess.toml`. For a JPK, use `jess package inspect` on the archive. The same command works on a source folder, so you can compare the two. The [first package](@/docs/getting-started/first-package.md) shows the output in full.

Read these parts in order:

1. **Identity and version**: `[package]` tells you which package this is and which Jess package API it expects. `include` selects the source files that ship.
2. **Dependencies**: `[dependencies]` names the other packages its code may use. A package appearing in a workspace is not, by itself, permission for every other member to require it.
3. **Public modules**: `[modules]` maps names to files; a layer's `exports` identifies the public names. Other payload files can remain private.
4. **Activation**: layers name dependencies on other layers and may have entrypoints. An entrypoint runs on activation; an export is an API, not an instruction to start a system.
5. **Targets**: each target names an entry and its capabilities. A command runs to completion; an application has a host and lifecycle. Inspect the declaration before granting authority.
6. **Tests**: `package.tests` names the suite modules. Run `jess package test` on the source to exercise both source and packed providers.

[The manifest reference](@/docs/reference/manifest.md) defines every field. [Packages](@/docs/scripting/packages.md) explains resolution and activation in detail.

## Follow a public call

Find the export name in a layer, then its file in `[modules]`. Read what that file returns: callers see the returned value, not every file in the package. Follow its relative requires within the package and its named requires into declared dependencies. A package module cannot walk into a dependency's private files; the dependency's exported names are its boundary.

The returned package module table is shallowly frozen. That protects the exported table, not every value reachable through it. Do not assume it makes all internal state immutable.

## Check what will run

`jess package graph` shows the resolved layer plan and entrypoints. It is useful when a package is composed from several layers: a manifest can describe a system without that system running merely because you inspected it. A target's capability declaration tells you what it requests, while the caller decides what to grant. See [Capabilities](@/docs/reference/capabilities.md) for the names and [Troubleshooting](@/docs/reference/troubleshooting.md) for common resolution failures.

## If source and JPK disagree

Compare their digests with `jess package inspect`. A digest covers the payload, so editing an included file after packing makes the old archive a different package. Repack it, then run the source package's tests again. A JPK is not a live view of its source directory.
