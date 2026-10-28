---
title: 'Glossary'
description: 'The words these docs use without stopping to explain them: packages and their parts, plans and activation, domains and their parts, pictures and sound, workspaces, and the files on disk.'
weight: 30
extra:
  kind: reference
---

Grouped by what they belong to, each with the page that explains it properly.

## Packages

**Package Source.** A folder with a `jess.toml` at its top: what you edit, test, and keep in git. One of the two forms a package takes. [Packages](@/docs/scripting/packages.md).

**JPK.** The same package packed into one file by `jess pack`: a narrow subset of ZIP, immutable, canonical, with the same bytes from every machine. The other form. A changed package is a new JPK. [`@jess/jpk`](@/docs/api/jpk.md).

**Package.** What jess sees after opening either form: a manifest, modules, layers, targets, and a payload. The two forms are the same package, down to the digest, and no package code may tell them apart.

**Provider.** The thing that hands jess a package's bytes: the folder or the archive. The boundary above which Package Source and JPK are indistinguishable.

**Payload.** Every file the package ships: `jess.toml` and everything under `[package] include`, in canonical order. The digest is over exactly these. [The Manifest](@/docs/reference/manifest.md#package).

**Digest.** The content hash of a package's payload, identical for a source and the JPK packed from it. What `jess.lock` pins and what `jess package test` compares across providers. [`@jess/package`](@/docs/api/package.md#the-content-digest).

**Manifest.** `jess.toml`: identity, payload, modules, layers, managed modules, targets, dependencies. Unknown keys are errors. [The Manifest](@/docs/reference/manifest.md).

**Module.** A `.jsl` file a manifest names under `[modules]`, by a name like `@greeter/words`. Not requirable until a layer exports it and a host activates that layer.

**Export.** A module a layer offers as public API. Inert: requiring one starts nothing. What comes back from `require` is frozen.

**Layer.** A named semantic system a host asks for by id, such as `example.greetings`: the layers it requires, the modules it exports, and the entrypoints that run when it is activated. [Packages](@/docs/scripting/packages.md#layers).

**Entrypoint.** A module a layer runs once, in dependency order, because the layer was activated. The only way package code runs without being required.

**Dependency.** A package another package's modules may require exports from, declared with a version requirement in `[dependencies]`. Also a resolution constraint. `[dev-dependencies]` are selected only for the package's own tests and type check.

**Resolution.** Asking for layers over a set of packages and getting back an activation plan. Runs no code. `jess package graph` shows it.

**Activation plan.** What resolution produces: the layer order, the packages selected, the module names, and the capabilities the entrypoints require.

**Activation.** Mounting a plan so its exports resolve through `require`, then running its entrypoints. The host's decision, never a side effect of opening something.

**Target.** Something `jess package run` runs, declared under `[targets]`: a **command** target, a script with a package around it that runs and returns, or an **application** target, which keeps running. Each declares the capabilities it needs. [The Manifest](@/docs/reference/manifest.md#targets).

**Managed module.** A module an application target loads into its script domain, declared under `[managed]`, which binds host callbacks and event handlers and may provide an interface. [Applications](@/docs/scripting/applications.md).

**Interface.** A table a managed module provides under a dotted id, published to `@jess/interfaces` for the whole domain, which a package later in the load order may override. [`jess.interfaces`](@/docs/api/interfaces.md).

**Built-in module.** Luau that ships inside the binary and resolves in every runtime: `@jess/log`, `@jess/test`, `@jess/jpk`, `@jess/package`. [Built-in Modules](@/docs/scripting/built-in-modules.md).

**Built-in package.** `org.dreamweave.jess`, the one package jess embeds and the only one allowed to export `@jess/` names: `@jess/style`, `@jess/json`, `@jess/l10n`, and the rest. Accepted only when it is byte-for-byte the copy jess carries.

## Authority

**Capability.** A named permission, such as `filesystem.read`, denied by default and checked by the function that would use it. Seventeen exist. [Capabilities](@/docs/reference/capabilities.md).

**Grant.** The set of capabilities a runtime, domain, context, or worker holds. Made by the operator (`--capability`, `--trusted`) or the host, never by a package.

**Trusted.** `--trusted`: grant a target everything it declares. `jess package inspect` shows what that is.

## Domains

**Script domain.** One isolated Luau authority in a process: a runtime, the contexts that say when its code runs, a capability grant, workers, owned resources, and a queue of events. A host may run several. [Script Domains](@/docs/contributing/script-domains.md).

**Context.** A named slot in a domain that a host declares with the phases it may run in, and that a script binds handlers to with `jess.context.bind`. [`jess.context`](@/docs/api/context.md).

**Phase.** When a context's handler runs: `load`, `startup`, `update`, `fixedUpdate`, `lateUpdate`, `input`, `renderPrepare`, `event`, `message`, `timer`, `onDemand`, `shutdown`.

**Worker.** A second Luau state a domain owns, with its own globals, modules, and collector, which sees nothing of the domain but the messages it is sent. [`jess.worker`](@/docs/api/worker.md).

**Generation.** A counter a domain advances on every restart, so a handle from before a reload can be recognised as stale instead of pointing at something that no longer exists.

**Teardown.** Releasing everything a domain's ownership registry recorded, in order, when the domain stops or restarts.

**Event.** A named message with a payload, emitted with `jess.events.emit`, queued, and delivered to handlers on the next drain. Routed between domains when the host says so. [`jess.events`](@/docs/api/events.md).

**Host.** The thing that owns a domain and advances it: for an application target, a `Host` that joins the domain to live input and runs frames. The host itself owns no window and no renderer; the windows a script opens are stepped beside it, each frame. [Application Kernel](@/docs/contributing/application-kernel.md).

**Frame.** One advance of a host: the input tick, the contexts' phases in order, the event drain. An application runs until it calls `jess.application.exit`, `--frames` runs out, or Ctrl+C. Not an audio frame, which is one sample per channel; [`jess.audio`](@/docs/api/audio.md) counts time in those.

**L3i.** The Luau binder jess runs on, and the owner of the native `@dream/*` modules: bytes, files, processes, the network, text. The only place a missing native primitive belongs. [The @dream modules](@/docs/api/l3i.md).

## Pictures and sound

**Device.** A GPU as a script holds it, from `jess.gpu.device()`: what makes buffers, textures, pipelines, plans, and windows. A machine with no GPU gets a software one or an error, never a crash. [`jess.gpu`](@/docs/api/gpu.md).

**GPU plan.** A render graph compiled for a device, no relation to an activation plan: the targets, buffers, and passes a frame uses, ordered, culled, and allocated up front so running it only records. [GPU plans](@/docs/api/gpu.md#plans), [Render Graph](@/docs/contributing/render-graph.md).

**Draw set.** One draw kept by the device, its pipeline, bind groups, and buffers, which a plan's pass draws every time it runs without the script naming it again.

**Commit.** The moment the GPU takes what a script changed about what it draws, all at once: at the end of every frame, at `jess.gpu.pump()`, and before anything that waits on the GPU. A window never shows half of one. [Commits](@/docs/api/gpu.md#commits).

**Window.** A native window from `jess.gpu.window`, showing a plan every frame without entering Luau, its keyboard and pointer reaching the application's input. Needs `gpu.window`. [Windows](@/docs/api/gpu.md#windows).

**Shader program.** A shader written in Luau with `jess.shader`, typed and validated in jess and lowered to WGSL, that a pipeline runs. [`jess.shader`](@/docs/api/shader.md).

**Audio context.** A mixer and its clock: an output that the device renders, or an offline one the script renders itself. [Contexts](@/docs/api/audio.md#contexts).

**Voice.** One playing of a sound in a context, which can be started, stopped, paused, moved, and routed while the mixer runs. [Voices](@/docs/api/audio.md#voices).

**Bus.** A node of a context's graph that sums what reaches it, runs its processors over the sum, and passes it on toward the master bus. [Nodes](@/docs/api/audio.md#nodes).

**Stream.** Audio written while it plays: a bounded ring that one writer, often a worker, fills while one voice drains it. [Streams](@/docs/api/audio.md#streams).

## Development

**Workspace.** A set of Package Sources developed together, named by `jess-workspace.toml`, found from any folder inside it. A development arrangement, not part of any package's identity. [`jess workspace`](@/docs/cli/workspace.md).

**Member.** A Package Source a workspace file lists.

**Lock.** `jess.lock`: what the members depend on beyond each other, pinned by id, exact version, and digest, with the JPKs kept under `.jess/packages`. Written by `jess workspace sync`, never by hand.

**Publisher.** A site that publishes packages through a project manifest, which `sync` can pin a package from, recording the manifest URL and the archive's SHA-256 in the lock.

**Type stub.** The `.jsl` files `jess luau-types` writes: `jess.d.jsl` for the global, one stub module per native `@jess/` and `@dream/` name, and a copy of each of the built-in package's modules, so luau-lsp can type-check a script under `--!strict`. [Editor Setup](@/docs/getting-started/editor-setup.md).

**`.jsl`.** The extension of every Luau source jess reads. The language is Luau; the extension says whose it is. `require` tries `name.jsl` and `name/init.jsl`, nothing else.

**`.jessrc`.** The editor alias file `jess init` writes, mapping `@jess/` and `@dream/` to the types folder, with strict mode on. What luau-lsp reads; jess itself never does.
