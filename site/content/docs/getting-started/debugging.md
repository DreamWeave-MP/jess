---
title: 'Debugging Jess Code'
description: 'Work from a symptom to the boundary that produced it, with checks that do not run the package.'
weight: 45
extra:
  kind: guide
---

The useful first question is not “which subsystem is broken?” It is “what was Jess asked to do, and how far did it get?” An error before activation, a type error, and a failing handler need different evidence.

## Establish the invocation

Write down the exact command, working directory, target, and arguments. Arguments after a script path belong to that script; a package target is run with `jess package run`, not by handing its name to `jess run`. Check the installed version and regenerate editor types after an upgrade if editor and runtime disagree.

## Separate reading from running

`jess check` and `jess package check` type-check without executing the checked code. `jess package inspect` reads a package's description and payload. `jess package graph` resolves its plan. Use those to establish what Jess sees before trying to explain a runtime failure. A successful check is useful evidence, not proof that files, permissions, or external services will be available later.

## Read the error at its boundary

For a missing module, check the name, the caller's package dependency, and whether the export exists. For a denied capability, read the requested name and the target's declaration before deciding on a grant. For a package failure, inspect the payload and graph. For an application handler failure, use the named context and source location to find the handler; the [application guide](@/docs/scripting/applications.md#how-an-application-starts) describes which failures stop launch and which are reported while the application continues.

The [troubleshooting reference](@/docs/reference/troubleshooting.md) is indexed by error text. Search it for the exact message before guessing.

## Make the failure smaller

Reduce the input, package graph, or number of frames while preserving the failure. Record the result you expected and what actually happened. For a package, compare source and packed tests: a difference points toward the payload or provider boundary. For an application, `--frames` can bound a run while you inspect startup and early updates.

Once the cause is understood, keep a focused test that fails for the old behavior and passes for the intended behavior. [Checking and Testing Your Work](@/docs/getting-started/testing.md) explains the tools and their limits.
