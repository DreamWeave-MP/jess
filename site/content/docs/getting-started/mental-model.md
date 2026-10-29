---
title: 'The Jess Mental Model'
description: 'Scripts, packages, authority, domains, and hosts: the boundaries to understand before building with Jess.'
weight: 15
extra:
  kind: guide
---

Jess runs Luau, but it is more than a Luau interpreter. It supplies a way to describe software, choose what runs, and decide what that software may touch. Start with the boundaries; the names make much more sense once you know who owns what.

## A script is the smallest starting point

A `.jsl` file is Luau source. `jess run` executes one directly. This is useful for experiments and small tasks; [your first script](@/docs/scripting/intro.md) walks through it. Once code needs a reusable public API, tests, or a command other people can run, give it a package.

## A package describes software

A Package Source is a directory with a `jess.toml`. The manifest identifies the package, selects its payload, declares dependencies, and names modules, layers, and targets. A JPK is the immutable distribution form of that same package. [Your first package](@/docs/getting-started/first-package.md) builds and tests both forms.

These names answer different questions:

| Name | Question it answers |
| --- | --- |
| Package | What software and payload is this? |
| Module | What code can I require? |
| Layer | What system does a host ask to activate? |
| Target | What command or application can I run? |
| Capability | What authority does that entrypoint or target need? |

An exported module is inert: requiring it does not activate its layer. Layer entrypoints run when the host activates a plan. A dependency makes another package available to the package that declares it; it does not by itself activate that package's layers. See [Packages](@/docs/scripting/packages.md) for the full model.

## Declaring authority does not grant it

A target declares its capabilities in the manifest. The caller controls the grant; a package cannot give itself permission. The CLI admits some device and terminal capabilities automatically when declared, while others need an explicit grant. For example, the first package's command declares `process.environment`, and the tutorial passes `--capability process.environment` to run it. [Capabilities](@/docs/reference/capabilities.md) lists the names and grant rules.

## A domain owns running state

A ScriptDomain owns the running contexts, resources, and lifecycle of its scripts. An application target gets a host and a domain; managed modules contribute handlers and, optionally, interfaces. The host drives the application, while the package decides what its callbacks mean. An interface is local to its domain; workers and other domains do not share that table. [Applications](@/docs/scripting/applications.md) explains startup, handlers, events, and interfaces.

## A workspace is a development arrangement

A workspace collects Package Sources developed together and pins external dependencies. It does not change a member's manifest or give it extra authority. A packed workspace is a separate distribution artifact that freezes runnable targets and their packages. See [jess workspace](@/docs/cli/workspace.md) for the commands and the distinction.

## Where to go next

Follow the [first package](@/docs/getting-started/first-package.md) from source through testing and packing. When you need a specific operation, use the [API reference](@/docs/api/_index.md); for a command or flag, use the [CLI reference](@/docs/cli/_index.md). If a term is unfamiliar, the [glossary](@/docs/reference/glossary.md) is there to get you unstuck.
