---
title: 'Troubleshooting'
description: 'The errors jess prints, keyed by their text, with what each one means and what to do: scripts, require, capabilities, manifests, resolution, tests, domains, GPUs, windows, and sound.'
weight: 40
extra:
  kind: reference
---

Every text below is quoted as the binary prints it, so you can search this page for the line you got. Errors go to stderr, prefixed `error:`, and exit `1`. A report that says a test failed or a target was refused is also exit `1`, but it is a document, not an error: jess did what you asked, and this is the answer.

## Running a script

**`error: 'nope.jsl' isn't a file`**

`jess run` takes a path, not a target name and not a module name. Check the path, and remember that `jess run fix.jsl --verbose` hands `--verbose` to the script, not to jess, so a flag you meant for jess goes before the script.

**`error: 'script.lua' isn't a .jsl file: jess runs .jsl sources, and nothing else`**

Sources are `.jsl` files, everywhere: `jess run`, `jess test`, `jess check`, a manifest's modules, and `require`. The language is Luau; only the extension is jess's. Rename the file. Under `jess test` the same text appears as the module's `error`, since a test module that didn't run is reported, not fatal.

**`error: 2 type errors`, after a report from `jess check`**

[`jess check`](@/docs/cli/check.md) found what strict mode would show in your editor, in the scripts given and the files their `./` requires reach. Each error names the file, line, and column. A script that passes `jess check` can still fail at run time for reasons types don't cover, a missing file or a denied capability; a script that fails it has a bug the checker can prove.

**`error: module not found: ./nothing`**

A relative `require` looked for `nothing.jsl` and `nothing/init.jsl` beside the requiring file and found neither. The extension is `.jsl`, not `.luau` or `.lua`, and there is no fallback. A bare name that isn't `@jess/`, `@dream/`, or an export of an activated package is also not found: `require` doesn't search the current folder, the data folder, or anything else.

**`error: module not found: @jess/whatever: @jess/ is jess's own namespace (its members, such as @jess/shader, @jess/log, @jess/test, @jess/jpk, @jess/package, and the built-in package's exports, such as @jess/style), never a file`**

`@jess/` names are jess's: a member of the `jess` global, a [built-in module](@/docs/scripting/built-in-modules.md), or an export of the built-in package. No file, alias, or other package can stand in for one. The message lists where the real ones come from; the [API reference](@/docs/api/_index.md) lists them all.

**`error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant`**

The function names itself and the capability. Under `jess run` and `jess test`, add `--capability filesystem.read`. In a command target, declare it in the target's `capabilities` and run with `--capability` or `--trusted`. [Capabilities](@/docs/reference/capabilities.md) has the whole table. If you'd rather degrade than fail, ask `jess.granted` first.

**`error: unknown capability 'filesystem.raed'; a jess runtime can be granted filesystem.read, filesystem.write, ...`**

A `--capability` name, or a target's declared one, that no part of jess knows. The message lists every name that exists. A misspelled grant is refused rather than granting nothing, which is the behaviour you want from a thing called a capability.

**`error: worker.spawn: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one`**

`jess.worker.spawn` and `jess.context.bind` only mean something inside a [script domain](@/docs/contributing/script-domains.md), which an application target or an embedding host provides. A plain script, a test module, and a command target run in a single VM with no domain around it. Make it an application target, or do the work in the script.

## Manifests

Every manifest error starts with `package:` and names the table and key. [The Manifest](@/docs/reference/manifest.md) lists the full set under each key; these are the ones people hit first.

**`error: package: jess.toml [package] has an unknown key 'author'`**

Jess rejects keys it doesn't know, at every level. Check the spelling against the manifest reference; there is no `author`, `description`, or `license` key, since jess doesn't read them.

**`error: package: jess.toml has no [package] table`**

The file parsed but the `[package]` table is missing, usually because it was written `[Package]` or the file is the wrong one.

**`error: toml: expected ']' at line 2, column 9`**

The file didn't parse as TOML at all. The line and column are the decoder's; the usual cause is an unclosed `[table` header or a string without its closing quote.

**`error: package: jess.toml: module '@greeter' is neither exported nor an entrypoint of any layer`**

A `[modules]` entry no `[[layer]]` exports or runs. Add it to a layer's `exports`, make it an entrypoint, or delete it. A module nobody can reach ships for nothing.

**`error: package: jess.toml: layer 'example.greetings' exports '@greeter', which [modules] doesn't name`**

The layer exports a name that `[modules]` doesn't map to a file. Add the module, or fix the name on one side.

**`error: package: jess.toml module '@jess/greeter': @jess/ is jess's own namespace, open only to its built-in package (org.dreamweave.jess)`**

Pick a scope of your own. `@jess/` is reserved, whatever the package is called.

**`error: package: bm/missing-entry: target 'greet' entry is 'tools/run.jsl', which isn't in the payload`**

The file a target, module, or managed entry names isn't under any `include` root, or isn't there. Either add its folder to `[package] include` or fix the path. The same shape of message, `module '...' is '...', which isn't in the payload` and `managed '...' is '...', which isn't in the payload`, means the same thing for those.

**`error: package: 'greeter': [package] include names 'assets', which isn't there`**

An `include` root that doesn't exist on disk. Remove it or create it; an empty folder contributes nothing, so you may as well remove it.

**`error: package: jess.toml dependency 'com.example.words' 'latest' is not a version requirement (^1.2, =1.2.3, =1.2.3-dev.4)`**

Requirements are caret ranges or exact versions. There is no `latest`, `*`, or `>=`. A pre-release can only be named exactly.

## Resolution

**`error: package: example.greetings requires example.words: no provider found for example.words`**

A layer requires another that nothing offered provides. Offer the package that has it with `--package` or put it in a `--package-path` folder; `jess package graph` shows what was found.

**`error: package: com.example.greeter 1.0.0 requires com.example.words ^1.2`** followed by **`resolution failed: com.example.words is not available`**

A `[dependencies]` entry nothing offered meets. Offer the package at a version the requirement accepts. If the only version is a pre-release, the requirement must name it exactly: no range selects a pre-release.

**`error: package: layer example.greetings has more than one candidate: com.example.greeter 1.0.0 (source pp/one), com.example.greeter 1.0.0 (source pp/two); offer one of them outright`**

Two packages in the offered set provide the same layer, typically a Package Source and a JPK of the same package in one `--package-path` folder, or two checkouts. Jess won't pick. Name the one you mean with `--package`, which is chosen over anything found in a path, or remove the other.

## Packages and targets

**`error: cannot run target greet`** followed by **`required capability denied:`** and the names

The target declares a capability you didn't grant. Grant each with `--capability <name>`, or everything the target declares with `--trusted`, after `jess package inspect` has shown you what that is. This happens before anything is activated, so nothing ran.

**`jess: greeter.jpk: tests run from a Package Source; run `jess package test` on its source`**

Not an error, and exit `0`: a JPK ships its tests but they only run from the folder they were written in, so `jess package test` on a `.jpk` reports no providers. Run it on the Package Source. That run packs the source into a temporary JPK and tests both, comparing their digests, which is the check you wanted.

**`error: 2 failed`** at the end of a `jess package test` report

Each test module ran twice, from the source and from it packed, so one failing case counts twice. Read the `error:` under the failing case: an expectation failure quotes both values and the file and line, as in `expected "Hello, world." to be "Hi, world."` and `bad/tests/run.jsl:8`.

**Two digests for what you thought was one package**

`jess package inspect` on a source and on a `.jpk` disagree on `digest` after an edit. The digest covers the whole payload, so the JPK is now a different package from the folder: re-pack it. A workspace whose `jess.lock` pins the old digest reports the kept copy as `not the one jess.lock pins` until you `jess workspace sync` again.

**`error: 1 type error`** from `jess package check`, with a list above it

The strict type checker found something, listed as `file`, `line`, `column`, and `message`, with each `require` resolved as the runtime resolves it. `Key 'gret' not found in table '{ read greet: (string) -> string }'` is a misspelled field; `unknown` where you expected a type usually means a value the checker can't see through, which you annotate. These are errors in your package, not in jess, and the check is a gate: fix them.

## GPUs, windows, and sound

jess loads the machine's GPU API, window system, and sound system when a script first asks for one, not when it starts, so every entry here is about a script that asked. A script that never touches [`jess.gpu`](@/docs/api/gpu.md) or [`jess.audio`](@/docs/api/audio.md) runs the same on a machine with none of them.

**`error: jess.gpu.device: no GPU adapter, not even a software one`**

jess looked through Vulkan, Metal, and Direct3D 12, then OpenGL, and found no adapter, hardware or software. On Linux that usually means no Vulkan driver and no working OpenGL either: install your distribution's Mesa Vulkan drivers, which include lavapipe, a software Vulkan adapter, and jess draws on the CPU where it can't draw on a GPU. A container or a CI runner needs them inside it.

**`device.kind` is `cpu`, and everything is slow**

Not an error. With no hardware adapter it can reach, jess opens a software one (lavapipe, llvmpipe, WARP), which draws everything correctly and none of it quickly. That's what an offscreen render on a server wants, and not what a desktop with a GPU should get: check that the GPU's own Vulkan driver is installed, then that `WGPU_BACKEND` (below) isn't steering jess away from it.

**`error: jess.gpu.device: the GPU adapter opened no device: …`**

An adapter answered, and its driver refused to open a device, with the driver's reason after the colon. Try another API with `WGPU_BACKEND`, and update the driver.

**Choosing the API: `WGPU_BACKEND`**

jess tries Vulkan, Metal, and Direct3D 12 first, and OpenGL only when none of those answers. `WGPU_BACKEND` replaces both lists with the APIs it names, comma-separated (`vulkan`, `metal`, `dx12`, `gl`): under `WGPU_BACKEND=gl jess run draw.jsl` a script draws through OpenGL or not at all. `device.backend` says which one a script got.

**`error: jess.gpu.window: no window system: …`**

There's no display to open a window on: a session over SSH, a container, a CI runner, a text console. On Linux the rest of the message is the window system's own, usually that none of `WAYLAND_DISPLAY`, `WAYLAND_SOCKET`, or `DISPLAY` is set. Run it from the desktop session, or forward a display. Everything that doesn't open a window still works, offscreen rendering and compute included, and the next `jess.gpu.window` tries again, in case the display came back.

**`error: jess.gpu.window: on macOS, windows open on the process's main thread`**

macOS lets one thread run a window system, the process's first, and jess opens windows there or nowhere. A script under `jess run` and an application's own code are on that thread; a worker isn't. Open the window from the domain's code and give the worker the work, not the window.

**`error: jess.gpu.window: windows belong to the thread that opened the first one`**

A process has one window system loop, on the thread that opened the first window, and every later window opens on that thread too.

**No sound, and no error**

A machine with no sound system (a server, a container, a session with no sound server running) still gets an output from [`jess.audio.output`](@/docs/api/audio.md): the null device, which plays to nowhere in real time, and `context:device().backend` is `Null`. Everything else about the context works, which is the point: a test or a headless run doesn't need speakers. If you expected sound, check that the platform's sound server is running for the user jess runs as.

## Editor types

**The editor approves a call the runtime rejects, or vice versa**

`types/` describes one jess version. After upgrading jess, run `jess init` in the folder again (it refreshes `types/` and leaves your files alone) or `jess luau-types types`. If the editor shows no jess types at all, it isn't reading `.jessrc`: the alias file `jess init` writes is `.jessrc`, and luau-lsp needs the definitions flag, which [Editor Setup](@/docs/getting-started/editor-setup.md) spells out per editor.

## None of these

If jess printed something this page doesn't have, and the [CLI](@/docs/cli/_index.md) and [API](@/docs/api/_index.md) references don't either, that is a documentation bug as well as whatever else it is. [File it](https://github.com/DreamWeave-MP/jess/issues) with the exact text.
