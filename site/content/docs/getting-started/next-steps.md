---
title: 'Choose Your Next Project'
description: 'Routes from the first package to a command, reusable library, application, or distributable workspace.'
weight: 60
extra:
  kind: guide
---

The first package proves the basic path: describe code, test it, run a target, and pack it. What comes next depends on what you are making.

## A command-line tool

Keep the useful work in a library and let the command entry handle arguments and output. This gives tests a way to exercise behavior without pretending to be a terminal. Add a target for the command, declare the capabilities it actually needs, and run it with an explicit grant. [Package targets](@/docs/cli/package.md) documents the invocation; [built-in modules](@/docs/scripting/built-in-modules.md) is a tour of the services available to the code.

## A reusable library

Export a module through a layer and keep the implementation files private. Make requiring the export inert; put activation behavior in an entrypoint when it is needed. Give consumers a stable contract and tests for it. [Packages](@/docs/scripting/packages.md) explains exports, dependencies, and why those are distinct from layer requirements.

## A long-running application

An application target creates a host and a ScriptDomain. Managed modules contribute lifecycle handlers, named-event handlers, and optional interfaces. Start with [Applications](@/docs/scripting/applications.md), then use the [application](@/docs/api/application.md), [events](@/docs/api/events.md), and [interfaces](@/docs/api/interfaces.md) references for the calls. Decide who owns state and when work should happen before adding a handler that runs every frame.

## Several packages developed together

Use a [workspace](@/docs/cli/workspace.md) to name members and pin external dependencies. Each member still declares the dependencies its own modules require. Inspect targets and the resolved graph before running them; use workspace tests as well as each package's own checks.

## Something to hand to another person

A JPK distributes one package. A packed workspace freezes exported targets and their package graph into a `.jsw`; it runs offline with Jess, and can also be combined with a runtime into a standalone executable. [Workspace packing](@/docs/cli/workspace.md#jess-workspace-pack) covers the options and platform details. Test the artifact you intend to distribute, not only the directory it came from.
