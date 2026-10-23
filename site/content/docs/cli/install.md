---
title: 'jess install and jess uninstall'
description: 'Put an application on your PATH: a package from its publisher, a packed workspace, or a workspace, embedded in this jess as one executable per command.'
weight: 47
extra:
  kind: reference
---

{{ api_signature(value="jess install PACKAGE [--version VERSION] [--resolve] [--publisher URL]... [--index URL]... [--offline] | FILE | --path WORKSPACE [--root DIR] [--force] [--yes] | --list [--root DIR]") }}

{{ api_signature(value="jess uninstall NAME... [--root DIR]") }}

`jess install` does for a jess application what `cargo install` does for a Rust program: it puts it on your PATH under the names its commands declare, so you run `greet` and not `jess workspace run org.example.greeter:greet`. Each command is a [standalone executable](@/docs/cli/workspace.md#standalone-executables): this jess, carrying the application as a [packed workspace](@/docs/cli/workspace.md#jess-workspace-pack). Nothing is compiled, and what it installs doesn't need jess afterwards, not even the jess that installed it.

## What it installs from

| You name | It installs |
| --- | --- |
| a package id, `org.example.tool` | The newest release its publisher offers that isn't a pre-release, or the one `--version` picks: the publisher's own packed workspace when the release carries one, and otherwise the package with what it depends on, resolved and packed here. See [From a publisher](#from-a-publisher). |
| a file, `app.jsw` | That packed workspace, as its bytes are. |
| a file that is a standalone executable | The packed workspace it carries, now carried by this jess instead. |
| `--path WORKSPACE` | The workspace that folder is in, packed as [`jess workspace pack`](@/docs/cli/workspace.md#jess-workspace-pack) packs it: its lock has to be fresh. |

A name that is a file is a file; anything else is a package id.

| Option | Does |
| --- | --- |
| `--version <VERSION>` | For a package id: a whole version (`1.2.0`) is that version, and a [requirement](@/docs/api/package.md#package-satisfies) (`^1.2`) the newest release meeting it. A pre-release is installed only when named exactly. |
| `--resolve` | For a package id: resolve it from its publisher's packages and pack it here, even when a release carries a packed workspace. |
| `--publisher <URL>` | A publisher to take the package from (repeatable): a DreamWeave site's `dreamweave.json`, or one project manifest. |
| `--index <URL>` | A package index that says who publishes an id (repeatable). Without one, AsHyAmS's; `JESS_REGISTRY_URL` names another. |
| `--offline` | Ask no publisher, as `JESS_OFFLINE=1` does. A package id then has nowhere to come from. |
| `--root <DIR>` | The install root. Without it, `JESS_INSTALL_ROOT`, or else `.jess` in your home folder. |
| `--force` | Take over a command name another install has, a file or link no install wrote, or a command of this install's that has changed since it was written, and install again what is installed already. A directory is never taken over. |
| `--yes` | Install without asking. Needed when there is no terminal to ask in. |
| `--list` | List what is installed instead. |

A package id is found exactly as [`jess workspace sync`](@/docs/cli/workspace.md#publishers-and-indexes) finds one: through the one publisher named or indexed, in a runtime that runs no package code.

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
kept: []
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

A DreamWeave site can publish a package's release as an application too: a packed workspace beside the package's JPK (protocol `schema_version` 4, an artifact of format `jsw`). That is what the publisher froze and tested, so it's what `jess install` takes:

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
kept: []
$ hello world
hello world from the tool
```

The download is checked against the manifest's SHA-256, every package inside against the digest the packed workspace pins, and then the packed workspace against what the manifest says of it: the same built-in package, default target, targets, commands, and packages, at the same versions and digests. Anything that doesn't add up is refused, and nothing is installed. Of the releases, it takes the newest one this jess can run: one packed for a newer jess's built-in package is skipped, with a note saying a newer jess would install it.

When no release carries the package as an application, or with `--resolve`, it's resolved here instead, as `sync` resolves one: the package and what it depends on, each download checked against the manifest's SHA-256 and then the content digest, in a workspace of the install's own that keeps the lock and the packages, and then packed. Every target the package declares is installed.

```text
note: no release of org.example.tool carries it as an application; resolving it from its packages
```

Running `jess install` again installs the newer release if there is one. The same release of a publisher's packed workspace, into the same jess, is already installed, and isn't downloaded again; a resolved one is resolved again to find out.

## The install root

```text
~/.jess/
  install.lock                  held by the one jess installing or uninstalling here
  bin/                          the commands, one executable per install, the others links to it
  installs/<package id>/
    install.toml                what the install is and where it came from
    app.jsw                     what the executables carry, byte for byte
```

A resolved install keeps its workspace there too: `jess-workspace.toml`, `jess.lock`, and `.jess/packages`. `install.toml` is written in one canonical form, like `jess.lock`:

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

`source` is `publisher` (a package's own packed workspace, with its `manifest` and the `artifact_sha256` it listed), `resolved` (a package resolved here, with its `manifest`), `file`, or `workspace`; `runtime` the version of the jess the executables carry. A command name belongs to one install. Another install that wants it, a file or link in `bin` no install wrote, or a command of the install's own that has changed since it was written (someone else's now) is refused until `--force` takes it over, and the install that had it no longer does. A directory where a command would go is refused outright, `--force` or not: jess install never replaces or removes one.

One jess at a time installs or uninstalls in a root: it holds `install.lock` from checking what is there, through asking you, to putting the install in place, and another jess that wants the root meanwhile is refused straight away. Every executable and record is written beside the install root first, and then put in place all together or not at all: if any of it fails, what was put in place is taken out again, what was there is put back, and the error says so. That is undoing an error, not surviving a crash: if jess is killed or the machine stops partway, an install can be left half in place, and `jess install --force` of the same thing puts it right. Whatever it replaces is moved aside rather than overwritten, so installing over a program that's running works on every platform, Windows included, and is removed only once everything is in place, or by a later install or uninstall if it was still running.

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

A command's `state` is `installed` when its executable is still the one written, `missing` when it's gone, and `changed` when something else is there now. Installing again with `--force` puts it back.

## jess uninstall

```text
$ jess uninstall showcase
- package: org.example.showcase
  version: 0.1.0
  commands:
  - greet
  - showcase
  kept: []
```

Names an install by its package id or by any of its commands, and removes the whole install, all of it or none: every command still the executable it wrote, and its folder under `installs`. A command that has changed since, or that a link or directory now stands in for, is someone else's: it stays where it is, listed under `kept` with why, and a warning says so.
