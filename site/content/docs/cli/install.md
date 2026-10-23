---
title: 'jess install and jess uninstall'
description: 'Put an application on your PATH: a package from its publisher, a packed workspace, or a workspace, embedded in this jess as one executable per command.'
weight: 47
extra:
  kind: reference
---

{{ api_signature(value="jess install PACKAGE [--version VERSION] [--publisher URL]... [--index URL]... [--offline] | FILE | --path WORKSPACE [--root DIR] [--force] [--yes] | --list [--root DIR]") }}

{{ api_signature(value="jess uninstall NAME... [--root DIR]") }}

`jess install` does for a jess application what `cargo install` does for a Rust program: it puts it on your PATH under the names its commands declare, so you run `greet` and not `jess workspace run org.example.greeter:greet`. Each command is a [standalone executable](@/docs/cli/workspace.md#standalone-executables): this jess, carrying the application as a [packed workspace](@/docs/cli/workspace.md#jess-workspace-pack). Nothing is compiled, and what it installs doesn't need jess afterwards, not even the jess that installed it.

## What it installs from

| You name | It installs |
| --- | --- |
| a package id, `org.example.tool` | The newest release its publisher offers that isn't a pre-release, or the one `--version` picks, with what it depends on, packed. |
| a file, `app.jsw` | That packed workspace, as its bytes are. |
| a file that is a standalone executable | The packed workspace it carries, now carried by this jess instead. |
| `--path WORKSPACE` | The workspace that folder is in, packed as [`jess workspace pack`](@/docs/cli/workspace.md#jess-workspace-pack) packs it: its lock has to be fresh. |

A name that is a file is a file; anything else is a package id.

| Option | Does |
| --- | --- |
| `--version <VERSION>` | For a package id: a whole version (`1.2.0`) is that version, and a [requirement](@/docs/api/package.md#package-satisfies) (`^1.2`) the newest release meeting it. A pre-release is installed only when named exactly. |
| `--publisher <URL>` | A publisher to take the package from (repeatable): a DreamWeave site's `dreamweave.json`, or one project manifest. |
| `--index <URL>` | A package index that says who publishes an id (repeatable). Without one, AsHyAmS's; `JESS_REGISTRY_URL` names another. |
| `--offline` | Ask no publisher, as `JESS_OFFLINE=1` does. A package id then has nowhere to come from. |
| `--root <DIR>` | The install root. Without it, `JESS_INSTALL_ROOT`, or else `.jess` in your home folder. |
| `--force` | Take over a command name another install or a stray file has, and install again what is installed already. |
| `--yes` | Install without asking. Needed when there is no terminal to ask in. |
| `--list` | List what is installed instead. |

A package id resolves exactly as [`jess workspace sync`](@/docs/cli/workspace.md#publishers-and-indexes) resolves one: from the one publisher named or indexed, every download checked against the manifest's SHA-256 and then the content digest, in a runtime that runs no package code. It happens in a workspace of the install's own, which keeps the lock and the packages, and is then packed. Every target the package declares is installed.

## Installing

```text
$ jess install --path .
org.example.showcase 0.1.0 installs into /home/you/.jess/bin:
  greet     runs org.example.greeter:greet, holding process.environment
  showcase  runs org.example.showcase:showcase, holding no capabilities
Install? [y/N] y
package: org.example.showcase
version: 0.1.0
bin: /home/you/.jess/bin
commands:
- name: greet
  path: /home/you/.jess/bin/greet
- name: showcase
  path: /home/you/.jess/bin/showcase
removed: []
warning: /home/you/.jess/bin is not on your PATH; add it to run what you installed by name
$ greet you
HELLO, YOU! (hello)
```

Every command the packed workspace exports gets an executable named for its target's `command`, and its default target, if it has no `command`, one named for the target. Each runs its own target with every argument it's given, holding what that target declares, as [`jess app.jsw`](@/docs/cli/workspace.md#running-a-packed-workspace) does. The install is named for the package holding the default target, or for the one package whose targets it exports.

Before anything is written to `bin`, it says what each command runs and what that holds, and asks. That's the moment you decide to trust it: installing a program from someone is trusting them, the same as running one they sent you. In a terminal you answer; with none, nothing is installed unless you pass `--yes`:

```text
error: installing it needs your yes, and there is no terminal to ask in; pass --yes
```

Installing a newer version asks again only when it asks for more: a command the installed one didn't have, or a capability one of its commands didn't declare. Installing exactly what is installed already, the same packed workspace into the same jess, changes nothing:

```text
$ jess install --path .
package: org.example.showcase
version: 0.1.0
state: already installed
```

`jess install` never edits your PATH. Add the `bin` folder once, the way your shell does it.

## From a publisher

```text
$ jess install org.example.tool --publisher https://publisher.example/dreamweave.json
org.example.tool 1.0.0 from https://publisher.example/dreamweave/projects/6f72672e-6578-416d-806c-652e746f6f6c.json installs into /home/you/.jess/bin:
  hello  runs org.example.tool:hello, holding no capabilities
Install? [y/N] y
package: org.example.tool
version: 1.0.0
bin: /home/you/.jess/bin
commands:
- name: hello
  path: /home/you/.jess/bin/hello
removed: []
$ hello world
hello world from the tool
```

Running it again resolves again, and installs the newer release if there is one.

## The install root

```text
~/.jess/
  bin/                          the commands, one executable per install, the others links to it
  installs/<package id>/
    install.toml                what the install is and where it came from
    app.jsw                     what the executables carry, byte for byte
```

An install from a package id keeps its workspace there too: `jess-workspace.toml`, `jess.lock`, and `.jess/packages`. `install.toml` is written in one canonical form, like `jess.lock`:

```toml
# Written by `jess install`: what this install is, and where it came from.
format = "jess-install"
format_version = 1
package = "org.example.showcase"
version = "0.1.0"
source = "workspace"
jsw_sha256 = "c43ff8e4e26bfb97233e35ed3862ff373069fa64ed0e7e367fced80ca12bc03c"
runtime = "0.1.0"
executable_sha256 = "d6ab42d89fcaa2a286034d4e46c9c652bce96ae5b37c5e7c5ebf087557505ad8"

[[command]]
name = "greet"
target = "org.example.greeter:greet"
kind = "command"
capabilities = ["process.environment"]
```

`source` is `resolved` (a package id, with its `manifest`), `file`, or `workspace`; `runtime` the version of the jess the executables carry. A command name belongs to one install. Another install that wants it, or a file in `bin` no install wrote, is refused until `--force` takes it over, and the install that had it no longer does.

Replacing an executable moves the old one aside first, so installing over a program that's running works on every platform, Windows included; what was moved aside is removed by a later install or uninstall, once nothing is running it.

On macOS, the executables are made on your machine rather than downloaded, so nothing quarantines them, and they are signed ad hoc as packing signs every standalone executable: they run without a Gatekeeper prompt.

## jess install --list

```text
$ jess install --list
- package: org.example.tool
  version: 1.0.0
  source: resolved
  manifest: https://publisher.example/dreamweave/projects/6f72672e-6578-416d-806c-652e746f6f6c.json
  runtime: 0.1.0
  commands:
  - name: hello
    path: /home/you/.jess/bin/hello
    target: org.example.tool:hello
    state: installed
```

A command's `state` is `installed` when its executable is still the one written, `missing` when it's gone, and `changed` when something else is there now. Installing again puts it back.

## jess uninstall

```text
$ jess uninstall showcase
- package: org.example.showcase
  version: 0.1.0
  commands:
  - greet
  - showcase
```

Names an install by its package id or by any of its commands, and removes the whole install: every command and its folder under `installs`.
