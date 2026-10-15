---
title: 'Crate Overview'
description: 'What each crate does, which to depend on, features, and the publish set.'
weight: 10
extra:
  kind: guide
---

jess is one workspace of crates on crates.io, released together. Most people want one of two things: the `jess` facade with the features they need, or one library crate directly. The rest are support crates you get for free.

## Pick one

```toml
[dependencies]
# Plugins only: the raw spine plus typed records.
jess = { version = "0.1", default-features = false, features = ["plugin", "omwaddon"] }
```

That gives you `jess::plugin` and `jess::omwaddon`. Want NIFs too? Add `"nif"`. Want the Luau API? That's `jess-lua`, a crate of its own. Or skip the facade and depend on `jess-plugin`, `jess-nif`, and friends by name. Same code either way: the facade is `pub use` and nothing else.

## The crates

In publish order, which is also dependency order:

| Crate | Library name | What it's for | Docs |
| --- | --- | --- | --- |
| `jess-macros` | `jess_macros` | The proc macros behind `jess-io`'s `Load`/`Save` derive, `jess-omwaddon`'s records (feature `tes3`), and `jess-nif`'s type hierarchy (feature `nif`). Each format crate turns on only its own. | [docs.rs](https://docs.rs/jess-macros) |
| `jess-io` | `jess_io` | Endian-aware binary `Load`/`Save` traits, byte-range helpers, and the legacy Windows code pages (`LegacyEncoding`: `Win1250`, `Win1251`, `Win1252`). | [docs.rs](https://docs.rs/jess-io) |
| `jess-luau-types` | `jess_luau_types` | `LuauType`: spells a serde shape as a Luau type. Record and NIF types derive it, which is how `jess luau-types` knows every field. | [docs.rs](https://docs.rs/jess-luau-types) |
| `jess-nif` | `jess_nif` | Read, write, and check NIF and KF files. Writes Morrowind NIFs; reads everything OpenMW reads. | [docs.rs](https://docs.rs/jess-nif) |
| `jess-plugin` | `jess_plugin` | The raw-preserving plugin spine. Unedited plugins save byte for byte. Also OpenMW save helpers (`openmw_save`). | [docs.rs](https://docs.rs/jess-plugin) |
| `jess-omwaddon` | `omwaddon` | Typed Morrowind/OpenMW records, views, and editors over `jess-plugin`. Lint, clean, diff, selectors. | [docs.rs](https://docs.rs/jess-omwaddon) |
| `jess-openmw` | `jess_openmw` | Reads `openmw.cfg`, edits its content list, and resolves names through the VFS the engine builds (`OpenMwContext`). No record types. | [docs.rs](https://docs.rs/jess-openmw) |
| `jess-assets` | `jess_assets` | Checks the files plugins and NIFs refer to, looked up the way OpenMW finds them, in folders, archives, or the load order's VFS. | [docs.rs](https://docs.rs/jess-assets) |
| `jess-save` | `jess_save` | Inspect, check, and strip OpenMW saves; read Morrowind `.ess` saves. | [docs.rs](https://docs.rs/jess-save) |
| `jess-render` | `jess_render` | A backend-independent render graph IR: targets, passes, ordering, culling, lifetimes, aliasing, and a debug dump. | [docs.rs](https://docs.rs/jess-render) |
| `jess-runtime` | `jess_runtime` | Script domains over l3i: isolated Luau states with contexts, capability grants, workers, and generations. `jess.context` and `jess.worker` are its Luau frontends. | [docs.rs](https://docs.rs/jess-runtime) |
| `jess-input` | `jess_input` | Input before GUI: a device vocabulary, timestamped events and state, a serializable action graph with sets and layers, and an allocation-free evaluator. `jess.input` binds it, and `@jess/input` is the Luau over that: labels, conflicts, and intents. | [docs.rs](https://docs.rs/jess-input) |
| `jess-host` | `jess_host` | The application kernel: a script domain with its live input attached as a service, the frame that ticks input into the input phase, restart, and teardown. | [docs.rs](https://docs.rs/jess-host) |
| `jess-shader` | `jess_shader` | A typed shader expression and program IR that validates at the author's source line and lowers to WGSL. `jess.shader` is its Luau frontend. | [docs.rs](https://docs.rs/jess-shader) |
| `jess` | `jess` | The facade: re-exports the library crates (not `jess-lua`) behind one feature each. | [docs.rs](https://docs.rs/jess) |
| `jess-lua` | `jess_lua` | The Luau API and the bundled load-order scripts. | [docs.rs](https://docs.rs/jess-lua) |
| `jess-cli` | (binary `jess`) | The command-line tool. `cargo install jess-cli`. Not a library. | |

One name doesn't match its package, and that trips people up: `jess-omwaddon` is `use omwaddon::...`. Everything else is the package name with underscores.

## Which one do I want?

- **Copy, clean, or surgically edit plugins without disturbing anything else**: `jess-plugin`. See [Plugins in Rust](@/docs/rust/plugins.md).
- **Read records as structs**: `jess-omwaddon` (pulls in `jess-plugin`).
- **Meshes**: `jess-nif`. Checking many of them, or checking what they point at: add `jess-assets`. See [NIFs in Rust](@/docs/rust/nifs.md).
- **Saves**: `jess-save`. See [Saves in Rust](@/docs/rust/saves.md).
- **Let your users script it**: `jess-lua`. See [Embedding jess-lua](@/docs/rust/embedding-lua.md).
- **Writing your own binary format crate**: `jess-io` on its own is a decent little `Load`/`Save` toolkit.

## The `jess` facade

One feature per library crate, named after the module it adds:

| Feature | Default | Gives you |
| --- | --- | --- |
| `plugin` | yes | `jess::plugin` (`jess-plugin`). |
| `omwaddon` | yes | `jess::omwaddon` (`jess-omwaddon`). |
| `nif` | yes | `jess::nif` (`jess-nif`). |
| `save` | yes | `jess::save` (`jess-save`). |
| `openmw` | no | `jess::openmw` (`jess-openmw`): reads `openmw.cfg` and builds the VFS (`openmw-config`, `vfstool_lib`). |
| `assets` | no | `jess::assets` (`jess-assets`). |
| `serde` | no | `Serialize`/`Deserialize` and Luau type metadata in every enabled crate that has them (`omwaddon`, `nif`). |

`serde` only turns on features of crates you've enabled (`omwaddon?/serde`), so it does nothing alone. A build with only `nif` compiles no TES3 record code, and one with only `plugin`/`omwaddon` compiles no NIF code; each format crate turns on only its own `jess-macros` feature. `save` and `assets` need `omwaddon`'s records and turn on its `serde` themselves.

The Luau API is behind the facade's `lua` feature, off by default: it builds Luau from source and needs l3i's clang and lld toolchain. Depending on `jess-lua` directly is the same code.

Depending on the crates directly? `jess-omwaddon` has `serde`; `jess-nif` has `serde` (which also brings its Luau types); `jess-macros` has `tes3`, `tes3-serde`, and `nif`, which the format crates turn on for you; `jess-luau-types` has `glam`, `hashbrown`, and `reflect`. Everything else has no features worth knowing about.

## Versions and MSRV

- **One version for everything.** Every crate shares the workspace version and release together. `jess-nif 0.1.x` goes with `jess-plugin 0.1.x`. Don't mix.
- **0.x means it can break.** A minor bump (0.1 → 0.2) can change anything; patch bumps shouldn't. Pin the minor version: `"0.1"`, not `"0"` or `"*"`.
- **MSRV is Rust 1.88**, edition 2024. CI checks the whole workspace against 1.88 before every release (see [Building and Testing](@/docs/contributing/building-and-testing.md)), so raising it is a deliberate, changelogged decision.
- **Licenses:** every crate is MIT OR Apache-2.0, and so is everything they link: l3i, vfstool, openmw-config, and, from their 1.1.0 releases on, dream-path and dream_archive (through vfstool). Every crate ships both license texts.

Breaking changes are in each release's notes on the [releases page](https://github.com/DreamWeave-MP/jess/releases).
