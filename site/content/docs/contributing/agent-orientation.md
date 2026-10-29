---
title: 'Orientation for Coding Agents'
description: 'Find the source, types, commands, authority, and evidence needed to work on a Jess project.'
weight: 15
extra:
  kind: guide
---

Start with the repository's own instructions and current tree. Jess is a generic native kernel; application behavior belongs in packages. The [architecture](@/docs/contributing/architecture.md) describes that boundary. A package checkout may be a separate repository with its own instructions and validation commands.

1. **Identify the package or workspace.** Read `jess.toml`, `jess-workspace.toml`, and `jess.lock` when present. `jess package inspect` and `jess package graph` read the description and plan without activating code. [Reading a Package](@/docs/getting-started/reading-a-package.md) explains what to look for.
2. **Use the installed version's types.** `jess luau-types types` writes definitions and module stubs to `types/`; `jess init` configures a project and its editor. [JSL, Luau, and L3i](@/docs/scripting/jsl-and-l3i.md) separates the language, dialect, native primitives, and Jess APIs. The [generated symbol index](@/docs/api/symbols.md) and its JSON catalog locate signatures for the documentation's build; the [API](@/docs/api/_index.md) and [manifest](@/docs/reference/manifest.md) references explain behavior. Consult implementation and tests where a contract is unclear.
3. **Check authority before execution.** Read target and entrypoint capability declarations. A declaration is a request; the caller grants it. `--trusted` grants what a target declares, while the program form of a loose script has broader authority. [Capabilities and Trust](@/docs/getting-started/authority.md) describes the distinction. Use scratch data for operations that write.
4. **Check and test the right unit.** `jess check` handles loose scripts; `jess package check` checks a package. `jess package test` runs source and packed providers; `jess workspace test` resolves workspace members together. In the Jess repository, `cargo test --test integration lua::luau_style::` runs the style gate. The kernel benchmark command is `jess run --capability filesystem.read --capability filesystem.write tools/bench/bench.jsl canaries`, using a verified release build; it writes results and must be appropriate to the task. Other package repositories may have different gates. [Checking and Testing](@/docs/getting-started/testing.md), [Building and Testing](@/docs/contributing/building-and-testing.md), and [Benchmarks](@/docs/contributing/benchmarks.md) explain their scope.
5. **Run the right target.** `jess run` takes a script path. Use `jess package run` or `jess workspace run` for a target; an application target has a host and lifecycle. [Your First Application](@/docs/getting-started/first-application.md) is a complete example.
6. **Report evidence.** Name the commands and versions actually run, what they established, and what was not checked. Types do not prove runtime behavior; a source test does not prove a packaged artifact works; a local build does not prove another platform works. Treat roadmap language and an unimplemented API as neither a supported contract nor a passing test.

These pages are source Markdown under `site/content/docs/`. They do not require a browser or JavaScript to read. Project-specific instructions remain authoritative for the checkout being changed; this page is an orientation, not a substitute for them.
