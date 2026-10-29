---
title: 'The Manifest'
description: 'Every table and key of jess.toml, what each one means, and the error you get when it is wrong; then jess-workspace.toml and jess.lock.'
weight: 10
extra:
  kind: reference
---

`jess.toml` at the top of a Package Source is the whole description of a package: who it is, what it ships, what it exports, what runs, and what that needs. Jess reads it with no defaults it can't justify and no tolerance for keys it doesn't know. A misspelled key is an error, not a silently ignored one, because a manifest that quietly does nothing is worse than one that refuses.

Every error below starts with `package:` and names the table and key it came from. The texts are quoted as the binary prints them.

The smallest useful manifest, the one the rest of these docs use:

```toml
manifest_version = 1

[package]
id = "com.example.greeter"
version = "1.0.0"
jess_api = "1"
include = ["src", "tests", "tools"]
tests = ["tests/run.jsl"]
default_target = "greet"

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

## Top level

| Key | Required | Is |
| --- | --- | --- |
| `manifest_version` | yes | `1`. Anything else: `jess.toml: manifest_version must be 1`. |
| `[package]` | yes | Identity and payload. Missing: `jess.toml has no [package] table`. |
| `[dependencies]` | no | Package ids to version requirements. |
| `[dev-dependencies]` | no | The same, for tests and `jess package check` only. |
| `[modules]` | no | Module names to the `.jsl` files behind them. |
| `[[layer]]` | no | The semantic systems a host asks for: what each exports and what runs when it is activated. |
| `[managed]` | no | Modules an [application](@/docs/scripting/applications.md) loads into its domain. |
| `[targets]` | no | What `jess package run` can run. |

Any other key at any level is `<where> has an unknown key '<key>'`, with `<where>` naming the table.

## `[package]`

| Key | Required | Is |
| --- | --- | --- |
| `id` | yes | At least two lowercase ASCII words joined by dots: `com.example.greeter`. Reverse-domain is the convention, not a rule. Wrong shape: `package id 'Greeter' is not lowercase ASCII words joined by dots (org.example.name)`. |
| `version` | yes | `major.minor.patch`, optionally followed by `-` and dot-separated pre-release identifiers of ASCII letters, digits, and hyphens: `1.1.0-dev.3`. A numeric identifier has no leading zero. A pre-release sorts below its release, and only an exact requirement ever selects one. Wrong: `package version '1.0' is not a version: major.minor.patch, with -identifiers for a pre-release`. |
| `jess_api` | yes | `"1"`, the package API generation this jess provides. A string, deliberately. Wrong: `jess.toml: jess_api must be "1", the package API this jess provides`. |
| `include` | yes | The payload: folders and files, as paths relative to the manifest, at least one. A folder is walked whole; a file is taken alone. Empty or absent: `jess.toml: [package] include must name the payload's folders or files`. A root that isn't there: `'<location>': [package] include names 'assets', which isn't there`. Naming `jess.toml` itself is an error; it is always included. |
| `tests` | no | Test modules, each a path inside the payload that returns an [`@jess/test`](@/docs/api/test.md) suite. `jess package test` runs them from the source and from the source packed, and checks both providers give the same digest. They ship in the JPK, since they are payload, but tests run from source only. |
| `default_target` | no | The target `jess package run <package>` runs when `--target` isn't given. A lowercase identifier that must name a `[targets]` entry: otherwise `jess.toml: [package] default_target 'greet' does not name a [targets] entry`. |

What goes into the payload is decided here and nowhere else. Under an included folder, every file is payload except an editor's or a merge tool's leftovers (`init.jsl.bak`, `init.jsl~`, `init.jsl.orig`), which would otherwise change the digest and ship. A symlink is refused: `'<location>': '<path>' is a symlink, which a package can't hold`. Two paths that differ only in ASCII case are refused, because one of your users has a filesystem that can't tell them apart. Every path must be canonical: relative, forward slashes, no `.` or `..` components, no empty components, no trailing slash, no backslashes, colons, or control bytes, valid UTF-8. The payload is the same from the folder and from the JPK packed from it, which is the whole point: the [digest](@/docs/api/package.md#the-content-digest) is over exactly these files.

## `[modules]`

A table of module name to the `.jsl` file that implements it:

```toml
[modules]
"@greeter" = "src/init.jsl"
"@greeter/words" = "src/words.jsl"
```

- A name is `@scope` or `@scope/path/...`, each segment lowercase ASCII letters, digits, `.`, `_`, and `-`. Wrong: `jess.toml module 'greeter' is not a module name (@scope/name, lowercase ASCII)`.
- `@jess/` belongs to jess. Only the built-in package may export there: `jess.toml module '@jess/greeter': @jess/ is jess's own namespace, open only to its built-in package (org.dreamweave.jess)`. `@self` is Luau's and refused too.
- The path must be a `.jsl` file in the payload: `jess.toml: module '@greeter' must be a .jsl file`, or, when the file isn't there, `<location>: module '@greeter' is 'src/init.jsl', which isn't in the payload`.
- **Every module must do something.** One that no layer exports and no layer runs is `jess.toml: module '@greeter' is neither exported nor an entrypoint of any layer`. A module nobody can reach is a module nobody meant to ship.

Naming a module here does not make it requirable. A layer has to export it, and a host has to activate that layer. A package's own modules may `require` each other and the exports of the packages its `[dependencies]` name, and nothing else.

## `[[layer]]`

A layer is a named semantic system a host asks for by id. It owns the public API (exports) and the code that runs because it was activated (entrypoints):

```toml
[[layer]]
id = "example.greetings"
requires = ["example.words"]
exports = ["@greeter"]

[[layer.entrypoint]]
module = "@greeter/register"
capabilities = ["filesystem.read"]
```

| Key | Required | Is |
| --- | --- | --- |
| `id` | yes | Dotted lowercase words, unique in the manifest: `jess.toml: layer 'example.greetings' is declared twice`. |
| `requires` | no | Layer ids this one needs activated first, in any package. A layer can't require itself. Resolution finds a provider for each: `example.greetings requires example.words: no provider found for example.words` when none is offered, and `layer example.greetings has more than one candidate: ... ; offer one of them outright` when two are. |
| `exports` | no | Names from `[modules]`. Exporting one `[modules]` doesn't name: `jess.toml: layer 'example.greetings' exports '@greeter', which [modules] doesn't name`. Exports are inert: requiring one starts nothing. |
| `[[layer.entrypoint]]` | no | `module`, a name from `[modules]`, and `capabilities`, what running it needs. Entrypoints run once, in dependency order, when the layer is activated, and never because something required an export. |

Asking for layers and getting an activation plan back is resolution, and it runs no code; `jess package graph` shows the plan. Mounting the plan and running the entrypoints is activation, and that is the host's decision. [Packages](@/docs/scripting/packages.md#layers) has the longer version.

## `[targets.<id>]`

A target is something `jess package run` can run. The id is a lowercase identifier.

```toml
[targets.greet]
kind = "command"
command = "greet"
entry = "tools/greet.jsl"
capabilities = ["process.environment"]

[targets.game]
kind = "application"
entry = "src/main.jsl"
capabilities = ["filesystem.read", "worker.spawn"]
```

| Key | Required | Is |
| --- | --- | --- |
| `kind` | yes | `"command"` or `"application"`. Anything else: `jess.toml [targets.greet] kind must be "application" or "command"`. |
| `entry` | yes | A `.jsl` file in the payload. A **command**'s entry runs as a script, with the package's layers activated, and the run is over when it returns. An **application**'s entry runs in a host of its own, after every selected package's `[managed]` modules have loaded into one domain, and frames run until it calls `jess.application.exit`, `--frames` runs out, or Ctrl+C. Missing file: `<location>: target 'greet' entry is 'tools/run.jsl', which isn't in the payload`. |
| `capabilities` | no | What the target needs, by name. Each must be dotted lowercase words (`jess.toml [targets.greet] capability 'environment' is not dotted lowercase words (vfs.read)`), and at run time each must be one this jess can grant: `unknown capability 'vfs.read'; a jess runtime can be granted ...`. The target holds exactly these and nothing else. The CLI admits certain declared terminal and device capabilities automatically; other names need `--capability` or `--trusted`. See [Capabilities](@/docs/reference/capabilities.md). |
| `command` | no | Command targets only: the name a [workspace](@/docs/cli/workspace.md) runs it by. A lowercase word, unique across the manifest's targets: `jess.toml: command 'greet' is declared by both targets 'a' and 'b'`. On an application target: `jess.toml [targets.game] command is only valid when kind = "command"`. |

## `[managed.<id>]`

The modules an application target loads into its script domain, one table per module:

```toml
[managed.inventory]
module = "src/inventory.jsl"
interface = "game.inventory"
```

| Key | Required | Is |
| --- | --- | --- |
| `module` | yes | A `.jsl` file in the payload: `<location>: managed 'inventory' is 'src/inventory.jsl', which isn't in the payload` when it isn't. It runs once, in dependency order, and may bind `on.host` callbacks and `on.event` handlers. |
| `interface` | no | A dotted id. The table the module returns under it is published to `@jess/interfaces`, and a package later in the load order may override it. |

[Applications](@/docs/scripting/applications.md) explains what a managed module looks like and when it runs.

## `[dependencies]` and `[dev-dependencies]`

Package ids to version requirements:

```toml
[dependencies]
"com.example.words" = "^1.2"

[dev-dependencies]
"com.example.fixtures" = "=0.3.1-dev.4"
```

A requirement is one of:

| Form | Meets |
| --- | --- |
| `^1`, `^1.2`, `^1.2.3`, or the same without the caret | At least that, below the next breaking release, as Cargo reads a caret: `^1.2` is 1.2.0 up to but not 2.0.0; `^0.3` is 0.3.x only; `^0.0.4` is exactly 0.0.4. |
| `=1.2.3` | Exactly that version. |
| `=1.2.3-dev.4` | Exactly that pre-release. No range ever picks a pre-release, so this is the only way to depend on one. |

Anything else: `jess.toml dependency 'com.example.words' 'latest' is not a version requirement (^1.2, =1.2.3, =1.2.3-dev.4)`. A package can't depend on itself (`jess.toml: the package depends on itself`).

A dependency does two things. It is a resolution constraint: a plan that can't meet it fails before anything runs, with `com.example.greeter 1.0.0 requires com.example.words ^1.2` and `resolution failed: com.example.words is not available`. And it is a `require` boundary: a package's modules may require the exports of the packages its `[dependencies]` name and no others, however many are mounted. `[dev-dependencies]` are selected only when the package's own tests run and when `jess package check` type-checks it.

## The workspace file and the lock

A [workspace](@/docs/cli/workspace.md) is a set of Package Sources developed together, named by `jess-workspace.toml`:

```toml
manifest_version = 1

[workspace]
members = ["jpk/builtin", "tools/site"]

[dev-dependencies]
"com.example.site" = "^0.1"
```

| Key | Is |
| --- | --- |
| `manifest_version` | `1`. |
| `[workspace] members` | Folder paths relative to the file, each a Package Source. `"."` names the folder the file is in. Missing: `'<path>' has no [workspace] table`, or `[workspace] needs members, a list of folders`. |
| `[dev-dependencies]` | Package ids to version requirements, as in a manifest: the tools the workspace is developed with. `sync` pins them, `run` runs their command targets by name, and no member depends on them. A workspace of no members may have them alone, as a site built by a tool it doesn't write does. |

`jess workspace sync` writes `jess.lock` beside it, and nothing else should:

```toml
lock_version = 2

[[package]]
id = "com.example.words"
version = "1.2.4"
digest = "778b9e0e53f146dac8b5669ee93edcc692140851bed6cf7e018032c784e9bbc8"
source = "local"
dependencies = []
```

Each `[[package]]` pins one package the members need beyond each other: `id`, an exact `version` (a pre-release spelled out), the content `digest` as `jess package inspect` shows it, where it came from (`source = "local"`, offered from a folder, or `"publisher"`, in which case `manifest` is the URL of the project manifest that publishes it and `artifact_sha256` the SHA-256 of the archive), and the ids of its `dependencies`. The pinned JPKs live under `.jess/packages`. A lock from a newer jess is refused by version (`is lock_version 3, and this jess reads lock_version 2`), and one that no longer matches the members is reported as stale, entry by entry, rather than silently resolved around.

Change a member's dependencies and sync again. Don't edit the lock by hand; the file says so at the top.
