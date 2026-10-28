---
title: 'jess'
description: 'The global table: its members, jess.null, jess.granted, jess.payload, jess.version, and require.'
weight: 10
extra:
  kind: api
---

{{ api_signature(value="jess: { version, null, granted, payload, bench, shader, context, worker, events, interfaces, application, input, audio, gpu }") }}

`jess` is the global table every script gets, however it was started: `jess run`, `jess test`, a command target, or a managed module in an application. It holds one table per native module, plus the few values that belong to no module and had to live somewhere. `require('jess')` is the same table.

{% usage_note(title="Conventions") %}
- Functions, methods, table fields, and report codes are camelCase. Types are PascalCase.
- Failures raise errors. Uncaught, one stops the script with its message and a stack traceback. An outcome isn't a failure: a test suite with failing cases comes back as a report with a `failed` count, because finding that out was the job.
- Option tables, the last argument most functions take, reject keys they don't take. The error names the key and lists the ones the function does take (`unknown worker.spawn option 'sources': expected capabilities, name, shareServices, or source`), so a typo fails instead of quietly doing the default.
- Lists you get back are ordinary 1-based arrays. A list that must keep its positions holds `jess.null` where it has no value.
- Raw bytes are `buffer`s.
- What a script may touch is a capability. Nothing optional is granted unless the operator asked; [`jess.granted`](#jess-granted) says what you hold, and [Capabilities](@/docs/reference/capabilities.md) lists the names.
{% end %}

## Module map

| Member | What it's for | Page |
| --- | --- | --- |
| `jess.context` | Handlers for the phases a ScriptDomain schedules | [jess.context](@/docs/api/context.md) |
| `jess.worker` | Isolated Luau workers and their messages | [jess.worker](@/docs/api/worker.md) |
| `jess.events` | Named application events, queued for the next drain | [jess.events](@/docs/api/events.md) |
| `jess.interfaces` | The tables an application's packages provide, by id, in one domain | [jess.interfaces](@/docs/api/interfaces.md) |
| `jess.application` | Ask the host running an application to stop | [jess.application](@/docs/api/application.md) |
| `jess.shader` | Typed shader programs from Luau modules | [jess.shader](@/docs/api/shader.md) |
| `jess.input` | The native half of `@jess/input`: compiling binding documents, and the runtime | [@jess/input](@/docs/api/input.md) |
| `jess.audio` | The native half of `@jess/audio`: PCM, streams, feeds, and contexts that mix voices through a graph of buses and processors on a sample clock, offline or on a device | [@jess/audio](@/docs/api/audio.md) |
| `jess.gpu` | The GPU: the device jess opens on it | [jess.gpu](@/docs/api/gpu.md) |
| `jess.bench` | What a measurement needs from the host | [jess.bench](@/docs/api/bench.md) |
| `@jess/log` | Structured logging with one sink the host owns | [@jess/log](@/docs/api/log.md) |
| `@jess/test` | Test suites, expectations, and reports | [@jess/test](@/docs/api/test.md) |
| `@jess/jpk` | The JPK container, read under budgets and written canonically | [@jess/jpk](@/docs/api/jpk.md) |
| `@jess/package` | Manifests, digests, packing, and resolution into a plan | [@jess/package](@/docs/api/package.md) |
| `@jess/style` | The Luau style checker | [@jess/style](@/docs/api/style.md) |
| `@jess/analysis` | Luau parsing and analysis the checker is built on | [@jess/analysis](@/docs/api/analysis.md) |
| `@jess/json`, `@jess/yaml` | JSON and YAML documents as tables, and back | [@jess/json](@/docs/api/json.md), [@jess/yaml](@/docs/api/yaml.md) |
| `@jess/data/toml` | TOML documents as tables, and back | [@jess/data/toml](@/docs/api/toml.md) |
| `@jess/data/value` | The one value model the codecs share | [@jess/data/value](@/docs/api/value.md) |
| `@jess/l10n` | Localized text from message bundles | [@jess/l10n](@/docs/api/l10n.md) |
| `@jess/input` | Action graphs, labels, conflicts, intents, and the input runtime | [@jess/input](@/docs/api/input.md) |
| `@jess/audio` | Voices on a sample clock through a graph of buses and processors, streams and feeds, voices in space, tempo maps, offline renders, WAV, and compressed files | [@jess/audio](@/docs/api/audio.md) |
| `@jess/render`, `@jess/render/camera` | Writers for shader layouts and a standard camera | [@jess/render](@/docs/api/render.md) |
| `@jess/backup` | Overwrite a file, keeping what was there as `.bak` | [@jess/backup](@/docs/api/backup.md) |
| `@jess/http` | HTTP/1.1 requests and responses over the transport the runtime exposes | [@jess/http](@/docs/api/http.md) |
| `@dream/fs` | Files and folders | [Files](@/docs/api/fs.md) |
| `bytes.regex` | Regular expressions | [Regular expressions](@/docs/api/regex.md) |
| `@dream/*` | Everything native underneath: bytes, processes, the network, text | [The @dream modules](@/docs/api/l3i.md) |

The `jess` members in the first part of the table are also available through `require('@jess/<name>')`. The next four are [built-in modules](@/docs/scripting/built-in-modules.md), Luau that ships inside the binary. The `@jess/` names after those are the exports of the built-in package, which every runtime carries. The `@dream/` names are [L3i](https://DreamWeave-MP.github.io/l3i/)'s native primitives.

## jess.version

{{ api_signature(value="jess.version: string") }}

The version of jess that's running, such as `"0.1.0"`.

It's the same version `jess --version` prints and the one written at the top of `jess luau-types` output. Check it when a script depends on a feature added in a later release.

```luau
--!strict
print(`running under jess {jess.version}`)
```

## jess.null

{{ api_signature(value="jess.null: JessNull") }}

The value that fills an empty slot in an array, where `nil` would end the array early.

Luau arrays can't hold `nil` in the middle: `{ a, nil, b }` has length 1 or 3 depending on how it was built, and `ipairs` stops at the gap. jess uses `jess.null` wherever a list keeps its positions, and it is the one null in the runtime: the built-in codecs read and write it, so a `null` decoded from JSON is the value any jess API takes.

- **Type:** `JessNull`, an opaque userdata. `typeof(jess.null)` is `"userdata"`.
- **One null:** `require('@jess/json').null`, `require('@jess/yaml').null`, and `@jess/data/value`'s `Null` are `jess.null`.
- **Comparing:** `value == jess.null` works, and the types spell a slot that may be empty as `number | JessNull`.

```luau
--!strict
local json = require '@jess/json'

local items = json.decode '[1, null, 3]' :: { number | JessNull }
print(#items, items[2] == jess.null, json.null == jess.null) -- 3  true  true
```

Related: [@jess/json](@/docs/api/json.md), [@jess/data/value](@/docs/api/value.md).

## jess.granted

{{ api_signature(value="jess.granted(capability: string) -> boolean") }}

Whether the running code holds `capability`: `'process.environment'`, `'network.transport'`, `'filesystem.read'`, and so on. Outside a ScriptDomain the answer is the runtime's grant, what `--capability`, `--trusted`, or a target's declared capabilities gave it; under a domain it is the broker's, for the context or worker asking. Asking is not a denial, so nothing is reported for a `false`.

Ask before reaching for something optional instead of calling it and catching the refusal:

```luau
--!strict
local process = require '@dream/process'

if jess.granted 'process.environment' then
  print(`home is {process.env 'HOME' or 'unset'}`)
else
  print('run with --capability process.environment to read the environment')
end
```

- **Errors:** `jess.granted: unknown capability '<name>'; a jess runtime can be granted filesystem.read, filesystem.write, ...` for a name no jess runtime can hold, so a misspelling isn't quietly `false`. The list in the message is [every capability there is](@/docs/reference/capabilities.md).

## jess.payload

{{ api_signature(value="jess.payload() -> { files: { string }, read: (path: string) -> buffer? }") }}

The calling module's own package payload: `files`, every path in canonical order (`jess.toml` first, then the rest bytewise), and `read`, one file's bytes or `nil` for a path the payload lacks. It is how a package ships things that aren't Luau, templates, stylesheets, fonts, and hands them to whoever depends on it: the package exports a module that returns what `jess.payload()` gave it, and a dependent requires that module. Nothing reads another package's payload uninvited, and the same code answers the same from a Package Source and from the JPK packed from it.

- **Returns:** a fresh table each call. `read` answers from the package's provider, the folder or the archive, so it needs no capability of its own.
- **Errors:** `jess.payload: <chunk> is a script, not a package's module, so it has no payload` outside a package.

```luau
--!strict
-- src/init.jsl of a theme package: its templates and stylesheets, for a site generator to read.
local payload = jess.payload()

return table.freeze { files = payload.files, read = payload.read }
```

## jess.bench

{{ api_signature(value="jess.bench: { verified: boolean, collect: () -> () }") }}

What a measurement needs from the host: whether this jess was built with the verified toolchain, as releases are, and a full garbage collection, so one batch's garbage isn't collected inside the next one's timing. A harness refuses to post a number from an unverified build, because it would be measuring a slower Luau. [jess.bench](@/docs/api/bench.md) has the details.

## require

{{ api_signature(value="require(name: string) -> any") }}

Loads a module. Four kinds of name resolve, and nothing else does:

- `jess`, the global table, and `@jess/<name>` for each of its members (`require('@jess/shader')` is `jess.shader`), for the built-in modules (`@jess/log`, `@jess/test`, `@jess/jpk`, `@jess/package`), and for the built-in package's exports (`@jess/style`, `@jess/json`, ...). The namespace is closed: no file and no other package can take a `@jess/` name.
- `@dream/<name>`, the [L3i modules](@/docs/api/l3i.md): `@dream/fs`, `@dream/bytes`, `@dream/process`, and the rest.
- The exports of the packages activated for this run: a `--package` on `jess run`, the layers of the command target that is running, or an application's whole graph. `@greeter` resolves when a package exporting it is active, and to nothing otherwise.
- A name starting with `./` or `../` is relative to the module calling `require`, as in Luau's require by string: jess tries `name.jsl`, then `name/init.jsl`. Inside a package, `@self/` is the package's own root. An `init` script stands for its folder, so its `./` starts beside the folder.

Each module runs once, called with its name as `...`, and is cached: the same value comes back from every `require`, by whichever name. A package module's return value is frozen, so one package can't patch another's export for the whole plan.

- **Errors:** `module not found: <name>`, with a reason for a name that looks like it should have worked: `module not found: @greeter: no package activated in this run exports it, and it isn't one of jess's own modules`, or `module not found: @jess/nope: @jess/ is jess's own namespace (...), never a file`.

```luau
--!strict
type Words = { farewell: (name: string) -> string }

local jessAgain = require 'jess'
local words: Words = require './words' -- words.jsl, or words/init.jsl, beside this file

print(words.farewell 'world', jessAgain == jess)
```

`require` is typed `(name: string) -> any` for a relative name: a module's type is whatever you annotate the result with, as with `words: Words` above. The `@jess/` and `@dream/` names are typed by the stubs `jess luau-types` writes, so `local log = require '@jess/log'` needs no annotation under `--!strict`.

Related: [Built-in Modules](@/docs/scripting/built-in-modules.md), [Packages](@/docs/scripting/packages.md#modules-and-require).

## Globals set by the runner

`jess run script.jsl a b` also sets these:

| Global | Value |
| --- | --- |
| `...` | The script's arguments as strings: `"a", "b"`. |
| `arg` | The same arguments as a table, with the script path at `arg[0]`: `{ [0] = "script.jsl", "a", "b" }`. |
| `print` | Luau's `print`, writing through jess's own stdout so `\n` stays `\n` on Windows and lines come out in order. |

A command target's entry gets the arguments after `--` the same way, with the module's package-qualified name at `arg[0]` (`com.example.greeter@1.0.0/tools/greet.jsl`). A test module under `jess test` gets no arguments; it returns a suite, and the runner calls that. There are no other globals: Luau's own libraries, `jess`, `arg`, `require`, and `print`.

```luau
-- jess run args.jsl a b
local first, second = ...
print(first, second) -- a  b
print(#arg, arg[0]) -- 2  args.jsl
```

Both are typed: `...` as the script's varargs, and `arg` as `{ [number]: string }`.

## Type definitions

`jess luau-types types` writes the types for everything on these pages, `jess.d.jsl`, and a file for every module `require` resolves by name: a stub for each native one, `types/jess/log.jsl`, `types/dream/fs.jsl`, and the built-in package's modules themselves, `types/jess/data/json.jsl` and the rest, since a Luau module's type is its source. `jess init` does it for you, along with a strict `.jessrc` aliasing the names and your editor's luau-lsp settings. Start scripts with `--!strict`: the types were written for the checker, and the checker catches the typo before jess has to. [Editor Setup](@/docs/getting-started/editor-setup.md) covers the editors.
