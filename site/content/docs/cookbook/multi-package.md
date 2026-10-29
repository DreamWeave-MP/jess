---
title: 'Compose Two Packages'
description: 'Develop a command and its library together, then pack their workspace.'
weight: 90
extra:
  kind: guide
---

This workspace has a library package, `words`, and a command package, `tool`. The command declares the library as a dependency; the workspace makes both sources available while you develop them. No network or additional packages are needed.

Create this layout:

```text
example/
  jess-workspace.toml
  words/jess.toml
  words/src/init.jsl
  tool/jess.toml
  tool/src/main.jsl
```

`example/jess-workspace.toml` names the members:

```toml
manifest_version = 1

[workspace]
members = ["words", "tool"]
```

`words/jess.toml` describes the library and its export:

```toml
manifest_version = 1

[package]
id = "com.example.words"
version = "0.1.0"
jess_api = "1"
include = ["src"]

[modules]
"@words" = "src/init.jsl"

[[layer]]
id = "example.words"
exports = ["@words"]
```

The library at `words/src/init.jsl` returns an inert API:

```luau
--!strict
local function greeting(name: string): string
  return `Hello, {name}.`
end

return table.freeze { greeting = greeting }
```

`tool/jess.toml` declares its dependency and command target:

```toml
manifest_version = 1

[package]
id = "com.example.tool"
version = "0.1.0"
jess_api = "1"
include = ["src"]
default_target = "greet"

[dependencies]
"com.example.words" = "0.1"

[targets.greet]
kind = "command"
entry = "src/main.jsl"
```

The command at `tool/src/main.jsl` uses the export:

```luau
--!strict
local words = require '@words'

print(words.greeting 'world')
```

From `example`, inspect and run the workspace:

```sh
jess workspace info
jess workspace targets
jess workspace run com.example.tool:greet
```

The command prints `Hello, world.`. The library's package dependency makes its export resolvable to the tool; requiring the export does not activate a layer entrypoint. This library has no entrypoint.

Pack the workspace and run its default target:

```sh
jess workspace pack --output ../example.jsw
jess ../example.jsw
```

The packed workspace prints the same greeting. It contains both packages and runs offline with Jess. Editing either source afterwards requires a new packed artifact. Both members here are local and there are no external dependencies to lock; [workspace sync](@/docs/cli/workspace.md#jess-workspace-sync) explains pinning dependencies once you add them. The fixture is `tests/fixtures/docs/multi`; the documentation checks type-check, pack, and run it.

## Pin the library instead

To see the external-dependency path without a network, change the workspace file to `members = ["tool"]`. Leave the `words` source folder where it is and, from `example`, offer it to sync:

```sh
jess workspace sync words
jess workspace sync --locked --offline
jess workspace run com.example.tool:greet
```

The first command packs the offered source into the workspace's `.jess/packages` and writes `jess.lock` with the selected version and digest. The second checks the existing lock and kept package without consulting a publisher; the command still prints the greeting. Commit the workspace file and lock for a real project. The kept package is regenerable and is not a workspace member. The tool's own dependency declaration did not change.

Publishers and indexes provide another way to supply external packages; they need network access during sync. [Workspace sync](@/docs/cli/workspace.md#jess-workspace-sync) specifies those sources and their integrity checks.
