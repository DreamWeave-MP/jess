---
title: 'jess workspace'
description: 'Develop several packages together: find the workspace from any folder in it, run and test its targets, pin what it depends on in jess.lock, and pack it into one file to ship.'
weight: 44
extra:
  kind: reference
---

{{ api_signature(value="jess workspace [--workspace FOLDER | FILE] info | targets | graph [TARGET] | run [TARGET] [--capability NAME]... [--trusted] [-- ARGS...] | test [MEMBER]... | sync [ARTIFACT]... [--package-path FOLDER]... [--publisher URL]... [--index URL]... [--locked] [--offline] | pack --output FILE [--runtime JESS]") }}

A **workspace** is a set of [Package Sources](@/docs/scripting/packages.md) developed together: a tool and the library it uses, an application and its editor. It saves you assembling `--package` and `--package-path` by hand for every command, and it makes what the members depend on beyond each other the same on every machine. It is a development arrangement and nothing more. A package's identity, its digest, and what its modules may `require` are its own manifest's business, in a workspace or out of one.

When it's time to hand the thing to someone else, [`pack`](#jess-workspace-pack) freezes it into one file, a **packed workspace**, that runs anywhere jess does, offline, with no workspace, no sync, and no idea what a lock file is. Or into one executable that runs with no jess at all.

Three files make one:

```text
showcase/
  jess-workspace.toml     # names the members
  jess.lock               # pins what they depend on beyond each other
  .jess/packages/         # those packages, as JPKs
  jess.toml               # the root's own package, a member here
  src/
  tools/greeter/
    jess.toml             # another member
    src/
```

Commit `jess-workspace.toml` and `jess.lock`. `.jess/` is regenerable: `sync` fills it.

The examples on this page use the showcase in the repository's `tests/cli/fixtures/workspace`: a root package, `org.example.showcase`, and a tool, `org.example.greeter`, which uses it. Between them they need two packages that aren't members, `org.example.words` and `org.example.style`, and here those come as JPKs.

## The workspace file

```toml
manifest_version = 1

[workspace]
members = [".", "tools/greeter"]
```

Each member is a folder below the workspace's root holding a `jess.toml`, written as folder names joined by `/`; `"."` is the root's own package. There are no globs, no `..`, no absolute paths, and no backslashes. A member reached through a link must land inside the workspace, and must not be another member under a second name. Two members may not differ only in case, since some filesystems can't tell them apart, and may not share a package id. A member may not have a workspace file of its own: workspaces don't nest. Every one of these is an error naming the file and the member, before anything runs.

`jess-workspace.toml` is its own file, not a table in `jess.toml`, so a package's manifest, its digest, and its JPK never carry how it happened to be developed. A root whose file doesn't list `"."` is **virtual**: a folder of members with no package of its own.

The file may also name the tools the workspace is developed with:

```toml
manifest_version = 1

[workspace]
members = []

[dev-dependencies]
"org.dreamweave.mod-template" = "0.3"
```

A workspace's `[dev-dependencies]` are package ids and [version requirements](@/docs/api/package.md#package-satisfies), as a manifest states them. `sync` pins them in `jess.lock` and keeps them in `.jess/packages` like anything a member needs, and `run` runs their targets over the workspace. No member depends on them: they are the workspace's, so no package's manifest or digest says what built it. A workspace may have them and no members at all, as this one does: a folder of content that a tool it doesn't write turns into something.

## Which workspace

Without `--workspace`, each command starts from the current folder, finds the package it is in, and uses the workspace above that lists it. A folder inside a workspace but outside any package belongs to the nearest workspace above it. A package that no workspace lists is a workspace of one, its own root, whatever is above it, and `info` says which workspace file passed it over. Two workspaces listing one package is an error.

`--workspace FOLDER` names the root outright: the folder holding `jess-workspace.toml`, or a package's folder for a workspace of one. `--workspace FILE` names a [packed workspace](#running-a-packed-workspace) instead, a `.jsw` or an executable carrying one.

Opening a workspace reads its file, its members' manifests, and its lock. It runs nothing.

## jess workspace info

The workspace: its root, its kind (`package`, `virtual`, or `implicit` for a workspace of one), each member with its targets and dependencies, and the state of its lock.

```text
$ jess workspace info
root: /home/you/showcase
kind: package
members:
- id: org.example.showcase
  version: 0.1.0
  path: .
  location: /home/you/showcase
  default_target: showcase
  targets:
  - id: showcase
    kind: command
    entry: src/main.jsl
    capabilities: []
    command: null
  dependencies:
  - id: org.example.words
    requirement: '1'
  dev_dependencies: []
  tests: []
- id: org.example.greeter
  ...
dev_dependencies: []
lock:
  file: false
  state: stale
  problems:
  - org.example.showcase 0.1.0 requires org.example.words 1, which jess.lock doesn't pin
  - org.example.greeter 0.1.0 requires org.example.style 1, which jess.lock doesn't pin
  packages: []
note: null
```

## jess workspace targets

Every member's targets, each named the way `run` takes it anywhere in the workspace:

```text
$ jess workspace targets
- target: org.example.showcase:showcase
  kind: command
  command: null
  default: true
  capabilities: []
- target: org.example.greeter:greet
  kind: command
  command: greet
  default: true
  capabilities:
  - process.environment
```

## jess workspace run

| Argument / option | Does |
| --- | --- |
| `[TARGET]` | The target to run, a member's or a pinned package's. Without one, the default target of the member the current folder is in. |
| `--capability <NAME>` | Grant a capability the target declares (repeatable). |
| `--trusted` | Grant everything the target declares. |
| `--frames <N>` | For an application target: stop after N frames, unless it stops itself first. Refused for a command target. |
| `-- ARGS...` | Arguments for the target, which gets them as `...` and in the global `arg` table. |

A target is named `<package id>:<target id>`; by a target id or a `command` name that only one target in the workspace has; or by a package id, for its `default_target`. The members' targets count, and so do the targets of the packages `jess.lock` pins: a tool a workspace depends on runs over that workspace by its command name, with nothing in the workspace wrapping it. That is how this site is built: the repository's workspace pins the generator, a package built on Jess, and runs it by its command name. A name two targets answer to is an error listing both; `targets` lists the members' alone.

The target runs as [`jess package run`](@/docs/cli/package.md#jess-package-run) runs one, command or [application](@/docs/scripting/applications.md): its package resolves over the other members and the packages `jess.lock` pins, each opened only when something needs it, and it runs with exactly the capabilities it declares, once you grant them. Being in a workspace grants nothing, and a member's modules still reach only the packages its own manifest declares. A command target's first returned number is the exit status; an application's is the status it asked to stop with.

```text
$ cd tools/greeter/src
$ jess workspace run --trusted -- you me
HELLO, YOU! (hello)
HELLO, ME! (hello)
$ echo $?
2
$ jess workspace run -- you
error: cannot run target greet

required capability denied:
    process.environment

requested by:
    org.example.greeter 0.1.0
    target greet
```

A lock that isn't what the members need stops the run before anything resolves:

```text
$ jess workspace run greet --trusted -- you
error: workspace: /home/you/showcase/jess.lock is out of date:
  org.example.showcase 0.1.0 requires org.example.words 1, which jess.lock doesn't pin
  org.example.greeter 0.1.0 requires org.example.style 1, which jess.lock doesn't pin
run jess workspace sync
```

The example application in jess's own repository is a workspace of four packages, so it runs this way too:

```text
$ jess workspace --workspace tests/lua/application run org.example.game:game --frames 2 -- print
armor replaces light armor, keeping its weights
entry: heavy helm, weighs 5
director load
inventory startup
director startup: heavy cuirass, weighs 30
director update 1 (number)
director update 2 (number)
inventory drops Rowan's loot
armor dents for Rowan
scoreboard: death 1, Rowan on frame 2
director shutdown
jess: info in org.example.game:game: 1 queued events were dropped at teardown
```

The scoreboard's `gameOver`, emitted during the second frame's drain, never got a third frame to be delivered in, and teardown says so.

## jess workspace graph

Resolves a target, named as for `run`, without running it: the member, the target as its manifest declares it, what it resolved over (the other members and the kept JPKs), the digests the pinned packages are held to, and the [activation plan](@/docs/api/package.md#package-resolve) running it would mount.

```text
$ jess workspace graph greet
package: org.example.greeter
version: 0.1.0
location: /home/you/showcase/tools/greeter
target:
  id: greet
  kind: command
  entry: src/main.jsl
  capabilities:
  - process.environment
  command: greet
candidates:
- /home/you/showcase
- /home/you/showcase/.jess/packages/org.example.style-1.0.0-62007be02fd652547216376101f43846676d99ccc8e4ee30abc989d6bafbe391.jpk
- /home/you/showcase/.jess/packages/org.example.words-1.2.0-d8ca63ad4cbe712ed9142e5550a53df3f37b1460a8ed73f04857a0b65be4363f.jpk
pins:
- id: org.example.style
  digest: 62007be02fd652547216376101f43846676d99ccc8e4ee30abc989d6bafbe391
- id: org.example.words
  digest: d8ca63ad4cbe712ed9142e5550a53df3f37b1460a8ed73f04857a0b65be4363f
plan:
  ...
```

## jess workspace test

Runs the tests of the members named, by package id or path, or of every member that has tests: each as [`jess package test`](@/docs/cli/package.md#jess-package-test) runs one, from its source and then packed, with its `[dependencies]` and `[dev-dependencies]` resolved over the workspace. A member's dev-dependencies are its tests' alone: they never reach a target's plan. The report is a list, one entry per member, and any failure fails the command.

## jess workspace sync

| Argument / option | Does |
| --- | --- |
| `[ARTIFACT]...` | A package offered outright: a Package Source folder or a `.jpk`. For its id, it is the one. |
| `--package-path <FOLDER>` | A folder of packages offered (repeatable): every Package Source folder and `.jpk` directly inside it. |
| `--publisher <URL>` | A publisher (repeatable): a DreamWeave site's `dreamweave.json`, or one project manifest. The authority for the packages its projects publish, and the way to say which publisher you mean when an index knows two. |
| `--index <URL>` | A package index that says who publishes an id (repeatable). Without one, AsHyAmS's `network-data/jess-packages-v1.json`; `JESS_REGISTRY_URL` names another. |
| `--locked` | Fail rather than change `jess.lock`. Packages it already pins may still be put in `.jess/packages`, from their publishers or from what is offered. |
| `--offline` | Ask no publisher: only what is offered and kept. `JESS_OFFLINE=1` says the same. |

Pins what the members depend on beyond each other, transitively, dev-dependencies included, and the workspace file's own `[dev-dependencies]`, and keeps each of those packages as a JPK in `.jess/packages`. A Package Source offered is packed there and a JPK copied there, each checked against the digest it was chosen by; an archive a publisher serves is downloaded there, checked against the SHA-256 its manifest declares and then, opened, against the content digest, and kept only when both hold. `jess.lock` is written last, so an interrupted sync never leaves a lock pointing at half a file.

```text
$ jess workspace sync ../words.jpk ../style.jpk
lock: written
packages:
- id: org.example.style
  version: 1.0.0
  digest: 62007be02fd652547216376101f43846676d99ccc8e4ee30abc989d6bafbe391
  installed: true
- id: org.example.words
  version: 1.2.0
  digest: d8ca63ad4cbe712ed9142e5550a53df3f37b1460a8ed73f04857a0b65be4363f
  installed: true
```

Run it again with nothing offered and it changes nothing, and says so: `lock: unchanged`, `installed: false`. From then on the JPKs it was handed can go; the workspace runs from what it kept.

How a package is chosen for each id:

1. One version per id, across the whole workspace, which every requirement on it must accept: the members', and those of the packages chosen for them. That is the resolver's own rule, one package per id in a plan.
2. A member is always the member. A requirement a member doesn't meet is an error naming both, never a reason to reach for a JPK.
3. A package named outright is the one for its id.
4. Otherwise, what `jess.lock` already pins, as long as it still meets every requirement and is kept or offered. A newer version turning up on a package path moves nothing.
5. Otherwise, the one package offered that meets every requirement. Two that would each do are an error naming both; name the one you mean.
6. Otherwise, unless `--offline`, the newest available version from the one publisher that lists the id: a site or manifest named with `--publisher`, or the manifest an index says publishes it. An index that knows two publishers is an error naming both manifests; `--publisher` picks one. Nothing chooses a publisher for you.

Ids are settled in order, so nothing depends on how a folder happens to list, and settled again whenever a package chosen later requires something an earlier choice doesn't meet: a pinned `words` 1.2.0 gives way to an offered 1.3.0 as soon as a new `style` asks for `^1.3`. A choice that still meets everything stays put. That is the whole search: no trying every combination, and no "pick the highest". When no one version can satisfy everyone, the error says who wanted what.

With the showcase pinned to exactly `words` 1.2.0, and a `style` that wants at least 1.3:

```text
$ jess workspace sync --package-path ../vendor
error: workspace: nothing offered for org.example.words is one the workspace can use; offered, but not meeting that: 1.2.0 (source ../vendor/words-1.2.0), 1.3.0 (source ../vendor/words-1.3.0)
  org.example.showcase 0.1.0 requires org.example.words =1.2.0
  org.example.style 1.0.0 requires org.example.words ^1.3
offer one with --package-path or by file
```

Nothing is fetched while something offered or kept will do: a workspace whose packages are all handed to it by file never touches the network, and neither does one whose lock pins what `.jess/packages` holds. The network is reached only for an id nothing local satisfies, and `--offline` forbids even that.

## Publishers and indexes

A **publisher** is a DreamWeave site, and a Jess package on it is a project like any other: one repository whose releases are the package's versions. The project's **manifest** (protocol `schema_version` 3) lists each release with a `jpk` artifact: the package's content digest, the archive's SHA-256, and where the archive downloads from. The manifest is the authority for what it lists; the site's `dreamweave.json` says which project publishes which package. An **index** is somebody's listing of which sites publish which ids, such as the one AsHyAmS publishes at `network-data/jess-packages-v1.json`, which is the default; sync reads it for one thing, who to ask, and believes nothing else in it. Both are documented on the [AsHyAmS site](https://dreamweave-mp.github.io/AsHyAmS/about/packages/).

A package from a publisher is pinned with its publisher:

```toml
[[package]]
id = "org.dreamweave.site"
version = "0.1.0"
digest = "778b9e0e53f146dac8b5669ee93edcc692140851bed6cf7e018032c784e9bbc8"
source = "publisher"
manifest = "https://publisher.example/dreamweave/projects/7f3c1a2e-5b6d-4e8f-9a0b-1c2d3e4f5a6b.json"
artifact_sha256 = "9fa73580fbe00766413be59765a5f8bd8d18ec0af87005c86975b31adea78742"
dependencies = []
```

`digest` is what every lock pins, the package's content; `artifact_sha256` is the archive file the publisher served. A restore (`sync --locked` on a clean checkout) reads that manifest, finds the pinned version, and refuses to download if the manifest now describes different bytes for it: a publisher whose story changed is a reason to stop, not to accept. A file offered with the same content restores the package too, offline, and the lock keeps saying where it was published. Nothing ever moves a pin to another publisher because an index says the package moved; sync without `--locked` chooses again, and says so in the lock.

Fetching is the host's own doing, in a runtime that runs no package code and holds `network.dns.resolve` and `network.tcp.connect` for the sync alone. No package gains a network capability because the workspace fetched it.

## jess.lock

```toml
# Written by `jess workspace sync`: what the members depend on beyond each other, pinned.
# Change a member's dependencies and sync again rather than editing this.
lock_version = 2

[[package]]
id = "org.example.style"
version = "1.0.0"
digest = "62007be02fd652547216376101f43846676d99ccc8e4ee30abc989d6bafbe391"
source = "local"
dependencies = ["org.example.words"]

[[package]]
id = "org.example.words"
version = "1.2.0"
digest = "d8ca63ad4cbe712ed9142e5550a53df3f37b1460a8ed73f04857a0b65be4363f"
source = "local"
dependencies = []
```

One canonical form: sorted, no timestamps, no paths. The same pins write the same bytes on every machine, and a workspace moved or copied somewhere else is the same workspace. A `lock_version = 1` file, which pinned local packages only, still reads; the next sync that changes it writes version 2. Members aren't in it: they are loose source you edit, and pinning every edit would be a lock that changes every time you save. The requirements aren't in it either, since those are the manifests'; the lock records what was chosen for them.

Each pinned package is kept as `.jess/packages/<id>-<version>-<digest>.jpk`, so two builds of one version are two files, and the lock says which one runs. `sync` writes only inside the workspace: a `.jess` or `.jess/packages` that links somewhere outside it is refused before anything is written.

Every command checks the lock against the members' manifests:

| State | Means | What runs |
| --- | --- | --- |
| `fresh` | It pins exactly what the members need. A workspace that needs nothing beyond its members is fresh without a lock. | Everything. |
| `stale` | A member requires something it doesn't pin, or a version it doesn't accept; it pins a package that is now a member, or that nothing requires. | Nothing, until `sync`. |
| `missing` | It is right, but `.jess/packages` lacks a package it pins, or holds something else under its name. | Nothing, until `sync` is offered that package. |

A kept JPK that isn't the package pinned, even one with the right id and version, is refused by digest before any of its code runs, and `info`, which opens every kept package and checks its digest, reports it as `missing`. Runs leave that check to resolution, which makes it for each package it opens, so a run doesn't hash packages it never touches. A lock this jess can't read, a `lock_version` it doesn't know, or a pin whose id could name a file outside `.jess/packages` is an error, and the workspace doesn't open.

## jess workspace pack

| Option | Does |
| --- | --- |
| `--output <FILE>` | Where to write the packed workspace. |
| `--runtime <JESS>` | Write a [standalone executable](#standalone-executables) instead: this jess executable carrying the packed workspace. |

Freezes the workspace into a **packed workspace**, a `.jsw`: every target it exports, and every package those targets run, as JPKs, each pinned to its content digest. It resolves each target exactly as `run` would (every layer of its package, over the other members and the packages the lock pins, its entrypoints admitted against what it declares), and keeps what those plans select and nothing else. A member's tests, its dev-dependencies, and the tools the workspace file's `[dev-dependencies]` name stay behind, unless a target the workspace exports needs them.

```text
$ jess workspace pack --output ../showcase.jsw
path: ../showcase.jsw
size: 4174
sha256: c43ff8e4e26bfb97233e35ed3862ff373069fa64ed0e7e367fced80ca12bc03c
default: org.example.showcase:showcase
targets:
- org.example.greeter:greet
- org.example.showcase:showcase
packages:
- id: org.example.greeter
  version: 0.1.0
  digest: abf45955bdd3d0855b77d9ee40e90f0bfdaad1d559f08013f214b8899b55cff0
- id: org.example.showcase
  version: 0.1.0
  digest: a5500ef51167c7a3977b4e87ee6280ae83926548fc83d3e8f7a0ccc4a5fc27b5
- id: org.example.style
  version: 1.0.0
  digest: 62007be02fd652547216376101f43846676d99ccc8e4ee30abc989d6bafbe391
- id: org.example.words
  version: 1.2.0
  digest: d8ca63ad4cbe712ed9142e5550a53df3f37b1460a8ed73f04857a0b65be4363f
```

Packing never reaches past the workspace. The lock has to be fresh, as it does for `run`, and nothing is fetched: a pack that needs the network is a pack that needs `sync` first. The same workspace packs to the same bytes, on every machine, every time, so the SHA-256 it reports is worth printing beside the download.

Which targets it exports, and which one runs when nobody names one, is the workspace file's to say:

```toml
[pack]
targets = ["greet", "org.example.showcase:showcase"]
default = "greet"
```

Both take target names as `run` does. Without `targets`, every member's targets are exported; a pinned package's targets (the tools a workspace is developed with) only when named. Without `default`, the default is the root member's `default_target`, or the one target when there is only one; a packed workspace with neither has no default, and every run names its target.

## Running a packed workspace

A packed workspace is a workspace, so the workspace commands take it where they take a folder:

```text
$ jess workspace --workspace showcase.jsw targets
- target: org.example.greeter:greet
  kind: command
  command: greet
  default: false
  capabilities:
  - process.environment
- target: org.example.showcase:showcase
  kind: command
  command: null
  default: true
  capabilities: []
$ jess workspace --workspace showcase.jsw run greet --trusted -- you me
HELLO, YOU! (hello)
HELLO, ME! (hello)
$ echo $?
2
```

`info`, `targets`, `graph`, and `run` work on it, naming targets as they always do, from any folder. `sync`, `test`, and `pack` refuse: it's sealed, and they need the workspace it was packed from.

```text
$ jess workspace --workspace showcase.jsw sync
error: 'showcase.jsw' is a packed workspace, which is sealed: syncing needs the workspace it was packed from
```

To run its default target as a program, name the file the way you'd name a script:

```text
$ jess showcase.jsw
hello, showcase
```

Every argument after the file is the target's. Run as a program, the target holds what it declares, the way `jess script.jsl` holds what a script needs: you chose to run it. It still holds nothing it doesn't declare, and `targets` says what that is before you run anything. A declaration is a ceiling, not a character reference: running a packed workspace someone sent you is trusting them, as running any program they sent you is. `workspace run` keeps its explicit grants, for when you'd rather say yes capability by capability.

A run resolves over the packed workspace and nothing else: no package path, no `.jess/packages`, no files beside it, no network. Every package is opened in place, inside the one file, and nothing is ever extracted. Each is held to the digest the packed workspace pins, so one whose bytes aren't the ones packed is refused, and so is anything the index doesn't list or lists but doesn't hold, before a line of package code runs. A packed workspace records the version of jess's built-in package it was packed against; a jess whose built-in package isn't compatible says so and runs nothing.

## Standalone executables

`--runtime` writes the packed workspace into a jess executable, which then *is* the application:

```text
$ jess workspace pack --output ../app --runtime "$(command -v jess)"
$ ./app
hello, showcase
$ cp app greet && ./greet you
HELLO, YOU! (hello)
```

It runs its default target with every argument it's given, and claims none of them for itself. Named for a target's `command`, or run through a link by that name, it runs that target instead, so one payload can ship as several commands. It needs nothing installed: no jess, no compiler, no network. It doesn't change the folder it runs in or write beside itself, and it runs from wherever it's moved to. `jess workspace --workspace app info` reads what it carries.

The runtime can be any jess executable, for any platform, whatever you're packing on: the release archive for another platform will do, so one machine builds every platform's executable without a compiler for any of them. What the executable carries is the packed workspace, byte for byte, so it reports the same SHA-256 as the `.jsw`. A runtime that already carries a packed workspace, or predates standalone executables, is refused.

On Linux and Windows the packed workspace follows the runtime, which is unchanged. Sign a Windows executable afterwards, if you sign at all; Authenticode then covers the packed workspace along with the rest of the file. Bytes added after the signature are outside it, so a signed executable with anything after its signature is refused rather than read, and a runtime that's already signed is refused: sign once, last. `strip` drops what a Linux executable carries, leaving plain jess: strip the runtime before packing, never after.

On macOS, code signing seals the file, and nothing appended after the seal can be signed again, so a macOS jess reserves a section for a packed workspace, `__JESS,__jsw`, and packing grows it. The result is signed ad hoc, as the linker signs jess on Apple silicon (an Intel runtime, which the linker leaves unsigned, stays unsigned), so it runs as it is. It passes `codesign --verify --strict`, and `codesign` signs it again, with your Developer ID and the hardened runtime, for notarizing. Apple documents the `com.apple.security.cs.allow-jit` entitlement as what lets a hardened program generate code, so sign with it: `--!native` code runs natively under the hardened runtime with it. A universal runtime is refused: pack one per architecture. Downloaded through a browser, an executable that isn't notarized is quarantined, and Gatekeeper refuses to open it until it is notarized or you let it through.

## The packed workspace format

A `.jsw` is written by [`@jess/jpk`](@/docs/api/jpk.md)'s canonical writer, so any unzip tool lists it, and it holds two kinds of entry, both stored:

```text
jsw.toml                                   the index
packages/<id>-<version>.jpk                one JPK per package, byte for byte
```

Each JPK is stored rather than compressed again: its own entries are already compressed, and a stored one is a contiguous run of the file, which opens in place. The index says what the packed workspace runs:

```toml
# Written by `jess workspace pack`: what this packed workspace runs, pinned.
format = "jess-packed-workspace"
format_version = 1
builtin = "0.1.0"
default = "org.example.showcase:showcase"

[[target]]
package = "org.example.greeter"
id = "greet"

[[target]]
package = "org.example.showcase"
id = "showcase"

# ...one [[package]] for each package it holds, this the last of four:
[[package]]
id = "org.example.words"
version = "1.2.0"
digest = "d8ca63ad4cbe712ed9142e5550a53df3f37b1460a8ed73f04857a0b65be4363f"
```

One canonical form, as `jess.lock` has: sorted, no paths, no timestamps. It doesn't restate what the packages' manifests say (a target's kind, entry, capabilities, command), so there is one authority for each fact. Opening one reads its directory and each package's manifest, under the same budgets and path rules as a JPK, and runs nothing.
