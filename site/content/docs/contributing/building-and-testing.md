---
title: 'Building and Testing'
description: 'The toolchain, the development build, the gates CI runs, and how the test suite is laid out.'
weight: 10
extra:
  kind: guide
---

jess is one Cargo crate: stable Rust 1.92 or newer, no build scripts of its own, no system libraries to hunt down. Luau compiles itself from source. The one thing you need besides Rust is clang and lld on the same LLVM major as your `rustc` (`rustc -vV` and `clang --version` tell you), because the repo's `.cargo/config.toml` compiles Luau with clang and links everything with lld under cross-language thin LTO. That's the toolchain [l3i](https://github.com/DreamWeave-MP/l3i) is measured on, and its build script refuses anything else rather than hand you a slower binary quietly.

```sh
sudo dnf install clang lld        # Fedora; Debian and Ubuntu want clang-N lld-N for rustc's LLVM N

git clone https://github.com/DreamWeave-MP/jess
cd jess
cargo build --all-features
cargo run -- --help
```

## Building Without LTO

The verified toolchain LTOs everything, test binaries included: rustc writes LLVM bitcode, and lld generates the machine code at every link, on every core, whatever `CARGO_BUILD_JOBS` says. That's right for what ships and wrong for an edit-and-test loop, where it is most of the wall time and nearly all of the CPU. For development, turn it off:

```sh
. tools/dev/no-lto.sh
CARGO_BUILD_JOBS=8 cargo test --test lua_package
```

It sets `RUSTFLAGS` to plain clang and lld, which replaces the config's flags (`--config` and `CARGO_TARGET_<triple>_RUSTFLAGS` would only add to them), sets `L3I_UNVERIFIED_TOOLCHAIN=1` so l3i's build script warns instead of refusing, and builds into `target/fast`, so the verified build in `target/` survives switching back. Source it in the shell you build jess from, not in your profile: it would reach every other cargo project too, l3i's own benchmarks included.

The verified build wants two jobs at most: more parallel LTO links than that take a machine down. Without LTO, eight is comfortable.

**A no-LTO build is never measured.** Luau runs without cross-language inlining in it, so it is slower than what ships, by an amount that varies with the workload. `jess --version` says `unverified toolchain, no cross-language LTO: not for measuring`, `jess.bench.verified` is `false`, and the [bench harness](@/docs/contributing/benchmarks.md) refuses to run. A measurement comes from a release build with the verified toolchain: a shell without `tools/dev/no-lto.sh`, and `cargo build --release`.

## What CI checks

Every push and pull request goes through [StroggForge](https://github.com/DreamWeave-MP/StroggForge)'s `rustGlobalBuild.yml`, called from `.github/workflows/release.yaml`. All of these block a release. Run them before you push and you won't meet them there:

| Gate | Command |
| --- | --- |
| Format | `cargo fmt --all --check` |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -W clippy::pedantic -D warnings` |
| Tests | `cargo test --workspace --all-targets --all-features` |
| MSRV | `L3I_UNVERIFIED_TOOLCHAIN=1 cargo +1.92 check --workspace --all-features --all-targets` |
| Docs | `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps` |
| The built-in package | `jess package test jpk/builtin` |

Things worth knowing about those:

- **Clippy is strict.** The lints already turn on `pedantic`, `nursery`, and `cargo` as warnings (see `[workspace.lints]` in `Cargo.toml`, with the allowed exceptions listed there), and `-D warnings` makes every one an error. If clippy has an opinion, fix it; don't `#[allow]` it without a reason in a comment. The gate is never left red, including a failure already on `main` that your change didn't cause.
- **Formatting** uses the repo's `rustfmt.toml`: `max_width = 125`, `merge_derives = false`. `cargo fmt` and move on.
- **Clippy runs on Windows and macOS too.** Code behind `cfg(windows)` or `cfg(target_os = "macos")`, and helpers whose only caller is Linux-gated, never meet a Linux clippy, and every round trip through CI costs about 25 minutes. Before you push, lint the Windows target from Linux: `rustup target add x86_64-pc-windows-gnu`, point `CC_x86_64_pc_windows_gnu`/`CXX_x86_64_pc_windows_gnu` at a stub compiler (clippy never links, so a script that answers `--version` and touches its `-o` output will do), set `L3I_UNVERIFIED_TOOLCHAIN=1` and a separate `CARGO_TARGET_DIR`, and run clippy with `--target x86_64-pc-windows-gnu`. macOS can't be cross-checked; read its `cfg` code by eye, remember APFS is case-insensitive and refuses non-UTF-8 file names, and gate such tests `target_os = "linux"`.
- **MSRV** is read from `rust-version` in `Cargo.toml` (`msrv: auto`). Rust 1.92 ships an older LLVM than current clang, so a local `cargo +1.92 check` needs `L3I_UNVERIFIED_TOOLCHAIN=1`; a check never links, so the mismatch can't matter there.
- **Tests must not depend on the platform.** No Linux path separators, no case-sensitive filesystems, no non-UTF-8 names: compare resolved paths as paths, keep case twins off disk, and gate symlink tests with `#[cfg(unix)]`.

For day-to-day work, the faster loop:

```sh
cargo check --workspace --all-features                          # what rust-analyzer sees
cargo clippy --workspace --no-deps --all-features --all-targets
cargo nextest run --workspace --all-features                    # if you have cargo-nextest
cargo hack check --feature-powerset --no-dev-deps               # if you have cargo-hack
```

The `cargo hack` run checks every feature combination. CI doesn't, so it's on you when you touch `#[cfg(feature = ...)]`.

## The test suite

Integration tests are explicit `[[test]]` binaries (`autotests = false`), because every test binary is another l3i link. Each is one root file under `tests/` that declares its modules; a new test file goes into the binary that owns the area, never beside it as a binary of its own.

| Binary | Root | Covers |
| --- | --- | --- |
| `cli` | `tests/cli/main.rs` | The `jess` binary end to end, one module per file, parsing its `--format json` output |
| `host` | `tests/host/kernel.rs` | The application kernel: frames, the input phase, restart |
| `runtime` | `tests/runtime/main.rs` | Script domains: lifecycle, scheduling, events, workers, services |
| `input` | `tests/input/main.rs` | The action graph and the evaluator, against a golden document |
| `render` | `tests/render/graph.rs` | The render graph IR, against golden dumps |
| `shader` | `tests/shader/main.rs` | The shader IR: builder, validation, folding, WGSL goldens, and a headless triangle |
| `lua_contracts`, `lua_kernel`, `lua_lua_api`, `lua_luau_style`, `lua_network_capability`, `lua_package`, `lua_script_support`, `lua_shader`, `lua_stdlib` | `tests/lua/*.rs` | Everything Luau sees: the API contracts, the kernel from a script's side, the style gate, capabilities, packages, workspaces, applications, the shader front end, and the stdlib |

Prefer the smallest binary that proves a change, then widen:

```sh
cargo test --test lua_package          # packages, resolution, targets, workspaces, applications
cargo test --test cli                  # the binary, end to end
cargo test --test lua_luau_style       # @jess/style over every .jsl under src/ and tools/,
                                       # the site's luau blocks, the definitions, jpk/builtin's suite
cargo test --test runtime --test host --test input
cargo test --test render --test shader --test lua_shader
cargo test --test cli workspace::      # one module of a binary
```

If you add a command, test it through the binary. If you add a member to the `jess` global, `only_generic_members_are_installed` in `tests/lua/contracts.rs` is where its name is pinned, and `jess luau-types` must stub it.

## Luau types and goldens

- **Type checks** need nothing installed. The suite type-checks the built-in package, the contract fixtures, and the shader corpus against the generated definitions, through l3i's embedded Luau frontend: `jess.d.jsl` loaded as definitions, `require` resolved through the stubs as the `.jessrc` aliases resolve it, every module strict (`tests/lua/common/mod.rs`, `type_check`). Nothing type-checks `src/lua/stdlib/` or the `jess init` template; when you change one, check it with `common::type_check` in a throwaway test, and don't commit the throwaway.
- **The style gate** is `cargo test --test lua_luau_style`: [`@jess/style`](@/docs/contributing/luau-style.md) over every `.jsl` under `src/`, `tools/`, and `jpk/builtin`, every ```` ```luau ```` block on this site, and the definitions `jess luau-types` writes. Any finding fails it.
- **`JESS_BLESS=1`** rewrites the goldens (`tests/input/golden`, `tests/render/golden`, `tests/shader/golden`) from the current run instead of comparing against them. Bless only when every change to the output is intended, and say why in the commit.
- **The built-in package** is embedded byte for byte: adding a file to `jpk/builtin` means adding it to `FILES` in `src/lua/builtin.rs`, or `the_embedded_built_in_package_is_its_source` fails.

## Disk space

`target/` gets big. `--all-features` builds, release builds, and `cargo doc` add up to tens of gigabytes fast. Check `du -sh target` around heavy work and `cargo clean` past roughly 12 GB unless the current work needs it. Scratch data goes under `target/` and is deleted when the task is done.

## The docs site

The docs are a Jess workload, end to end. The generator is a package built on Jess, `org.dreamweave.site`, and `tools/site` is a small package of two command targets that drive it over `site/`. Each target declares the capabilities it needs and nothing more, so `--trusted` grants exactly those.

The repository is a [workspace](@/docs/cli/workspace.md): `jess-workspace.toml` lists `jpk/builtin` and `tools/site`, and `jess.lock` pins the generator they depend on and keeps it, as a JPK, in `.jess/packages`. From the repository root (`run` and `test` work from anywhere inside it):

```sh
jess workspace sync --locked                               # restore the pinned generator from its publisher
jess workspace run --trusted jess-site-build               # production build into site/public
jess workspace run --trusted jess-site-build -- target/site # another output folder
jess workspace run --trusted jess-site-serve               # dev build, http://127.0.0.1:63333/, rebuild on edits
jess workspace run --trusted jess-site-serve -- 0.0.0.0 8080
jess workspace test org.dreamweave.jess                    # the built-in package's tests, from source and packed
```

The generator renders Markdown through its template layer, compiles Sass, copies static assets, and writes the search index and sitemap. No Zola, Node, or Python is in the path.

The `serve` target builds into `target/site-dev` with `base_url` as `/`, so CSS, scripts, search, and documentation links stay on the local server, and serves it over HTTP/1.1 from one nonblocking loop on `@dream/tcp`: GET and HEAD, a folder's `index.html`, `404.html` for what isn't there. It polls `config.toml` and everything under `content/`, `templates/`, `sass/`, `static/`, and `data/` about three times a second and rebuilds in-process when a name, size, or modification time changes; a build that fails prints why and leaves the last good one served. It is a dev server and is not meant to face the internet.

Build it for any docs or documented-behavior change. Link between pages with source paths such as `@/docs/scripting/packages.md`; the generator resolves those against the configured site URL, and passes one it can't resolve through as written, so a deleted page leaves a dead link rather than a failed build: grep for its path. Examples on the site are run against the real binary before they are written down, and every ```` ```luau ```` block passes the style gate.
