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
| `jess-Portmaster-ARM64.zip` | 64-bit ARM Linux with glibc 2.34 or newer: PortMaster handhelds, Raspberry Pi OS Bookworm, Ubuntu 22.04, RHEL 9, and anything later. |

There are no 32-bit or Android release archives. A source build on those targets is not an established supported distribution. Each zip holds, with no folder around them:

- `jess` (`jess.exe` on Windows), the whole tool
- `jess-<platform>.bundle`, the binary's [Sigstore](https://www.sigstore.dev) signature (below)
- `jess-README.md`
- `LICENSE-MIT` and `LICENSE-APACHE`, the two license texts (pick either)

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

With a Rust toolchain (1.92 or newer):

```sh
cargo install jess
```

That builds the `jess` binary into `~/.cargo/bin`, which rustup already put on your `PATH`, and takes a few minutes, since it compiles Luau from source. Run exactly as above it will stop with a message from [L3i](https://DreamWeave-MP.github.io/l3i/docs/building/)'s build script, the runtime underneath jess, which refuses to build without the toolchain it is measured on rather than quietly hand you a slower binary. You have two choices.

**The verified build**, the one the release archives are: clang and lld on the same LLVM major as your `rustc` (`rustc -vV` and `clang --version` show the majors; on Debian and Ubuntu that's `clang-N lld-N` for rustc's LLVM N, on Fedora `clang lld`, on macOS LLVM's own release rather than Apple's clang), with the flags L3i checks for:

```sh
CC=clang CXX=clang++ RUSTFLAGS='-Clinker-plugin-lto -Clinker=clang -Clink-arg=-fuse-ld=lld' cargo install jess
```

**The unverified build**, with whatever C++ compiler your platform has and no cross-language LTO:

```sh
L3I_UNVERIFIED_TOOLCHAIN=1 cargo install jess
```

It works, and it says what it is: `jess --version` prints `(unverified toolchain, no cross-language LTO: not for measuring)`, and `jess.bench` refuses to post a number from it. For a binary to measure with, take the archive. Upgrade later by running the same command again.

## From a git checkout

```sh
git clone https://github.com/DreamWeave-MP/jess
cd jess
cargo build --release
```

The binary lands at `target/release/jess`. `cargo run -- <command>` runs a debug build without installing anything. [Building and Testing](@/docs/contributing/building-and-testing.md) has the rest.

## What it needs at run time

Nothing beyond the binary, to start. A release reaches the machine's GPU, window system, and sound system when a script first asks for one, not when jess starts, so a script that never draws or plays runs anywhere the binary does.

| A script that uses | Needs | Without it |
| --- | --- | --- |
| [`jess.gpu.device`](@/docs/api/gpu.md) | A GPU API and its driver: Vulkan, or OpenGL, on Linux; Metal on macOS; Direct3D 12, Vulkan, or OpenGL on Windows. | A software adapter where the system has one, which draws correctly and slowly; with none at all, the call fails and says so. |
| [`jess.gpu.window`](@/docs/api/gpu.md#windows) | A display: a Wayland or X11 session on Linux, the desktop on macOS and Windows. | The call fails (`no window system: …`), and everything offscreen still works. |
| [`jess.audio.output`](@/docs/api/audio.md) | A sound system: PulseAudio (which PipeWire also serves), ALSA, or JACK on Linux; Core Audio on macOS; WASAPI on Windows. | The null device, which plays to nowhere in real time. |

So a server, a container, or a session over SSH runs jess headless without complaint. [Troubleshooting](@/docs/reference/troubleshooting.md#gpus-windows-and-sound) has what each refusal means.

## Check it works

```sh
jess --version
```

```text
jess 0.3.0
```

Every subcommand takes `--version` too, and `jess help` (or `jess <command> --help`) lists what's there.

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

`jess man --output <folder>` writes `jess.1` plus a page for every command (`jess-package-inspect.1`, `jess-workspace-sync.1`, and so on) and lists what it wrote. `man` looks for pages in a `man1` folder under each entry of its search path, so write them straight into one:

```sh
jess man --output ~/.local/share/man/man1
man jess-package-inspect
```

On most Linux systems `~/.local/share/man` is already searched. If `man jess` says there's no entry, add `export MANPATH="$HOME/.local/share/man:"` to your shell's rc file (the trailing `:` keeps the system pages).

## Editor types for scripters

If you're going to write Luau, `jess init my-scripts` sets up a folder with the types for the jess you have installed, an example script, and your editor's luau-lsp settings. [Editor Setup](@/docs/getting-started/editor-setup.md) covers VS Code, Neovim, Zed, and Helix.

## Upgrading

Replace the binary the same way you installed it: unpack the new zip over the old `jess`, or rerun `cargo install jess`. Then refresh what was generated from the old one:

- **Script types:** `jess init <folder>` in each script folder rewrites `types/` and leaves your files alone.
- **Completions and man pages:** rerun the commands above, so new commands and flags show up.

Read the release notes on the [releases page](https://github.com/DreamWeave-MP/jess/releases) first. Until 1.0, a minor version can rename a flag.

## Uninstalling

jess keeps no state of its own: no config folder, no cache, no registry entries. What it writes, it writes where you pointed it. Delete what you put on disk:

- the binary (`~/.local/bin/jess`, `C:\Tools\jess\jess.exe`, or `cargo uninstall jess`), and the `Path` entry if you added one;
- the completion file and man pages, if you generated them;
- the `types/` folders `jess init` wrote, if you don't want them, and a workspace's `.jess/packages`, which [`jess workspace sync`](@/docs/cli/workspace.md) fills and recreates on demand;
- any `.bak` files jess made next to files it replaced with `--force`, once you're sure you don't want them.

## Next

Write [your first script](@/docs/scripting/intro.md). Only `jess init` writes anything, and only in the folder you name.
