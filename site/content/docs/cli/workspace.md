---
title: 'jess workspace'
description: 'Develop several packages together: find the workspace from any folder in it, run and test its targets, and pin what it depends on in jess.lock.'
weight: 44
extra:
  kind: reference
---

{{ api_signature(value="jess workspace [--workspace FOLDER] info | targets | graph [TARGET] | run [TARGET] [--capability NAME]... [--trusted] [-- ARGS...] | test [MEMBER]... | sync [ARTIFACT]... [--package-path FOLDER]... [--feed URL]... [--index URL]... [--locked] [--offline]") }}

A **workspace** is a set of [Package Sources](@/docs/scripting/packages.md) developed together: a tool and the library it uses, an application and its editor. It saves you assembling `--package` and `--package-path` by hand for every command, and it makes what the members depend on beyond each other the same on every machine. It is a development arrangement and nothing more. A package's identity, its digest, and what its modules may `require` are its own manifest's business, in a workspace or out of one.

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

## Which workspace

Without `--workspace`, each command starts from the current folder, finds the package it is in, and uses the workspace above that lists it. A folder inside a workspace but outside any package belongs to the nearest workspace above it. A package that no workspace lists is a workspace of one, its own root, whatever is above it, and `info` says which workspace file passed it over. Two workspaces listing one package is an error.

`--workspace FOLDER` names the root outright: the folder holding `jess-workspace.toml`, or a package's folder for a workspace of one.

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

A target is named `<package id>:<target id>`; by a target id or a `command` name that only one target in the workspace has; or by a package id, for its `default_target`. The members' targets count, and so do the targets of the packages `jess.lock` pins: a tool a workspace depends on runs over that workspace by its command name, with nothing in the workspace wrapping it. That is how a DreamWeave site runs the Mod Template it pins: `jess workspace run --trusted dreamweave-build`. A name two targets answer to is an error listing both; `targets` lists the members' alone.

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
inventory drops Caius's loot
armor dents for Caius
scoreboard: death 1, Caius on frame 2
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
| `--feed <URL>` | A publisher's `jess-packages.json` (repeatable): the authority for the packages it lists, and the way to say which publisher you mean when an index knows two. |
| `--index <URL>` | A package index that says who publishes an id (repeatable). Without one, AsHyAmS's `network-data/jess-packages-v1.json`; `JESS_REGISTRY_URL` names another. |
| `--locked` | Fail rather than change `jess.lock`. Packages it already pins may still be put in `.jess/packages`, from their publishers or from what is offered. |
| `--offline` | Ask no publisher: only what is offered and kept. `JESS_OFFLINE=1` says the same. |

Pins what the members depend on beyond each other, transitively, dev-dependencies included, and keeps each of those packages as a JPK in `.jess/packages`. A Package Source offered is packed there and a JPK copied there, each checked against the digest it was chosen by; an archive a publisher serves is downloaded there, checked against the SHA-256 its feed declares and then, opened, against the content digest, and kept only when both hold. `jess.lock` is written last, so an interrupted sync never leaves a lock pointing at half a file.

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
6. Otherwise, unless `--offline`, the newest available version from the one publisher that lists the id: a feed named with `--feed`, or the feed an index says publishes it. An index that knows two publishers is an error naming both feeds; `--feed` picks one. Nothing chooses a publisher for you.

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

## Publishers, feeds and indexes

A **feed** is a publisher's `jess-packages.json`, a static file a site serves beside its DreamWeave `dreamweave.json`: every package it distributes, every version of each with the package's content digest and the archive's SHA-256, and where the archive downloads from. The feed is the authority for what it lists. An **index** is somebody's listing of which sites publish which ids, such as AsHyAmS's `network-data/jess-packages-v1.json`; sync reads it for one thing, who to ask, and believes nothing else in it. Both formats are documented on the [AsHyAmS site](https://dreamweave-mp.github.io/AsHyAmS/about/packages/).

A package from a publisher is pinned with its publisher:

```toml
[[package]]
id = "org.dreamweave.site"
version = "0.1.0"
digest = "778b9e0e53f146dac8b5669ee93edcc692140851bed6cf7e018032c784e9bbc8"
source = "feed"
feed = "https://publisher.example/jess-packages.json"
artifact_sha256 = "9fa73580fbe00766413be59765a5f8bd8d18ec0af87005c86975b31adea78742"
dependencies = []
```

`digest` is what every lock pins, the package's content; `artifact_sha256` is the archive file the publisher served. A restore (`sync --locked` on a clean checkout) reads that publisher's feed, finds the pinned version, and refuses to download if the feed now describes different bytes for it: a publisher whose story changed is a reason to stop, not to accept. A file offered with the same content restores the package too, offline, and the lock keeps saying where it was published. Nothing ever moves a pin to another publisher because an index says the package moved; sync without `--locked` chooses again, and says so in the lock.

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
