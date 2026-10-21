## jess

A native application kernel for [Luau](https://luau.org) applications.

**Documentation: <https://DreamWeave-MP.github.io/jess>**

Rust owns operating-system and hardware contact, resource lifetimes, and the trust boundary. Luau owns everything that means something: application behaviour, tools, workflows, formats, policy. The kernel knows nothing about what runs on it, and that is the whole design.

> Rust exposes reality. Luau decides what it means.

One binary, `jess`, is the runtime, the test runner, the package tool, the workspace tool, the type generator, and the style checker.

### Install

[Release archives](https://github.com/DreamWeave-MP/jess/releases) carry the `jess` binary for Linux, macOS, and Windows, signed with Sigstore. `cargo install jess` builds it from crates.io with Rust 1.92 or newer. [Install](https://DreamWeave-MP.github.io/jess/docs/getting-started/install/) covers both, plus shell completions and man pages.

### A script

```sh
jess init my-scripts      # types for your editor, a .jessrc, and main.jsl
jess run my-scripts/main.jsl
```

```luau
--!strict

local log = require('@jess/log').new 'main'

log.info('ready', { version = jess.version })
```

Sources are `.jsl` files: Luau, type-checked under `--!strict` against the types `jess luau-types` writes for every module. `jess run --profile` prints call, sampling, and native-block statistics; `--emit-asm` writes the generated assembly; `--disassemble` the bytecode.

A script holds your terminal and the input devices in front of you, and nothing else. Files, the environment, processes, and the network are capabilities, denied until you grant them:

```text
$ jess run read.jsl
error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant
$ jess run --capability filesystem.read read.jsl
```

### A package

A **Package Source** is a folder with a `jess.toml`: modules, the layers that export them, dependencies, tests, and command targets with the capabilities each one needs. `jess pack` turns it into a **JPK**, an immutable archive. Above the provider boundary the two are the same package, down to the digest.

```toml
manifest_version = 1

[package]
id = "com.example.greeter"
version = "1.0.0"
jess_api = "1"
include = ["src", "tests", "tools"]
tests = ["tests/run.jsl"]

[modules]
"@greeter" = "src/init.jsl"

[[layer]]
id = "example.greetings"
exports = ["@greeter"]

[targets.greet]
kind = "command"
command = "greet"
entry = "tools/greet.jsl"
capabilities = ["process.environment"]
```

```sh
jess package test greeter        # from the source, then packed; both digests must agree
jess package check greeter       # strict types, every .jsl, each require resolved as the runtime does
jess package run greeter         # refused: the target needs process.environment
jess package run --capability process.environment greeter
jess pack greeter -o greeter.jpk
```

Exports are inert: requiring one starts nothing. A package never grants itself a capability, and running from a source folder gives code no power the packed archive would lack. Several packages developed together are a workspace, with a lock file and `jess workspace sync`, `run`, and `test`.

Applications are packages too: managed modules loaded into one script domain, host callbacks per frame, named events, and interfaces that later packages override on purpose. [Packages](https://DreamWeave-MP.github.io/jess/docs/scripting/packages/) and [Applications](https://DreamWeave-MP.github.io/jess/docs/scripting/applications/) cover both; the [manifest reference](https://DreamWeave-MP.github.io/jess/docs/reference/manifest/) has every key.

### What's in the kernel

- **Script domains**: isolated Luau authorities in one process, each with its contexts, its capability grant, its workers, a generation that moves on every restart, and an ownership registry that teardown releases.
- **Packages**: the container (`@jess/jpk`), the model (`@jess/package`), resolution into an activation plan that runs no code, and activation that does, in dependency order.
- **Capabilities**: fourteen names, checked at the native seam, granted only by the operator.
- **Input**: a device vocabulary, a serializable action graph, and an evaluator; the policy above it is Luau.
- **Render graph and shader IR**: backend-independent IRs, a deterministic WGSL lowering, and no renderer.
- **The built-in package**: `@jess/style`, `@jess/analysis`, `@jess/json`, `@jess/yaml`, `@jess/data/toml`, `@jess/data/value`, `@jess/l10n`, `@jess/input`, `@jess/backup`, and `@jess/http`, built on the same public architecture any package can use.

Luau runs on [L3i](https://github.com/DreamWeave-MP/l3i), which binds it and provides the native primitives (`@dream/fs`, `@dream/process`, `@dream/tcp`, and the rest). Native code is a privilege: a module compiles natively only where a measurement earned it.

### Building

```sh
git clone https://github.com/DreamWeave-MP/jess
cd jess
cargo build --release
```

The binary lands at `target/release/jess`. [Contributing](https://DreamWeave-MP.github.io/jess/docs/contributing/) covers the gates every change passes and how the runtime is put together. The documentation site is built by a package running on jess, from `site/`, through the command targets in `tools/site`.

Until 1.0, a minor version can break things: pin one.

## License

MIT OR Apache-2.0, at your option: [MIT](LICENSE-MIT), [Apache-2.0](LICENSE-APACHE).
