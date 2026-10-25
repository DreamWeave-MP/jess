---
title: 'Capabilities'
description: 'Every capability a jess runtime can be granted, what each one gates, who holds what by default, how a grant is made, and what a refusal looks like.'
weight: 20
extra:
  kind: reference
---

Nothing optional is granted unless the operator asks. Files, the environment, child processes, the network, input devices, the GPU, and workers are named capabilities, denied by default and checked at the native seam, by the function that would do the thing. The check is not advisory: a function that needs a capability its caller doesn't hold raises, names itself, and names the capability.

There are fifteen names. Jess knows all of them up front, so a misspelled grant is an error rather than a grant of nothing.

## The names

| Capability | Gates | Asked for by |
| --- | --- | --- |
| `filesystem.read` | Every read of the disk: file contents, sizes, listings, canonical paths. | [`@dream/fs`](@/docs/api/l3i.md) |
| `filesystem.write` | Every change to the disk: writing, creating, removing, renaming. | `@dream/fs`, and [`@jess/backup`](@/docs/api/backup.md) through it |
| `process.environment` | Reading environment variables. | `@dream/process`'s `env` |
| `process.spawn` | Running a child process. | `@dream/process`'s `run` |
| `process.stdio` | Writing to the host's standard streams. `print` doesn't need it; it is jess's own. | `@dream/process`'s `write` |
| `network.dns.resolve` | Resolving names to addresses. | `@dream/dns` |
| `network.tcp.connect` | Outbound TCP connections to any address. A TLS session rides on a connected stream, so `@dream/tls` needs nothing of its own, and neither does [`@jess/http`](@/docs/api/http.md) beyond the transport it is given. | `@dream/tcp`'s `connect` |
| `network.tcp.listen` | Listening on a loopback address. | `@dream/tcp`'s `listen` |
| `network.tcp.public` | With `network.tcp.listen`, binding a wildcard or non-loopback address. | `@dream/tcp`'s `listen` |
| `network.transport` | Creating UDP transports, client or server. | `@dream/udp` |
| `input.configure` | Configuring the input runtime: compiling and installing action graphs, bindings, sets, and layers. | [`jess.input`](@/docs/api/input.md) |
| `input.actions` | Reading action state and action events from the runtime. | `jess.input` |
| `input.observe` | Observing the devices themselves. | `jess.input` |
| `gpu.device` | Opening the GPU: drawing and computing with it, and reading back what it made. | [`jess.gpu`](@/docs/api/gpu.md) |
| `worker.spawn` | Spawning an isolated Luau worker beside a script domain. | [`jess.worker.spawn`](@/docs/api/worker.md) |

The native modules are L3i's, and L3i owns the check for each; jess decides what is granted. [The @dream modules](@/docs/api/l3i.md) says which module is which.

## Who holds what

**A script you run as a program holds everything.** `jess script.jsl`, which is what a shebang line (`#!/usr/bin/env jess`) hands jess, and `jess run --trusted`, grant every capability in the table: you made the file executable and ran it, the same decision you make for any program, and a loose script has no manifest to declare what it needs in. The two forms below are for code you are inspecting rather than trusting.

**A script you run holds what you already have.** Under `jess run` and `jess test`, the runtime starts with `process.stdio`, `input.actions`, `input.observe`, `input.configure`, and `gpu.device`: your terminal, and the input devices and the GPU in front of you. Everything else is `--capability <name>`, repeatable, and denied without it.

```text
$ jess run read.jsl
error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant
$ jess run --capability filesystem.read read.jsl
```

**A command target holds exactly what it declares, and nothing by default.** Not even the terminal: a target with `capabilities = []` can `print`, because `print` is jess's, but `jess.granted('process.stdio')` is `false` inside it. The manifest's `capabilities` list is a request, not a grant. `jess package run` admits each name with `--capability`, or all of them with `--trusted`, and a declared name that isn't granted stops the run before anything is activated:

```text
$ jess package run greeter
error: cannot run target greet

required capability denied:
    process.environment

requested by:
    com.example.greeter 1.0.0
    target greet

$ jess package run --capability process.environment greeter
Hello, dave.
```

`jess package inspect` shows what `--trusted` would hand over, so look before you type it.

**A layer's entrypoint declares what it needs** in `[[layer.entrypoint]] capabilities`, and it is admitted against what the script or target activating it holds. One that needs more stops the run before anything is activated. `jess package graph` lists every capability a plan's entrypoints require, in one place.

**Under a script domain, the host decides.** The domain is granted a set when the host creates it, and the broker answers for every context and worker inside. A worker's `capabilities` option is a subset of the spawning context's; by default it inherits them all. There is no path by which code inside the domain widens the set.

**A package never grants itself anything.** Declaring a capability is the only thing a manifest can do, and running from a source folder gives code no power the packed JPK would lack: the two are the same package, down to the digest, and the operator's grant is the same decision either way.

## Asking first

[`jess.granted`](@/docs/api/jess.md#jess-granted) says whether the running code holds a capability, so a script can offer a degraded path instead of calling and catching the refusal:

```luau
--!strict
local process = require '@dream/process'

if not jess.granted 'process.environment' then
  print 'run with --capability process.environment to greet you by name'

  return
end

print(`Hello, {process.env 'USER' or 'stranger'}.`)
```

Asking is not a denial, so nothing is reported for a `false`. A name no jess runtime can hold is an error here too.

## What a refusal looks like

Three different texts, from three different places, and each tells you which:

| Where | Text |
| --- | --- |
| The function, at the seam | `dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant` |
| `jess package run`, before activation | `cannot run target greet` followed by `required capability denied:` and the names, then `requested by:` the package, version, and target |
| The grant itself | `unknown capability 'filesystem.raed'; a jess runtime can be granted filesystem.read, filesystem.write, gpu.device, input.actions, input.configure, input.observe, network.dns.resolve, network.tcp.connect, network.tcp.listen, network.tcp.public, network.transport, process.environment, process.spawn, process.stdio, worker.spawn` |

The last one also fires for a target that declares a name jess doesn't know. `jess package inspect` accepts any dotted name, since the manifest format is older than any particular list; `jess package run` refuses it. Inside a script domain, the runtime's own broker reports a denial as `capability '<name>' denied to <context or worker>`, attributed to whoever asked.

The one capability with its own message is `worker.spawn` outside a domain. There is nothing to grant it against: `worker.spawn: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one`.
