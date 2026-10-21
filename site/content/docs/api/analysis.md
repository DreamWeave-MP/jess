---
title: '@jess/analysis'
description: 'Semantic facts over Luau source: requires, calls, members, functions, and a folder of modules with the edges between them.'
weight: 125
extra:
  kind: api
---

{{ api_signature(value="require('@jess/analysis'): { analyze, file, workspace, files, callsTo, callsInModule, qualifiedPath, literalString, tableField, resolvedTableField, resolve, line, normalizePath }") }}

`@jess/analysis` reads Luau source and answers questions about it, Luau in the built-in package over [`@dream/luau`](@/docs/api/l3i.md)'s parser. It knows Luau and jess, and nothing about what any particular package means by its code: a linter, a dependency checker, or a documentation tool takes the facts here and adds its own semantics. [`@jess/style`](@/docs/api/style.md) is built this way.

`analyze` takes text and needs no capability. `file`, `files`, and `workspace` read the disk, so they need `filesystem.read`; a script without it stops at the first read, naming the function and the capability.

## analysis.analyze

{{ api_signature(value="analysis.analyze(source: string, options: { declarations: boolean?, path: string? }?) -> Source") }}

Parses one chunk and walks it once, collecting the facts below. A chunk that doesn't parse still gives a `Source`, with what could be read and the parse errors beside it; when the text is a declaration file (`declare` statements) and `declarations` isn't given, the module parses it a second time with declarations on and keeps that result if it is clean.

- **Parameters:** `source`, the text. `options.declarations`: parse `declare` statements, as `jess.d.jsl` needs. `options.path`: recorded as `source.path`, for messages.
- **Returns:** a `Source`, with:
  - `root`, `errors`, `comments`, `hotComments`, the parse result as `@dream/luau` gives it;
  - `lineStarts`, the byte offset of each line, and `source`, the text;
  - `requires`, one `RequireFact` per `require` of a string literal: `module`, the name as written, `binding`, the local it was assigned to, `protected` and `via`, whether it ran inside `pcall`, and `expr`, the call. A require whose argument isn't a literal string isn't a fact; nothing here evaluates code;
  - `calls`, one `CallFact` per call: `target`, the callee as a dotted path where it can be named, and `module` and `member` when the callee is a member of a required module;
  - `functions`, `members`, `tables`, `assignments`: every function expression, every member read, every table constructor, every assignment, each with its span;
  - `globals`, how often each free name is read; `strings`, every string literal with its position;
  - `origins` and `localValues`, what each local is (a required module, a global, or a member of one) and the expression it was last given;
  - `moduleReturn`, the chunk's final `return` expression, when it has one.

  Every fact carries `line`, `column`, `endLine`, `endColumn`, and `functionDepth`, `loopDepth`, `conditionalDepth`: how many functions, loops, and branches it sits inside, so a tool can tell a require at the top of a module from one inside a function.

```luau
--!strict
local analysis = require '@jess/analysis'

local source = analysis.analyze [[
--!strict
local log = require '@jess/log'
local greeter = require '@greeter'

local logger = log.new 'greet'

local function run(name: string)
  logger.info(greeter.greet(name))
end

return table.freeze { run = run }
]]

for _, fact in source.requires do
  print(fact.module, fact.line, fact.protected, fact.binding and fact.binding.name)
end

for _, call in analysis.callsInModule(source, '@greeter') do
  print(call.target, call.module, call.member, call.line, call.functionDepth)
end

print(#source.functions, source.functions[1].name, source.moduleReturn and source.moduleReturn.kind)
print(analysis.line(source, 2))
```

```text
@jess/log	2	false	log
@greeter	3	false	greeter
@greeter.greet	@greeter	greet	8	1
1	run	ExprCall
local log = require '@jess/log'
```

`greeter.greet` is a call into `@greeter` because the walker followed `greeter` back to its `require`. `functionDepth` is 1: the call is inside `run`.

## analysis.file

{{ api_signature(value="analysis.file(path: string) -> Source") }}

Reads one file and analyzes it, with `source.path` set to the path with its separators normalized. Needs `filesystem.read`.

- **Errors:** `<path>: <why>` when the file can't be read.

## analysis.files, analysis.workspace

{{ api_signature(value="analysis.files(paths: { string }, options: WorkspaceOptions?) -> { string }; analysis.workspace(paths: { string }, options: WorkspaceOptions?) -> Workspace") }}

`files` walks the paths given, folders recursively, and returns every Luau source under them, sorted. Folders whose names start with `.` are skipped, as are `target` and `node_modules`; `options.skipDirectories` adds names, and `options.extensions` replaces the extensions it collects. A path that doesn't exist is an error, not an empty result: finding no files would read as finding nothing wrong.

`workspace` reads and analyzes every file `files` finds, then pairs each module's requires with the file they name. Needs `filesystem.read`.

- **Parameters:** `paths`, the roots. `options.transform`: a function from a file's text and path to the text to analyze, for a tool that strips or rewrites something first.
- **Returns:** a `Workspace`: `roots`, `files`, `sources` in file order, `byPath` and `byModule` to find a source by its path or its module key, `requireEdges` (`{ from, to, module, line }` for each require paired with a file) and `unresolvedRequires` (`{ from, module, line }` for the rest). Names starting with `@` are a package's or jess's business and are never edges.
- **Errors:** `<path>: no such file or folder` for a root that isn't there; `<path>: <why>` for a folder that can't be listed.

```luau
--!strict
local analysis = require '@jess/analysis'

local workspace = analysis.workspace { 'src' }
print(#workspace.files, table.concat(workspace.files, ', '))
```

```text
$ jess run --capability filesystem.read inventory.jsl
3	src/greet.jsl, src/stray.jsl, src/words.jsl
```

Without the flag:

```text
error: dream.fs.exists: needs the 'filesystem.read' capability, which this runtime does not grant
```

## analysis.callsTo, analysis.callsInModule

{{ api_signature(value="analysis.callsTo(source: Source, target: string) -> { CallFact }; analysis.callsInModule(source: Source, module: string) -> { CallFact }") }}

The calls whose `target` is exactly `target` (`'print'`, `'@jess/log.new'`), or whose callee is any member of the module required as `module`.

## analysis.resolve, analysis.tableField, analysis.resolvedTableField

{{ api_signature(value="analysis.resolve(source: Source, expr: Expr, depth: number?) -> Expr; analysis.tableField(expr: Expr, name: string) -> Expr?; analysis.resolvedTableField(source: Source, expr: Expr, name: string) -> Expr?") }}

`resolve` follows a local back through the expressions it was given, up to `depth` steps (16 by default) and never around a cycle, and through groups and type assertions, to the expression that made it. `tableField` is the value of a `name = value` entry in a table constructor; `resolvedTableField` resolves the expression first, so a table built into a local and returned later still answers. A tool that reads a module's returned table uses these to find the `run` in `return table.freeze { run = run }`.

## analysis.qualifiedPath, analysis.literalString, analysis.line, analysis.normalizePath

{{ api_signature(value="analysis.qualifiedPath(expr: Expr, origins: { [Local]: Origin }) -> (string?, string?, string?); analysis.literalString(expr: Expr) -> string?; analysis.line(source: Source, number: number) -> string; analysis.normalizePath(path: string) -> string") }}

`qualifiedPath` names an expression as the walker does: the dotted path, the module it is a member of, and the member, or `nil` for one that can't be named. `literalString` is the value of a string-literal expression, `nil` for anything else. `line` is one line of the source text without its line ending, or `''` past the end. `normalizePath` turns backslashes into slashes, drops a leading `./`, and collapses repeated slashes; it is for module keys and messages, never for a path handed back to the filesystem, where a backslash can be part of a name.

Related: [`@jess/style`](@/docs/api/style.md), [Luau Style](@/docs/contributing/luau-style.md).
