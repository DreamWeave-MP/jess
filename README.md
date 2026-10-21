## jess

An optimized Luau runtime for general-purpose programming and game engine development.

**Documentation: <https://DreamWeave-MP.github.io/jess>**

Jess combines a small Rust kernel with a programmable [Luau](https://luau.org) environment. Rust manages access to the operating system and hardware; Luau implements applications, tools, workflows, and policy. The kernel knows nothing about what runs on it, and that is the whole design.

> Rust exposes reality. Luau decides what it means.

It ships as one binary, `jess`: the runtime, the test runner, the type checker, the package tool, the workspace tool, the type generator, and the style checker. There is no separate build system, package manager, or linter to install.

Jess is not a game engine. It is the environment you build one in, and the same environment builds command-line tools, servers, build pipelines, and the site that documents it.

### Getting started

[Release archives](https://github.com/DreamWeave-MP/jess/releases) carry the binary for Linux, macOS, and Windows, signed with Sigstore. `cargo install jess` builds it from crates.io with Rust 1.92 or newer. Then write a file:

```luau
--!strict
local name = ... or 'world'

print(`Hello, {name}.`)
```

```text
$ jess run hello.jsl
Hello, world.
$ jess run hello.jsl Dave
Hello, Dave.
```

A shebang line makes it a program. `#!/usr/bin/env jess` at the top and `chmod +x`, and `./hello.jsl Dave` runs it with every capability, as you'd expect of a program you chose to run; `jess run` is the default-deny form for code you're inspecting.

Sources are `.jsl` files: Luau, checked under `--!strict` against types Jess generates for every module, so your editor knows the whole API before you've written a line. `jess init` sets a folder up with them, and `jess check` runs the same checker from the command line. [Getting Started](https://DreamWeave-MP.github.io/jess/docs/getting-started/) takes it from there.

### Packages and applications

Luau software ships for Jess as a **JPK**: an immutable archive with a manifest naming its modules, what they export, what they depend on, the tests that ship with them, and the commands they provide. A source folder is the same package before packing, down to the digest, so what you test is what you ship. Several packages developed together form a workspace with a lock file.

An **application** is packages loaded into one isolated runtime and driven by frames until it decides it is done. A game, an editor, and a server are all some packages, some callbacks, and a loop. [Packages](https://DreamWeave-MP.github.io/jess/docs/scripting/packages/) and [Applications](https://DreamWeave-MP.github.io/jess/docs/scripting/applications/) explain both.

### What it gives you

- **Isolated execution.** Script domains run separate programs in one process, each with its own lifetime and an explicit capability grant. Files, the environment, processes, and the network are denied until the operator grants them, and a package can never grant itself anything:

  ```text
  $ jess run read.jsl
  error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant
  $ jess run --capability filesystem.read read.jsl
  ```

- **Extensible packages.** Compose applications and tools from reusable Luau modules. A package is inert until a host activates it, and its tests, types, and style checks ship inside it.
- **Game-engine foundations.** An input action graph, a typed shader IR that lowers to WGSL, a render graph, and localization bundles, none of them tied to a renderer or a game.
- **Native speed where it's earned.** Luau runs on [L3i](https://github.com/DreamWeave-MP/l3i), with native code generation for the modules that ask for it and profiling built into the binary. Nothing goes native without a measurement.

[Architecture](https://DreamWeave-MP.github.io/jess/docs/contributing/architecture/) covers how the pieces fit.

### Building from source

```sh
git clone https://github.com/DreamWeave-MP/jess
cd jess
cargo build --release
```

The binary lands at `target/release/jess`. [Contributing](https://DreamWeave-MP.github.io/jess/docs/contributing/) covers the gates every change passes. Until 1.0, a minor version can break things: pin one.

## License

MIT OR Apache-2.0, at your option: [MIT](LICENSE-MIT), [Apache-2.0](LICENSE-APACHE).
