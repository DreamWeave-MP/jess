---
title: 'Your First Script'
description: 'Set up a folder, write a jess script, run it, and learn the handful of rules the whole API follows.'
weight: 10
extra:
  kind: guide
---

A jess script is a [Luau](https://luau.org) file with the `jess` global already there. No build step, no project file you have to understand first: write a file, run it. There is a one-command setup, and you want it, because it writes the types.

## Set up a folder

```sh
jess init my-scripts
```

```yaml
dir: my-scripts
editor: vscode
types:
  folder: my-scripts/types
  definitions: my-scripts/types/jess.d.jsl
  files: 44
  removed: []
  aliases:
    dream: types/dream
    jess: types/jess
written:
- path: my-scripts/.jessrc
- path: my-scripts/main.jsl
- path: my-scripts/.vscode/settings.json
skipped: []
next_steps:
- 'run the example: jess run my-scripts/main.jsl'
- 'type-check it: jess check my-scripts/main.jsl'
- 'vscode: install the Luau Language Server extension (JohnnyMorganz.luau-lsp) and open the folder'
- rerun jess init after updating jess to refresh types/
```

Four things landed: `types/`, the Luau types for the jess you have installed; `.jessrc`, which turns strict mode on for the folder and aliases `@jess/` and `@dream/` into `types/`; the editor settings that point [luau-lsp](https://github.com/JohnnyMorganz/luau-lsp) at them (`--editor neovim`, `zed`, `helix`, or `none` for another); and `main.jsl`:

```luau
--!strict
local log = require('@jess/log').new 'main'

print(`Hello from jess {jess.version}.`)
log.info 'ready'
```

Run it:

```sh
$ jess run my-scripts/main.jsl
Hello from jess 0.3.0.
{"level":"info","target":"main","message":"ready","fields":{},"source":"my-scripts/main.jsl"}
```

That is a complete script. Three things happened:

- `jess` is a global. Its members are the runtime's own API: `jess.version`, `jess.context`, `jess.worker`, `jess.events`, and the rest of the [`jess` page](@/docs/api/jess.md). `print` is Luau's, and goes to stdout.
- `require('@jess/log')` loaded a [built-in module](@/docs/scripting/built-in-modules.md). It is Luau that ships inside the binary, and the `@jess/` name resolves in every runtime without a file anywhere.
- The logger wrote one JSON line to stderr. Where log records go is the host's decision, not the script's: `jess run` writes them to stderr, `jess test` collects them per case. Two streams, two jobs: what the script is for goes to stdout, what it has to say about itself goes to stderr.

## Arguments

Everything after the script's path is the script's, as strings, two ways:

```luau
--!strict
local first, second = ...
print(first, second)
print(#arg, arg[0])
```

```text
$ jess run args.jsl a b
a	b
2	args.jsl
```

`...` is the script's varargs; `arg` is the same list as a table, with the script's own path at `arg[0]`. Flags after the script belong to the script, not to jess: `jess run build.jsl --verbose` hands `--verbose` to `build.jsl`.

## Make it executable

A script can be a program. Put a shebang line above `--!strict`, make the file executable, and run it like anything else on your `PATH`:

```luau
#!/usr/bin/env jess
--!strict
local process = require '@dream/process'

print(`Hello, {process.env 'USER' or 'stranger'}.`)
```

```text
$ chmod +x greet.jsl
$ ./greet.jsl
Hello, dave.
```

Luau has no notion of a shebang; jess blanks that line before compiling and keeps the line numbers after it. The shebang hands jess `jess ./greet.jsl`, and a `.jsl` path where a command would go runs the script as a program: trusted, with every capability, which is why the environment read above needed no flag. You made it executable and ran it, the same call you make for any program. `jess run greet.jsl` is the cautious spelling of the same thing, denied by default, for a script you are reading rather than trusting; [Capabilities](#capabilities) below covers that side.

{% usage_note(title="The program form is for scripts you trust") %}
`jess script.jsl`, `./script.jsl`, and a file dropped onto `jess.exe` all run with every capability: files, your environment, processes, and the network, as your account. That is the right default for a program you wrote or chose to install, and the wrong one for a `.jsl` you just downloaded. For a script you haven't read, `jess run script.jsl` runs it denied by default and tells you exactly what it reaches for; `jess check script.jsl` reads it without running anything.
{% end %}

### On Windows

Dropping a `.jsl` file onto `jess.exe` in Explorer runs it the same way, as a program. Two things to know: the console window jess opens closes the moment the script finishes, so output and errors disappear with it, and double-clicking a `.jsl` does nothing unless you associate the extension with `jess.exe` yourself, which jess doesn't do for you. When you want to read what a script printed, or why it stopped, run it from a terminal:

```text
> jess greet.jsl
Hello, dave.
> jess run greet.jsl
error: dream.process.env: needs the 'process.environment' capability, which this runtime does not grant
```

## Turn on the types

That `--!strict` on the first line isn't decoration. It tells Luau to type-check the whole file, and jess ships a type for every function it exposes and every module `require` resolves. Strict mode catches the misspelled field before you run the script, not after, when the only evidence is a job that quietly did nothing.

`jess init` already wired the types into your editor. By hand, `jess luau-types types` writes the same folder, and `.jessrc` needs the two aliases it prints:

```json
{
  "languageMode": "strict",
  "aliases": {
    "dream": "types/dream",
    "jess": "types/jess"
  }
}
```

[Editor Setup](@/docs/getting-started/editor-setup.md) covers each editor and the type solver flag some of the types need.

{% usage_note(title="Regenerate after upgrading") %}
`types/` describes one jess version. After you upgrade jess, run `jess init` in the folder again (it only refreshes `types/`, leaving your files alone), or the editor will happily approve calls the new version doesn't have.
{% end %}

## Check it from the command line

The editor isn't the only thing that reads the types. [`jess check`](@/docs/cli/check.md) runs the same strict check over a script and prints what it found, which is nothing for `main.jsl`:

```sh
jess check my-scripts/main.jsl
```

```text
files: 1
errors: []
```

Put a mistake in a script and the check names the file, the line, the column, and the problem, then exits `1`:

```luau
--!strict
local function width(): number
  return 'three'
end

print(width())
```

```text
$ jess check wrong.jsl
files: 1
errors:
- file: wrong.jsl
  line: 3
  column: 10
  message: Expected this to be 'number', but got 'string'
error: 1 type error
```

The check follows `./` and `../` requires and reports those files too, and `--package` names packages whose exports the script requires, so they type the way they will under [`jess run --package`](@/docs/cli/run.md#running-with-packages). Nothing runs. `jess run` never type-checks: strict mode is a promise the checker keeps, not the runtime, so run the check before the script, or let the editor run it as you type.

## require

`require` resolves four kinds of name and nothing else:

- `@jess/<name>`: a member of `jess` (`@jess/worker` is `jess.worker`), a built-in module (`@jess/log`, `@jess/test`, `@jess/jpk`, `@jess/package`), or an export of the built-in package (`@jess/style`, `@jess/json`, ...). The namespace is closed: `require '@jess/whatever'` is `module not found: @jess/whatever: @jess/ is jess's own namespace (...), never a file`.
- `@dream/<name>`: one of [L3i's native modules](@/docs/api/l3i.md), `@dream/fs`, `@dream/bytes`, `@dream/process`, and the rest.
- The exports of a package activated with [`jess run --package`](@/docs/cli/run.md#running-with-packages), by the names its manifest gives them.
- `./` and `../` paths, relative to the file calling `require`: `require './lib/util'` tries `lib/util.jsl`, then `lib/util/init.jsl`, beside the script. A module runs once and is cached, so two scripts requiring the same file get the same table.

A bare name is none of those. `require 'lib.util'` is `module not found: lib.util`.

```luau
--!strict
type Util = { answer: number }
local util: Util = require './lib/util'

print(util.answer)
```

`require` is typed `(name: string) -> any` for a path, so annotate what comes back, as `util: Util` does; the `@jess/` and `@dream/` names are typed by their stubs and need nothing.

## Capabilities

A script you run holds what you already have: your terminal, and the input devices and the GPU in front of you. Everything else is a [capability](@/docs/reference/capabilities.md), denied unless you grant it, and the refusal names the function and the capability:

```luau
--!strict
local fs = require '@dream/fs'

print(fs.readFileString 'settings.toml')
```

```text
$ jess run read.jsl
error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant
$ jess run --capability filesystem.read read.jsl
```

`jess.granted('filesystem.read')` says whether the running code holds one, so a script can ask before reaching for something optional instead of catching the refusal.

## House rules

A handful of conventions hold across the whole API. Learn them once.

- **camelCase everywhere.** Functions, methods, fields, and option keys: `fs.readFileString`, `log.new`, `{ atomic = true }`. Types are PascalCase.
- **Errors raise.** A bad argument, an option key the function doesn't take, a missing capability, or a `require` that resolves nothing throws. Uncaught, it stops the script, prints `error: read.jsl:4: <message>` on stderr, and exits `1`. Don't wrap calls in `pcall` to find out whether a field exists or a call works. Know the type and use the documented field. A script that swallows its own bugs isn't resilient; it's lying to you.
- **Outcomes aren't errors.** A file that isn't there is an outcome: `@dream/fs` answers `nil`, a message, and a kind (`notFound`) rather than throwing, so `assert(fs.readFileString(path))` is how a script says a missing file is fatal, and `fs.readFileString(path) or ''` is "use it if it's there". Errors mean jess couldn't do what you asked; a result tells you what it found.
- **Lists are 1-based.** Every list jess returns is an ordinary Luau array starting at 1.
- **Bytes are buffers.** Anything that hands you raw bytes hands you a Luau `buffer`, and anything that takes bytes also takes a string.
- **Holes are `jess.null`.** Where a list has to keep a missing value in place, it holds `jess.null`, because `nil` would shift everything after it. It is the one null: `require('@jess/json').decode('[1,null,3]')[2] == jess.null` is `true`.
- **The exit status is yours.** A script's first returned number, an integer from 0 to 255, is the process's exit status. `return 3` exits `3`.

## Where next

- [Your first package](@/docs/getting-started/first-package.md): the same script with a manifest around it, tested, packed, and run as a command.
- [Files, Data, and Regex](@/docs/scripting/files-toml-regex.md): the standard-library gaps jess fills.
- [Packages](@/docs/scripting/packages.md), where almost everything else lives.
- The [Luau API Reference](@/docs/api/_index.md), for every signature, and [`jess run`](@/docs/cli/run.md) for the command's flags.
