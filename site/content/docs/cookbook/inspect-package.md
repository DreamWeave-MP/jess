---
title: 'Trace a Package Before Running It'
description: 'Inspect the package, its resolved layer plan, and the authority its targets request.'
weight: 30
extra:
  kind: guide
---

Given the `greeter` folder from [Your First Package](@/docs/getting-started/first-package.md), inspect the package and its activation plan:

```sh
jess package inspect greeter
jess package graph greeter
```

Neither command runs package code. The inspection tells you the package id, version, digest, exported modules, layers, targets, and capability declarations. The graph shows the resolved layers and entrypoints. Read both: the manifest describes what a package offers, and the plan shows what a particular resolution selects.

For this example, the default command requests `process.environment`. Run it with that named grant only after you have decided it is appropriate:

```sh
jess package run --capability process.environment greeter
```

An export can be required without being an entrypoint, and a dependency is not itself a request to activate every layer in that package. [Modules, Layers, and Dependencies](@/docs/getting-started/modules-and-layers.md) explains those distinctions; [Capabilities and Trust](@/docs/getting-started/authority.md) explains the grant.
