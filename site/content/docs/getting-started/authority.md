---
title: 'Capabilities and Trust'
description: 'Understand declarations, grants, and the difference between running a script and running a target.'
weight: 50
extra:
  kind: guide
---

Some operations reach outside the Luau runtime: reading files, changing them, inspecting the environment, and connecting to a network. Jess names those powers as capabilities. A refusal tells you which operation asked for which capability; [the capability reference](@/docs/reference/capabilities.md) lists the names.

## A declaration is a request

A package target declares the capabilities it needs in `jess.toml`. The person or host running it decides the grant. The CLI automatically admits the terminal, input, GPU, window, and audio capabilities listed under [Who holds what](@/docs/reference/capabilities.md#who-holds-what) when the target declares them; other required capabilities need an explicit grant. If one is missing, the target is refused before its code runs. `--capability` grants a named capability; `--trusted` grants what the target declares. Inspect the package before deciding what to grant.

Layer entrypoints also declare what they need. Exports do not grant themselves authority: a library function runs with the authority of the code calling it. A package source is not entitled to more than its packed form.

## The invocation matters

`jess run` starts a script with capabilities denied unless granted. The program form, such as `jess script.jsl`, runs a script as a trusted program with every capability. That distinction matters when copying an example: check which command you are running, not just which file it names. [Your First Script](@/docs/scripting/intro.md#capabilities) demonstrates a refusal and a named grant.

Tests and command targets are not application domains. A function that needs a ScriptDomain, such as spawning a worker, needs an application target or an embedding host even if the capability you wanted has been granted. Authority and lifecycle are separate questions.

## Optional behavior

Code can ask `jess.granted` whether it has a capability before attempting an optional operation. That is useful when an application can offer a meaningful alternative. It is not a reason to silently skip a required operation: explain what is unavailable and what the caller needs to grant.

Capabilities describe access, not intent. Granting filesystem write lets code write with the process's operating-system permissions; it does not confine writes to a convenient folder. Read the code and manifest of software you choose to trust.
