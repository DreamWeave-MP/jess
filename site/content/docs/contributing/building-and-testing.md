---
title: 'Building and Testing'
description: 'The gates CI runs, and the real-data tests that run when you point them at data.'
weight: 10
extra:
  kind: guide
---

jess is a plain Cargo workspace: stable Rust 1.92 or newer, no build scripts of its own, no system libraries to hunt down. Luau compiles itself from source. The one thing you need besides Rust is clang and lld on the same LLVM major as your `rustc` (`rustc -vV` and `clang --version` tell you), because the repo's `.cargo/config.toml` compiles Luau with clang and links everything with lld under cross-language thin LTO. That's the toolchain [l3i](https://github.com/DreamWeave-MP/l3i) is measured on, and its build script refuses anything else rather than hand you a slower binary quietly.

```sh
sudo dnf install clang lld        # Fedora; Debian and Ubuntu want clang-N lld-N for rustc's LLVM N

git clone https://github.com/DreamWeave-MP/jess
cd jess
cargo build --all-features
cargo run -p jess-cli -- --help
```

## Building Without LTO

The verified toolchain LTOs everything, test binaries included: rustc writes LLVM bitcode, and lld generates the machine code at every link, on every core, whatever `CARGO_BUILD_JOBS` says. That's right for what ships and wrong for an edit-and-test loop, where it is most of the wall time and nearly all of the CPU. For development, turn it off:

```sh
. tools/dev/no-lto.sh
CARGO_BUILD_JOBS=8 cargo test -p jess-nif --all-features
```

It sets `RUSTFLAGS` to plain clang and lld, which replaces the config's flags (`--config` and `CARGO_TARGET_<triple>_RUSTFLAGS` would only add to them), sets `L3I_UNVERIFIED_TOOLCHAIN=1` so l3i's build script warns instead of refusing, and builds into `target/fast`, so the verified build in `target/` survives switching back. Source it in the shell you build jess from, not in your profile: it would reach every other cargo project too.

On the 16-thread laptop jess is developed on, after a one-line edit, rebuilding the tests that depend on it:

| Edit, then rebuild | Verified, `-j2` | No LTO, `-j8` |
| --- | --- | --- |
| `jess-plugin`, then `jess-cli` (relinks `jess`) | 57 to 72 s wall, 410 to 460 s CPU | 7 to 12 s, 9 to 15 s CPU |
| `jess-nif`, then its tests | 12 to 13 s, 74 s CPU | 5 to 9 s, 7 to 12 s CPU |
| `jess-omwaddon`, then its tests and `jess-nif`'s | 14 to 26 s, 110 to 195 s CPU | 5 to 7 s, 8 to 14 s CPU |

The verified column is at two jobs because more parallel LTO links than that take the machine down; without LTO, eight is comfortable.

**A no-LTO build is never measured.** Luau runs without cross-language inlining in it, so it is slower than what ships, by an amount that varies with the workload. `jess --version` says `unverified toolchain, no cross-language LTO: not for measuring`, the bench harness refuses such a binary, and the Luau benches refuse to run. Every performance number in these docs comes from a release build with the verified toolchain: a shell without `tools/dev/no-lto.sh`, and `cargo build --release`.

## What CI checks

Every push and pull request goes through [StroggForge](https://github.com/DreamWeave-MP/StroggForge)'s `rustGlobalBuild.yml@v54`, called from `.github/workflows/release.yaml`. All of these block a release. Run them before you push and you won't meet them there:

| Gate | Command | Where |
| --- | --- | --- |
| Tests | `cargo test --workspace --all-targets --all-features -- --show-output` | macOS (ARM and Intel), Ubuntu 22.04, Windows |
| Format | `cargo fmt --all --check` | Ubuntu |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -W clippy::pedantic -D warnings`, then again with only the features the release binary ships | Every platform the tests run on |
| Audit | `cargo audit`, against the committed `Cargo.lock` | Ubuntu |
| MSRV | `cargo +1.92 check --workspace --all-features --all-targets` | Ubuntu, Rust 1.92 |

Things worth knowing about those:

- **Clippy is strict.** The workspace lints already turn on `pedantic`, `nursery`, and `cargo` as warnings (see `[workspace.lints]` in the root `Cargo.toml`, with the allowed exceptions listed there), and `-D warnings` makes every one an error. If clippy has an opinion, fix it; don't `#[allow]` it without a reason in a comment.
- **Formatting** uses the repo's `rustfmt.toml`: `max_width = 125`, `merge_derives = false`. `cargo fmt` and move on.
- **Clippy runs on Windows and macOS too.** Code behind `cfg(windows)` or `cfg(target_os = "macos")`, and helpers whose only caller is Linux-gated, never meet a Linux clippy, and every round trip through CI costs about 25 minutes. Before you push, lint the Windows target from Linux: `rustup target add x86_64-pc-windows-gnu`, point `CC_x86_64_pc_windows_gnu`/`CXX_x86_64_pc_windows_gnu` at a stub compiler (clippy never links, so a script that answers `--version` and touches its `-o` output will do), set `L3I_UNVERIFIED_TOOLCHAIN=1` and a separate `CARGO_TARGET_DIR`, and run clippy with `--target x86_64-pc-windows-gnu`. macOS can't be cross-checked; read its `cfg` code by eye, remember APFS is case-insensitive and refuses non-UTF-8 file names, and gate such tests `target_os = "linux"`.
- **MSRV** is read from `rust-version` in `Cargo.toml` (`msrv: auto`). The check runs with `CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback`, so dependencies resolve to versions that still build on 1.92. If you use a newer std API, this is where you find out. Rust 1.92 ships an older LLVM than current clang, so a local `cargo +1.92 check` needs `L3I_UNVERIFIED_TOOLCHAIN=1`; a check never links, so the mismatch can't matter there.
- **Publish dry run**: on non-tag pushes, `cargo publish --workspace --dry-run` packages every crate against the others. It doesn't block anything, but if it fails, the next release will too. Usually it means a new file isn't in a crate's `include`, or a path dependency lost its `version`.

For day-to-day work, the faster loop:

```sh
cargo check --workspace --all-features                          # what rust-analyzer sees
cargo clippy --workspace --no-deps --all-features --all-targets
cargo nextest run --workspace --all-features                    # if you have cargo-nextest
cargo hack check --feature-powerset --no-dev-deps               # if you have cargo-hack
```

The `cargo hack` run checks every feature combination of every crate. CI doesn't, so it's on you when you touch `#[cfg(feature = ...)]`.

### Focused tests

```sh
cargo test -p jess-plugin                          # the raw spine
cargo test -p jess-cli -p jess-plugin              # CLI plus spine
cargo test -p jess-omwaddon --all-features         # typed records (the JSON round trip needs serde)
cargo test -p jess-nif --all-features load_save    # one test by name
```

CLI tests run the real `jess` binary and parse its `--format json` output. If you add a command, test it that way.

A crate's integration tests are one binary: `tests/main.rs` declares each file as a module, and the crate's `Cargo.toml` has `autotests = false` and one `[[test]]` named after the crate. Every test binary is another link, so a new test file goes in `tests/main.rs`, not beside it as a binary of its own. Pick one file's tests with a filter:

```sh
cargo test -p jess-cli --test cli s3lightfixes::   # libs/cli/tests/s3lightfixes.rs
cargo test -p jess-nif --all-features --test nif scene::
```

## Byte-identical round trips

Reading a file and saving it unchanged must give back the same bytes. Not "equivalent". The same. It's the one rule jess doesn't bend (see [Format Notes](@/docs/contributing/format-notes.md) for why), and there's a tool for it:

```sh
cargo run -p jess-cli -- plugin test-codec MyMod.esp OtherMod.esp
```

`test-codec` parses and saves each plugin and compares. `--continue` keeps going past failures, `-x TAG` leaves a record type out, `--ignore-cruft` skips junk before the `TES3` record. It's a development tool, so it only exists in debug builds: `cargo run` has it, a release binary doesn't.

## Real data

Game data can't ship with jess, so the tests that matter most are opt-in. Each one reads an environment variable, and without it passes trivially (it tells you it's skipping). Point them at your install before you touch a parser. Use `--release`: these read a lot of data.

| Variable | Points at | Run |
| --- | --- | --- |
| `JESS_DATA_FILES` | Morrowind's `Data Files` folder | `cargo test --release -p jess-plugin --test plugin retail_morrowind::` and `cargo test --release -p jess-omwaddon --test omwaddon retail::` |
| `JESS_SAVES` | A folder of `.omwsave` files | `cargo test --release -p jess-plugin --test plugin openmw_save::` |
| `JESS_NIF_DIRS` | Folders of loose NIFs, separated like `PATH` | `cargo test --release -p jess-nif --all-features --test nif gamebryo:: -- --nocapture` |
| `JESS_NIF_ARCHIVES` | BSA/BA2 archives, separated like `PATH`, read in place | same as above |

What they hold you to:

- **Retail masters** (`JESS_DATA_FILES`): whichever of `Morrowind.esm`, `Tribunal.esm`, and `Bloodmoon.esm` it finds round-trip byte for byte through the raw spine, reparse to the same layout, and pass the vanilla profile. On the typed side, writing back an unchanged record keeps every byte, a flag edit changes only the record header, and moving a cell reference changes exactly one subrecord.
- **Saves** (`JESS_SAVES`): every save clones byte-identically and has a sane shape.
- **NIFs** (`JESS_NIF_DIRS`, `JESS_NIF_ARCHIVES`): every record of every file reads exactly its own size and every file parses. Point it at other games' archives too; the multi-version reader is supposed to handle everything OpenMW does. `--nocapture` prints the tallies.

The same two NIF variables drive the Luau round trip:

```sh
JESS_NIF_DIRS=~/Morrowind/Data\ Files/Meshes \
  cargo test --release -p jess-lua --all-features --test real_nifs -- --nocapture
```

That pushes every record of real Morrowind NIFs through Luau tables (`record:set(record:decode())`) and requires the saved file to match the original byte for byte. It's what keeps the serde layer in `libs/lua/src/convert.rs` honest about NaN, `-0`, and the rest.

`jess-plugin`'s `openmw_save` tests also compare `CURRENT_FORMAT_VERSION` against the OpenMW checkout at `./openmw` (gitignored; CI doesn't have one, so it skips there). See [Format Notes](@/docs/contributing/format-notes.md#openmw-save-format-bumps).

## Luau types and goldens

- **Type checks** need nothing installed. `cargo test -p jess-lua` checks the example scripts in `libs/lua/tests/typecheck.luau`, the bundled scripts, the template, the contract fixtures, the shader corpus, both Luau packages, and every record of `all_types.esp` against the types folder, through l3i's embedded Luau frontend: `types/jess.d.luau` loaded as definitions, `require` resolved through the stubs as the `.luaurc` aliases resolve it, every module strict (`tests/common/mod.rs`, `type_check`). If you change a record type (`#[esp_meta]` types in `jess-omwaddon`) or a NIF record (the `LuauType` derive in `jess-macros`), these run on what you changed; the types are generated from them. `jess luau-types <FOLDER>` writes the result.
- **`JESS_BLESS=1`**: rewrites the S3LightFixes and Merged Lands goldens in `libs/cli/tests/s3lightfixes/` and `libs/cli/tests/mergedlands/` from the current run, instead of comparing against them. Those goldens are what the originals wrote, so a diff there means `@dreamwind/tools/s3lightfixes` or `@dreamwind/tools/merged-lands` stopped matching them. Bless only when you meant that, and say why in the commit.

## Disk space

`target/` gets big. `--all-features` builds, release builds for the real-data tests, and `cargo doc` add up to tens of gigabytes fast. `cargo clean` now and then, or `cargo clean --release` when you're done with real data.

Don't run the ignored tests in `libs/nif/src/types/nistream.rs` for fun: they're visual checks that write `tests/assets/random_objects~1.nif` into the tree.

## The docs site

The docs are a Jess workload. The site generator is a separate package: `buildSite` resolves `JESS_SITE_PACKAGE`, then a `jpk/dreamweave-site` checkout, then an ignored local `tools/site/dreamweave-site.jpk` artifact. The bootstrap activates it with the same runtime and package machinery application code uses. From the repository root:

```sh
./buildSite                 # production build into site/public
./buildSite /tmp/jess-site  # alternate production output directory
./buildSite serve           # dev build, localhost:1111, rebuild on edits
```

The generator renders Markdown through its Tera-shaped template layer, compiles Sass, copies static assets, and writes the search index and sitemap. `buildSite` assumes `jess` is already in `$PATH`; no separate Zola install is part of the build.

`./buildSite serve [HOST] [PORT]` defaults to `127.0.0.1:1111`. Dev builds override `base_url` to `/`, so CSS, scripts, search, and documentation links stay on the local server instead of jumping to GitHub Pages. The watcher rebuilds through Jess and preserves the last successful output if an edit fails. Serving itself is a small Python 3 standard-library shim for now: L3i exposes DreamNet transport to Luau, not a browser-facing raw TCP listener.

Link between pages with source paths such as `@/docs/scripting/packages.md`; the generator resolves those against the configured site URL. `site/Readme.md` has the site layout and writing conventions.
