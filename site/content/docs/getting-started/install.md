---
title: 'Install'
description: 'Release archives, cargo install, PATH setup, shell completions, man pages, upgrading, and uninstalling.'
weight: 10
extra:
  kind: guide
---

jess is one binary called `jess`. Get it onto your `PATH` and you're done; completions and man pages are optional extras it can generate for itself.

## From a release archive

The [GitHub releases](https://github.com/DreamWeave-MP/jess/releases) page has one zip per platform:

| Archive | For |
| --- | --- |
| `jess-Linux-X64.zip` | 64-bit x86 Linux with glibc 2.28 or newer: Debian 10, Ubuntu 20.04, RHEL 8, and anything later. |
| `jess-Windows-X64.zip` | 64-bit Windows. |
| `jess-macOS-ARM64.zip` | Apple Silicon Macs. |
| `jess-macOS-X64.zip` | Intel Macs. |

There's no ARM Linux or 32-bit build; use `cargo install` there. Each zip holds, with no folder around them:

- `jess` (`jess.exe` on Windows), the whole tool
- `jess-<platform>.bundle`, the binary's [Sigstore](https://www.sigstore.dev) signature (below)
- `jess-README.md`
- `LICENSE-MIT` and `LICENSE-APACHE`, the two license texts (pick either)
- `docs/save-strip.md`, the notes on how `save strip` decides what's safe to remove

Everything else lives on this site. The Luau types aren't in the zip, on purpose: `jess init` or `jess luau-types` writes them for the jess you actually have (see [Editor Setup](@/docs/getting-started/editor-setup.md)).

The binary is signed with [cosign](https://github.com/sigstore/cosign) from the GitHub Actions run that built it. To check that the `jess` you unpacked is the one CI built:

```sh
cosign verify-blob jess --bundle jess-Linux-X64.bundle \
  --certificate-identity-regexp '^https://github.com/DreamWeave-MP/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

Unpack it and put `jess` somewhere on your `PATH`.

**Linux and macOS.** `~/.local/bin` is the usual spot:

```sh
mkdir -p ~/.local/bin
cp jess ~/.local/bin/
chmod +x ~/.local/bin/jess
```

If your shell can't find it afterwards, add `~/.local/bin` to your `PATH`:

| Shell | Add to | Line |
| --- | --- | --- |
| bash | `~/.bashrc` | `export PATH="$HOME/.local/bin:$PATH"` |
| zsh | `~/.zshrc` | `export PATH="$HOME/.local/bin:$PATH"` |
| fish | run once | `fish_add_path ~/.local/bin` |

On macOS, a binary you downloaded may be quarantined, and Gatekeeper refuses to run it; `xattr -d com.apple.quarantine ~/.local/bin/jess` lets it run.

**Windows.** Make a folder such as `C:\Tools\jess` and put `jess.exe` in it. Then add that folder to your user `Path`, either through *Start* → *Edit environment variables for your account* → *Path* → *Edit* → *New*, or from PowerShell:

```powershell
$dir = "C:\Tools\jess"
$path = [Environment]::GetEnvironmentVariable("Path", "User")
[Environment]::SetEnvironmentVariable("Path", "$path;$dir", "User")
```

Open a new terminal afterwards; old ones keep the old `Path`.

## From crates.io

With a Rust toolchain (1.88 or newer):

```sh
cargo install jess-cli
```

That builds the `jess` binary into `~/.cargo/bin`, which rustup already put on your `PATH`. It compiles Luau from source, so you also need a C++ compiler (the one your platform's Rust setup normally uses), and the first build takes a few minutes. Upgrade later by running the same command again.

## From a git checkout

```sh
git clone https://github.com/DreamWeave-MP/jess
cd jess
cargo build --release -p jess-cli
```

The binary lands at `target/release/jess`. `cargo run -p jess-cli -- <command>` runs a debug build without installing anything; debug builds also have `jess plugin test-codec`, the byte-for-byte round-trip checker that release builds leave out.

## Check it works

```sh
jess --version
```

```text
jess 0.1.0
```

Every subcommand takes `--version` too, and `jess help` (or `jess <group> --help`) lists what's there.

## Shell completions

`jess completions <shell>` prints a completion script for `bash`, `elvish`, `fish`, `powershell`, or `zsh`. Save it where your shell looks, creating the folder if it's missing:

| Shell | Command |
| --- | --- |
| fish | `jess completions fish > ~/.config/fish/completions/jess.fish` (fish loads it on the next completion; no restart) |
| bash | `jess completions bash > ~/.local/share/bash-completion/completions/jess` |
| zsh | `jess completions zsh > ~/.zfunc/_jess`, with `fpath=(~/.zfunc $fpath)` before `compinit` in `~/.zshrc` |
| elvish | `jess completions elvish > ~/.config/elvish/lib/jess.elv`, then `use jess` in `rc.elv` |
| PowerShell | `jess completions powershell >> $PROFILE` |

Regenerate them after upgrading jess, so new commands and flags complete.

## Man pages

`jess man` prints `jess(1)` to stdout:

```sh
jess man > jess.1
man ./jess.1
```

`jess man --output <folder>` writes `jess.1` plus a page for every command (`jess-plugin-dump.1`, `jess-save-strip.1`, and so on) and lists what it wrote. `man` looks for pages in a `man1` folder under each entry of its search path, so write them straight into one:

```sh
jess man --output ~/.local/share/man/man1
man jess-plugin-dump
```

On most Linux systems `~/.local/share/man` is already searched. If `man jess` says there's no entry, add `export MANPATH="$HOME/.local/share/man:"` to your shell's rc file (the trailing `:` keeps the system pages).

## Editor types for scripters

If you're going to write Luau scripts, `jess init my-scripts` sets up a folder with the types for the jess you have installed, an example script, and your editor's luau-lsp settings. [Editor Setup](@/docs/getting-started/editor-setup.md) covers VS Code, Neovim, Zed, and Helix.

## Upgrading

Replace the binary the same way you installed it: unpack the new zip over the old `jess`, or rerun `cargo install jess-cli`. Then refresh what was generated from the old one:

- **Script types:** `jess init <folder>` in each script folder rewrites `types/` and leaves your files alone.
- **Completions and man pages:** rerun the commands above, so new commands and flags show up.

Read the release notes on the [releases page](https://github.com/DreamWeave-MP/jess/releases) first. Until 1.0, a minor version can rename a flag.

## Uninstalling

jess keeps no state of its own: no config folder, no cache, no registry entries. Delete what you put on disk:

- the binary (`~/.local/bin/jess`, `C:\Tools\jess\jess.exe`, or `cargo uninstall jess-cli`), and the `Path` entry if you added one;
- the completion file and man pages, if you generated them;
- any `.bak` backups jess made next to files it edited, once you're sure you don't want them.

Two things you may have asked for without noticing: a script that activates content (`openmw:save()`, or a load-order tool's `activate = true`) edits your user `openmw.cfg` and leaves `openmw.cfg.bak` beside it, and the [load-order tools](@/docs/bundled-scripts/_index.md) write their plugins to `data-local` (S3LightFixes also writes `lightconfig.toml` next to your user `openmw.cfg`). Remove their `content=` lines before deleting their plugins.

## Next

Run the [first commands](@/docs/getting-started/first-commands.md) against your own setup. Only `jess init` writes anything.
