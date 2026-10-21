---
title: jess Documentation
description: The Jess runtime, package model, application kernel, Luau APIs, CLI, and host architecture.
template: docs/section.html
page_template: docs/page.html
sort_by: weight

extra:
  api_docs: true
  docs_root: true
  docs_project_name: jess
  docs_short_title: jess Docs
  docs_project_path: '@/_index.md'
  docs_repository_url: https://github.com/DreamWeave-MP/jess
  docs_sidebar_label: Documentation
  kind: guide
---

Jess is a native application kernel for Luau applications, built around one rule: **policy does not get a privileged native path just because engines traditionally put it there.** Packages ship systems and tools. Script domains isolate authority. Capabilities make access explicit. L3i exposes the primitives that genuinely earn the boundary.

That makes it deliberately wider than a game engine. The same runtime hosts command-line tools, long-running applications, servers, build pipelines, and the generator that produced this page.

## Where to start

- **New here?** [Install](@/docs/getting-started/install.md), write [your first script](@/docs/scripting/intro.md), then build [your first package](@/docs/getting-started/first-package.md).
- **Want the runtime model?** [Script Domains](@/docs/contributing/script-domains.md), [Application Kernel](@/docs/contributing/application-kernel.md), then [Packages](@/docs/scripting/packages.md).
- **Building a package?** [Packages](@/docs/scripting/packages.md) covers Package Sources, JPKs, layers, resolution, and activation; the [manifest reference](@/docs/reference/manifest.md) has every key; [Capabilities](@/docs/reference/capabilities.md) has every name a target can declare.
- **Building an application?** [Applications](@/docs/scripting/applications.md): managed modules, host callbacks, named events, and interfaces.
- **Looking something up?** The [Luau API Reference](@/docs/api/_index.md) and the [CLI Reference](@/docs/cli/_index.md).
- **Working on Jess itself?** [Contributing](@/docs/contributing/_index.md) covers building, testing, the architecture, and the runtime internals. How decisions are made, who makes them, and what the project commits to is the [Constitution](@/governance.md).

## Architecture in one pass

| Layer | Job |
| --- | --- |
| **Packages / Luau** | Application policy, systems, formats, tools, games, sites, and replaceable implementations. |
| **Script domains** | Stable identity, lifecycle, workers, resources, deterministic contexts, and explicit authority. |
| **Capabilities / IRs** | Narrow contracts across boundaries instead of ambient access to an engine singleton. |
| **L3i** | Lowered operations and native capabilities whose cost model actually justifies native residency. |
| **Host** | Process integration, platform resources, and the smallest practical privileged surface. |

Opening a package does not secretly start it. A host resolves packages into an activation plan, grants capabilities deliberately, and owns teardown. That distinction, **description first, activation second**, is what keeps packages composable instead of turning them into another hidden engine lifecycle.

## Documentation map

| Section | What's in it |
| --- | --- |
| [Getting Started](@/docs/getting-started/_index.md) | Install, the first package, and an editor that knows the types. |
| [Scripting](@/docs/scripting/_index.md) | Scripts, `require`, the built-in modules, packages, applications, performance, and the Luau pitfalls. |
| [Reference](@/docs/reference/_index.md) | The manifest, every capability, the glossary, and the errors. |
| [CLI Reference](@/docs/cli/_index.md) | Every command and flag. |
| [Luau API Reference](@/docs/api/_index.md) | Lookup-first module, type, and function reference. |
| [Contributing](@/docs/contributing/_index.md) | Building, testing, architecture, script domains, the application kernel, render graph, shader IR, input, localization, and releasing. |

## Stability

Jess is pre-1.0. Runtime APIs, package contracts, CLI surfaces, and native interfaces can change between minor versions. If you build on a moving surface, pin the version and treat your package's tests as part of the contract.
