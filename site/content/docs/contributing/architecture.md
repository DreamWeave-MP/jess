---
title: 'Architecture'
description: 'The module map: what lives where, what moves between the layers, and the rules the layout enforces.'
weight: 20
extra:
  kind: guide
---

jess is one crate, `jess`, with a library and the `jess` binary. Rust owns operating-system and hardware contact, native resource lifetimes, trust boundaries, generic hot paths, and a few small retained machines. Luau owns everything that means something: application semantics, composition, workflows, tools, package behavior, and policy. Nothing under `src/` knows what any application is.

| Module | Job |
| --- | --- |
| `src/runtime` | `ScriptDomain`: lifecycle and generation, contexts and phases, capabilities, workers, ownership teardown, diagnostics, profiling. Ignorant of GUI and application identity. |
| `src/host` | The headless application kernel: a `ScriptDomain` joined to live input, advancing frames. No window, renderer, or GUI. |
| `src/input` | Device and control vocabulary, action graph compilation, the evaluator, synthetic input, and the optional platform backends (features `linux-evdev`, `gilrs`). Input policy is Luau, `@jess/input`. |
| `src/audio` | The audio engine: voices, buses, and processors mixed on a sample clock by frame-stamped commands, offline or on an output device (feature `audio-device`, on by default). Audio policy is Luau, `@jess/audio`. See [Audio Engine](@/docs/contributing/audio-engine.md). |
| `src/render` | The backend-independent render graph IR. Not a renderer. |
| `src/gpu` | The GPU through wgpu, behind the `gpu` feature: a device, buffers, textures, samplers, pipelines from shader programs, bind groups, offscreen passes, and readback. Generic: it knows bytes and formats, never what they mean. |
| `src/shader` | The typed shader IR, validation, deterministic WGSL lowering. |
| `src/lua` | The l3i extension plan: everything Luau sees. `stdlib/` holds the built-in Luau (`@jess/log`, `@jess/test`, `@jess/jpk`, `@jess/package`, and the private package bootstrap); `templates/` is `jess init`'s; `definitions.jsl` feeds `jess luau-types`. |
| `src/cli` | Thin argv and printing layers for the binary; `src/main.rs` pulls them in. |
| `jpk/builtin` | `org.dreamweave.jess`, the built-in package jess embeds and the one package allowed to export under `@jess/`. |
| `tools/bench`, `tools/site`, `tools/dev` | The benchmark harness, the two command targets that build this site, and the no-LTO development script. |
| `tests/` | The integration test binaries; see [Building and Testing](@/docs/contributing/building-and-testing.md). |

The crate depends on [l3i](https://DreamWeave-MP.github.io/l3i/) for the Luau VM, the binder, the native code generator, and the `@dream/*` primitives (bytes, files, processes, the network, text), on clap for the command line, on serde for the documents the binary prints, and on miniaudio's device layer, vendored, for sound out. That is the whole native stack.

## What moves between the layers

```text
bytes, files, sockets, devices           L3i's @dream/* and src/input: reality, behind capabilities
    v
modules and payloads                     @jess/jpk and @jess/package: a Package Source or a JPK, opened
    v
an activation plan                       @jess/package: layers resolved over packages; no code has run
    v
a runtime with the plan mounted          src/lua: require resolves exports; entrypoints run once, in order
    v
contexts, workers, events, interfaces    src/runtime: one isolated authority per domain
    v
frames                                   src/host: input phase, fixed steps, update, event drain
```

Each boundary is explicit about ownership and cost. Opening a package reads its manifest and payload and runs nothing. Resolving layers into a plan runs nothing. Activation, mounting the plan and running the entrypoints, is the host's decision and happens once per generation. A capability is checked where the native call is made, under the authority of the context or worker asking. A frame with nothing to do enters no Luau. Nothing is retained between frames on the Luau side just because native presentation continues.

## The package seam

The package model is Luau. `@jess/jpk` is the container, a narrow ZIP subset read lazily under budgets and written canonically. `@jess/package` is manifests, providers, content digests, resolution, and plans. `src/lua/stdlib/bootstrap.jsl` is the host's private entry into that Luau. `src/lua/package.rs` is the narrow native seam: the file reads handed to that Luau, mounting an accepted plan so `require` resolves package modules (under chunk names `@<id>@<version>/<path>`, in a cache keyed by id, digest, and path), package-aware `require`, and running entrypoints and targets. Package policy never moves into Rust.

A Package Source and the JPK packed from it are one package. Above the provider boundary they behave the same, down to the digest, and nothing a package's code does may tell them apart. `jess package test` runs a package's tests from its source and then from a temporary JPK and compares the two digests, which is how that promise is kept honest.

## The Luau seam

`src/lua/plan.rs` composes jess's extensions with l3i's into one runtime plan, which assigns every tag, atom, and direct-access slot; `runtime` makes a VM from it with `jess`, `require`, and `print` as globals, then sandboxes the globals so builtins run on Luau's fast path. `convert.rs` moves values between Rust and Luau through a `serde_value` tree, renaming snake_case to camelCase, keeping NaN and `-0`, and using `jess.null` to hold array slots. `definitions()` builds `jess.d.jsl`, every alias an `export type`; `types.rs` is the folder around it that `jess luau-types` and `jess init` write: a stub per name `require` resolves natively, `.jessrc` aliases per scope, and stale stubs removed.

The built-in modules are Luau embedded with `include_str!`, compiled once per thread and cached as bytecode, run as factories that receive the few native values they are built on, and frozen. The built-in package is embedded byte for byte (`src/lua/builtin.rs`); a package claiming its id is accepted only if it is that copy.

## Rules the layout enforces

**The kernel is generic.** Nothing in `src/` may know what a game, an editor, a launcher, or a file format is. If a name from an application shows up in Rust, it is in the wrong place.

**Application semantics are Luau.** If it can be built from bytes, buffers, handles, generic services, and l3i primitives, it is a package module, not new Rust. The first-party packages use the same public architecture any trusted package can; there is no private framework for them.

**A missing primitive belongs in l3i.** A codec, a buffer operation, a filesystem call, a socket option: those go into l3i, the one Luau binder, not into a new jess dependency or a native jess API. jess has no second binder.

**The CLI is a thin frontend.** `src/cli` parses flags, calls a library function, and prints one YAML, JSON, or TOML document. If a command needs logic, the logic goes below it. Tool packages follow the same shape: a library, a `commands` module taking typed requests and returning typed results, and a thin `cli` over it.

**Native acceleration follows profiling.** A module compiles natively only when a measurement on a verified build says so; see [Luau Style](@/docs/contributing/luau-style.md#types) and [Benchmarks](@/docs/contributing/benchmarks.md).

**Licenses follow the links.** The crate is MIT OR Apache-2.0, and so is everything it links, transitive and native components included. A dependency is checked before it goes in; one that isn't compatibly licensed doesn't go in, and code moved to Luau keeps its provenance.
