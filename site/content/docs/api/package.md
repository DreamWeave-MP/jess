---
title: '@jess/package'
description: 'Packages from a Package Source or a JPK: manifests, content digests, packing, and code-free resolution of layers into an activation plan.'
weight: 116
extra:
  kind: api
---

{{ api_signature(value="require('@jess/package'): { open, pack, resolve, manifest, satisfies }") }}

A [built-in module](@/docs/scripting/built-in-modules.md), typed as `Module__jess_package`. It is the package model [Packages](@/docs/scripting/packages.md) describes, as functions: open a package from either of its forms, pack one, and resolve layers over several into a plan. The container underneath is [`@jess/jpk`](@/docs/api/jpk.md).

Nothing here runs package code. Opening reads `jess.toml` and the payload's bytes, and resolving reads manifests. A package's modules run when something `require`s them, after a host has activated a plan, which this module never does.

## package.open

{{ api_signature(value="package.open(location: string) -> JessPackage") }}

Opens the package at `location`: a Package Source folder (the folder holding `jess.toml`), or a `.jpk` file. Either way the result is the same shape:

| Field | Is |
| --- | --- |
| `manifest` | The checked [manifest](@/docs/scripting/packages.md#the-manifest), as a table: `id`, `version`, `jessApi`, `include`, `tests`, `dependencies`, `modules`, `layers`. |
| `id`, `version` | The manifest's. |
| `digest` | The [content digest](#the-content-digest), hex. |
| `kind` | `'source'` or `'jpk'`. Ordinary code has no business branching on it. |
| `location` | Where it was opened from. |
| `files` | The payload, in canonical order: `jess.toml` first, then every other path bytewise. |
| `read(path)` | A payload file's bytes, or nil for a path that isn't in the payload. |

Opening checks the whole package before it returns. A Package Source must not hold symlinks, files that are neither file nor folder, or two paths that differ only in case (`Name.jsl` and `name.jsl` are one file on Windows and a fresh bug everywhere else). A JPK must hold nothing outside its manifest's `include`. Every module the manifest exports must be in the payload.

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
dab090658ab19adbb8fc0f6e125269cde47027ce918173b1bd9c9fad313c5971
jess.toml
src/init.jsl
src/words.jsl
```

## package.pack

{{ api_signature(value="package.pack(source: string, output: string) -> JessPackResult") }}

Packs the Package Source at `source` into the [canonical JPK](@/docs/api/jpk.md#the-canonical-writer) at `output`, written through a temporary file and a rename. Returns `{ path, digest, files, size }`. Only the payload goes in: `jess.toml` and what `include` names. Tests, goldens, editor droppings, and `.git` stay home. [`jess pack`](@/docs/cli/package.md#jess-pack) is this function.

## package.resolve

{{ api_signature(value="package.resolve(packages: { JessPackage }, request: JessPackageRequest) -> JessActivationPlan") }}

Resolves `request.layers` over `packages` into an activation plan, deterministically, or fails naming exactly what's wrong. Every rule is checked before a plan exists:

- one provider per package id: the same package offered twice is an error, even as its source and its JPK (`com.example.greeter is offered twice (...); choose one`);
- one provider per layer id;
- every requested layer, and every layer they require, has a provider (`example.tools requires example.core: no provider found for example.core`);
- no layer cycles (`layer cycle: cycle.a -> cycle.b -> cycle.a`);
- every package dependency of a selected package is offered and satisfies its requirement, with the chain that asked for it;
- no module name is exported by two selected packages, or by a package and the host (`request.hostModules`): `@dream/vfs` belongs to vfstool, and a package exporting it is an error, not a shadow;
- with `request.grant`, every capability the resolved layers' entrypoints require is in it; otherwise the plan fails before any code runs:

```text
cannot activate example.tools

required capability denied:
    vfs.read

requested by:
    org.example.alpha 1.2.3
    layer example.tools
    entrypoint @alpha/tools/register
```

The plan is `{ requested, layers, packages, modules, entrypoints, capabilities }`:

- `layers`, every resolved layer, each after the layers it requires, with its provider package and version, its exports, and its entrypoints;
- `packages`, the providers and their package dependencies, with each one's digest, kind, and location;
- `modules`, the module namespace: every export of every layer of the selected packages, with its package and path. Exports are inert, so a dependency's whole public API is resolvable without activating any of its layers;
- `entrypoints`, the resolved layers' entrypoints in the order activation runs them, with their package, path, and layer;
- `capabilities`, every capability those entrypoints require, with the entrypoint, layer, and package asking.

`jess package graph` prints this table. Activating it is the host's job: `jess run --package` does it, and a Rust host calls `jess_lua::package::activate`. Activation mounts the plan, so `require` reaches the namespace, then runs the entrypoints, each once, in order. An entrypoint that fails stops the activation with its error; what ran before it stays done, because activation isn't a transaction.

## package.manifest

{{ api_signature(value="package.manifest(text: string) -> JessPackageManifest") }}

Parses and checks a `jess.toml`, with the same errors `open` gives. An unknown key is an error, not a field quietly ignored.

## package.satisfies

{{ api_signature(value="package.satisfies(version: string, requirement: string) -> boolean") }}

Whether `version` meets `requirement`. Requirements are Cargo's caret ranges and exact pins: `^1.2` (and `1.2`) is at least 1.2.0 and below 2.0.0, `^0.2` stays below 0.3.0, `^0.0.3` is exactly 0.0.3, and `=1.2.3` is exactly that.

## The content digest

A package's identity is its id and version. Its content is the digest: BLAKE3 over the files, framed so no two different packages can hash the same.

1. The domain tag `jess-package-digest-v1` and a NUL byte.
2. For every payload file in canonical order (`jess.toml` first, then the rest bytewise): the path's length as a little-endian u64, the path, the file's size as a little-endian u64, and the file's own BLAKE3.

The ZIP container is nowhere in it. A Package Source and the JPK packed from it have the same digest, however the JPK was compressed, which is the point: same digest, same package. Edit one byte of the source and the digest changes, though the version doesn't. A version string is a promise. The digest is evidence.
