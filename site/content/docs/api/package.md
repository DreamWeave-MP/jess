---
title: '@jess/package'
description: 'Packages from a Package Source or a JPK: manifests, content digests, packing, and code-free resolution of layers into an activation plan.'
weight: 115
extra:
  kind: api
---

{{ api_signature(value="require('@jess/package'): { open, openJpk, peek, candidates, pack, resolve, manifest, satisfies, requirement }") }}

A [built-in module](@/docs/scripting/built-in-modules.md), typed as `Module__jess_package`. It is the package model [Packages](@/docs/scripting/packages.md) describes, as functions: open a package from either of its forms, pack one, and resolve layers over several into a plan. The container underneath is [`@jess/jpk`](@/docs/api/jpk.md).

Nothing here runs package code. Opening reads `jess.toml` and the payload's bytes, and resolving reads manifests. A package's modules run when something `require`s them, after a host has activated a plan, which this module never does.

## package.open

{{ api_signature(value="package.open(location: string) -> JessPackage") }}

Opens the package at `location`: a Package Source folder (the folder holding `jess.toml`), or a `.jpk` file. It needs `filesystem.read`. Either way the result is the same shape:

| Field | Is |
| --- | --- |
| `manifest` | The checked [manifest](@/docs/scripting/packages.md#the-manifest): `manifestVersion`, `id`, `version`, `jessApi`, `defaultTarget`, `include`, `tests`, `dependencies`, `devDependencies`, `modules`, `layers`, `managed`, and `targets`. |
| `id`, `version` | The manifest's. |
| `digest` | The [content digest](#the-content-digest), hex. |
| `kind` | `'source'` or `'jpk'`. Ordinary code has no business branching on it. |
| `location` | Where it was opened from. |
| `files` | The payload, in canonical order: `jess.toml` first, then every other path bytewise. |
| `read(path)` | A payload file's bytes, or nil for a path that isn't in the payload. |

Opening checks the whole package before it returns. A Package Source must not hold symlinks, files that are neither file nor folder, or two paths that differ only in case (`Name.jsl` and `name.jsl` are one file on Windows and a fresh bug everywhere else). A JPK must hold nothing outside its manifest's `include`. Every module the manifest exports must be in the payload, and a Package Source must hold every test its manifest names.

The returned package, its file list, and its manifest are frozen. `read` returns a buffer for a named payload file and checks its content against the digest recorded at open; if the file disappears or changes, it raises an error asking the caller to open the package again. For a source package, the manifest bytes are retained from the opening read rather than reread from disk. An absent payload path returns `nil`.

```luau
--!strict
local package = require '@jess/package'
local greeter = package.open 'greeter'
print(greeter.id, greeter.version, greeter.kind)
print(greeter.digest)

for _, path in greeter.files do
  print(path)
end
```

```text
com.example.greeter	1.0.0	source
6fdd1a56f49cf9b23f2f9765b0cfce84836ba001cf57a602aa5ea47f7768e35b
jess.toml
src/init.jsl
src/words.jsl
tests/run.jsl
```

## package.openJpk

{{ api_signature(value="package.openJpk(source: JessJpkSource, label: string) -> JessPackage") }}

Opens a JPK that isn't a file of its own: one in memory, or stored inside another archive, from `source`, `{ size, read }`, the same [source](@/docs/api/jpk.md#jpk-open) `@jess/jpk` reads. It is the same package as the file would be, with the same digest and the same checks, `label` standing in for where it came from in `location` and in every message. Reading an in-memory source does not itself need a filesystem grant; a source callback that reads a file needs the authority for that read. A packed workspace opens its packages this way, in place.

```luau
--!strict
local package, fs = require '@jess/package', require '@dream/fs'
local bytes, problem = fs.readFile 'words.jpk'
assert(bytes, problem)

local held = package.openJpk({
  size = buffer.len(bytes),
  read = function(offset: number, length: number): buffer
    local slice = buffer.create(length)
    buffer.copy(slice, 0, bytes, offset, length)
    return slice
  end,
}, 'words.jpk, in memory')
print(held.id, held.kind, held.location)
print(held.digest == package.open('words.jpk').digest)
```

```text
org.example.words	jpk	words.jpk, in memory
true
```

## package.peek

{{ api_signature(value="package.peek(location: string) -> JessPackageOffer") }}

Reads and checks the manifest at a source folder or JPK without opening the package's complete payload. It requires `filesystem.read`. The frozen offer contains `manifest`, `id`, `version`, `kind` (`'source'` or `'jpk'`), and `location`. A missing or invalid manifest raises. For a JPK, the archive directory and manifest entry must be read; this is not validation of every payload file. Nothing executes.

## package.candidates

{{ api_signature(value="package.candidates(folder: string) -> { JessPackageOffer }") }}

Reads package offers directly inside `folder`, in name order: directories containing `jess.toml` and files ending in `.jpk`. It requires `filesystem.read` and does not recurse. A folder that is not a package path, or an unreadable or invalid candidate, raises rather than silently omitting that candidate. Nothing executes.

## package.pack

{{ api_signature(value="package.pack(source: string, output: string, options: { version: string? }?) -> JessPackResult") }}

Packs the Package Source at `source` into the [canonical JPK](@/docs/api/jpk.md#the-canonical-writer) at `output`, written through a temporary file and a rename. It needs `filesystem.read` and `filesystem.write`. Returns `{ path, version, digest, files, size }`. Only the payload goes in: `jess.toml` and what `include` names, which holds the tests the manifest names. Benchmarks, `.git`, and an editor's or a merge tool's leftovers (`*.bak*`, `*~`, `*.orig`) stay home. [`jess pack`](@/docs/cli/package.md#jess-pack) is this function.

With `options.version`, the payload is packed at that version: the one `version` line of `jess.toml`'s `[package]` table is revised, every other byte stays, and the manifest, the id, the digest, and the files all read as revised. That is how a development build is made: the same files as the source, at `1.2.1-dev.57`, with a digest of its own, because its manifest differs. The source on disk is untouched.

## package.resolve

{{ api_signature(value="package.resolve(packages: { JessPackage }, request: JessPackageRequest) -> (JessActivationPlan, { JessPackage })") }}

Resolves `request.layers` over `packages` into an activation plan, deterministically, or fails naming exactly what's wrong. The second return is the selected package objects, in the plan's package order. Every rule is checked before a plan exists:

- one provider per package id: the same package offered twice is an error, even as its source and its JPK (`com.example.greeter is offered twice (...); choose one`);
- one provider per layer id;
- every requested layer, and every layer they require, has a provider (`example.tools requires example.core: no provider found for example.core`);
- no layer cycles (`layer cycle: cycle.a -> cycle.b -> cycle.a`);
- every package dependency of a selected package is offered and satisfies its requirement, with the chain that asked for it;
- no module name is exported by two selected packages, or by a package and the host (`request.hostModules`): `@dream/fs` is L3i's, and a package exporting it is an error (`<id> exports '@dream/fs', which the host already provides`), not a shadow;
- with `request.grant`, every capability the resolved layers' entrypoints require is in it; otherwise the plan fails before any code runs:

```text
cannot activate example.tools

required capability denied:
    filesystem.read

requested by:
    org.example.alpha 1.2.3
    layer example.tools
    entrypoint @alpha/tools/register
```

Besides the packages handed over, a request may offer `candidates`, packages read only as far as their manifests and opened only when a layer or a dependency needs one, never in place of a package handed over and never chosen between. A candidate is opened with `package.open` by its location, or by `request.open` when the request names one: a resolution whose candidates aren't files of their own, such as a packed workspace's, names what opens them, and the rules are the same either way. `request.pins` holds packages with given ids to given digests.

`request.roots` selects package ids and their dependencies even when no requested layer belongs to them. `request.dev` names selected package ids whose development dependencies should also be resolved; listing an id in `dev` does not itself select it. Development dependencies are for the host's tests and do not grant the package's own modules permission to require them. An id in `roots` that cannot be offered or opened is an error.

The plan is `{ requested, layers, packages, modules, entrypoints, capabilities }`:

- `layers`, every resolved layer, each after the layers it requires, with its provider package and version, its exports, and its entrypoints;
- `packages`, the providers and their package dependencies, with each one's digest, kind, and location;
- `modules`, the module namespace: every export of every layer of the selected packages, with its package and path. Exports are inert, so a dependency's whole public API is resolvable without activating any of its layers;
- `entrypoints`, the resolved layers' entrypoints in the order activation runs them, with their package, path, and layer;
- `capabilities`, every capability those entrypoints require, with the entrypoint, layer, and package asking.

`jess package graph` prints this table. Activating it is the host's job: `jess run --package` does it, and a Rust host calls `jess::lua::package::activate`. Activation mounts the plan, so `require` reaches the namespace, then runs the entrypoints, each once, in order. An entrypoint that fails stops the activation with its error; what ran before it stays done, because activation isn't a transaction.

## package.manifest

{{ api_signature(value="package.manifest(text: string) -> JessPackageManifest") }}

Parses and checks a `jess.toml`, with the same errors `open` gives. An unknown key is an error, not a field quietly ignored.

## package.satisfies

{{ api_signature(value="package.satisfies(version: string, requirement: string) -> boolean") }}

Whether `version` meets `requirement`. Requirements are Cargo's caret ranges and exact pins: `^1.2` (and `1.2`) is at least 1.2.0 and below 2.0.0, `^0.2` stays below 0.3.0, `^0.0.3` is exactly 0.0.3, and `=1.2.3` is exactly that. A version is `major.minor.patch`, optionally with a pre-release after a hyphen: dot-separated identifiers of ASCII letters, digits and hyphens, numeric ones without a leading zero, as `1.2.1-dev.57`. A pre-release precedes its release, and pre-releases order identifier by identifier, numbers numerically and below words, words bytewise, a shorter list first. A pre-release meets only the exact requirement that names it, `=1.2.1-dev.57`: no range ever picks one, so a development build is used by whoever asks for it and nobody else.

## package.requirement

{{ api_signature(value="package.requirement(requirement: unknown, what: string) -> string") }}

`requirement` when it is a version requirement as `satisfies` reads one, and otherwise an error that starts with `what`: the same check a manifest's `[dependencies]` get, for a file that states requirements of its own, as `jess-workspace.toml` does.

## The content digest

A package's identity is its id and version. Its content is the digest: BLAKE3 over the files, with lengths and paths framing the input so different file boundaries cannot be confused. As with any fixed-size cryptographic digest, collisions are possible.

1. The domain tag `jess-package-digest-v1` and a NUL byte.
2. For every payload file in canonical order (`jess.toml` first, then the rest bytewise): the path's length as a little-endian u64, the path, the file's size as a little-endian u64, and the file's own BLAKE3.

The ZIP container is nowhere in it. A Package Source and the JPK packed from it have the same digest, however the JPK was compressed, which is the point: same digest, same package. Edit one byte of the source and the digest changes, though the version doesn't. A version string is a promise. The digest is evidence.
