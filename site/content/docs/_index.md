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

Jess is a high-performance Luau application runtime built around a narrow rule: **policy does not get a privileged native path just because engines traditionally put it there.** JPKs package systems and tools. Script domains isolate authority. Capabilities make access explicit. L3i exposes the primitives that genuinely earn the boundary.

The result is deliberately wider than a game engine. The same runtime can host game systems, content tooling, format implementations, build pipelines, servers, and this documentation generator.

## Where to start

- **Want the runtime model?** Read [Application Kernel](@/docs/contributing/application-kernel.md), [Script Domains](@/docs/contributing/script-domains.md), then [Packages and JPKs](@/docs/scripting/packages.md).
- **Want to write Luau?** Start with [Your First Script](@/docs/scripting/intro.md), then use the [Luau API Reference](@/docs/api/_index.md) for lookup.
- **Want to build a package?** [Packages and JPKs](@/docs/scripting/packages.md) covers manifests, dependencies, exports, resolution, activation, and capability requirements.
- **Want to understand what stays native?** [Architecture](@/docs/contributing/architecture.md) and the L3i boundary explain where Rust ends and application policy begins.
- **Want the CLI?** The [CLI Reference](@/docs/cli/_index.md) documents commands and flags.
- **Working with Bethesda/OpenMW content?** That application stack lives in Dreamwind; start with the [Bundled Scripts](@/docs/bundled-scripts/_index.md), [NIF Deep Dive](@/docs/nif/_index.md), or the task-first [Guides](@/docs/guides/_index.md).
- **Working on Jess itself?** [Contributing](@/docs/contributing/_index.md) covers building, testing, architecture, benchmarks, rendering, input, localization, and runtime internals.

## Architecture in one pass

| Layer | Job |
| --- | --- |
| **JPK / Luau** | Application policy, systems, formats, tools, games, sites, and replaceable implementations. |
| **Script domains** | Stable identity, lifecycle, workers, resources, deterministic contexts, and explicit authority. |
| **Capabilities / IRs** | Narrow contracts across boundaries instead of ambient access to an engine singleton. |
| **L3i** | Lowered operations and native capabilities whose cost model actually justifies native residency. |
| **Host** | Process integration, platform resources, and the smallest practical privileged surface. |

Opening a package does not secretly start it. A host resolves packages into an activation plan, grants capabilities deliberately, and owns teardown. That distinction — **description first, activation second** — is what keeps packages composable instead of turning them into another hidden engine lifecycle.

## Documentation map

| Section | What's in it |
| --- | --- |
| [Scripting](@/docs/scripting/_index.md) | Luau programs, package use, records/views, files, VFS, performance, and downstream libraries. |
| [Luau API Reference](@/docs/api/_index.md) | Lookup-first module, type, and function reference. |
| [CLI Reference](@/docs/cli/_index.md) | Commands and flags. |
| [Contributing](@/docs/contributing/_index.md) | Runtime architecture, application kernel, domains, render graph, shader IR, input, localization, benchmarks, and development. |
| [Bundled Scripts](@/docs/bundled-scripts/_index.md) | Dreamwind's load-order tools and their measured behavior. |
| [NIF Deep Dive](@/docs/nif/_index.md) | NIF structure, editing, construction, optimization, and compatibility checks. |
| [Guides](@/docs/guides/_index.md) | Task-first compatibility and content workflows. |
| [Glossary and Troubleshooting](@/docs/reference/_index.md) | Terms, diagnostics, and fixes. |
| [Rust Crates](@/docs/rust/_index.md) | Native host/library interfaces that still exist below or alongside the Luau runtime. |

## Stability

Jess is still pre-1.0. Runtime APIs, package contracts, CLI surfaces, and native interfaces can change between minor versions. If you build on a moving surface, pin the version and treat the benchmark suite plus package tests as part of the contract.
