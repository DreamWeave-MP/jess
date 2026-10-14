---
title: '@jess/style'
description: "jess's Luau style checker: findings with rule ids, spans, and severities, over Luau's own parser, from the built-in package jess carries."
weight: 117
extra:
  kind: api
---

{{ api_signature(value="require('@jess/style'): { check, checkFile, files, format, markdownBlocks, rules, run }") }}

The [Luau style](@/docs/contributing/luau-style.md) as a checker. It's an export of jess's [built-in package](@/docs/scripting/packages.md#the-built-in-package), `jpk/builtin`, layer `jess.style`, which the binary carries, so every script can `require` it.

```sh
jess run check-style.jsl libs jpk
```

```luau
--!strict
-- check-style.jsl: the findings in one source, as data.
local style = require '@jess/style'
local findings = style.check 'local Total = 0\nTotal += 1\nprint("total", Total)\n'

for _, finding in findings do
  print(style.format(finding, 'example.jsl'))
end
```

```text
example.jsl:1:7: error[naming.case]: 'Total' is reassigned, so it isn't a constant: name it in camelCase
example.jsl:3:7: error[quotes.double]: single quotes: double quotes are for a string that holds a single quote
```

It parses with [`@dream/luau`](@/docs/api/downstream-globals.md), so a file that doesn't parse gets `syntax` findings at Luau's own error positions, and the rest of the tree, which the parser recovers, is still checked.

### style.check

{{ api_signature(value="style.check(source: string, options: Options?) -> { Finding }") }}

Every finding in `source`, sorted by position.

| Finding field | Is |
| --- | --- |
| `rule` | The rule's id, such as `pcall.comment`; `style.rules` has them all. |
| `severity` | `'error'` or `'warning'`. |
| `message` | What's wrong and what the style wants instead. |
| `line`, `column`, `endLine`, `endColumn` | The span, 1-based, the end column inclusive, columns in bytes. |

| Option | Does |
| --- | --- |
| `profile` | `'jess'`, the default, or `'lua'` for s3stash's Lua: no `hoisting` rule, PascalCase allowed for module-level and member-alias locals, LuaLS `---@param` types read for `truthiness`. |
| `directives` | Hot comments the file must start with, such as `{ 'strict', 'native' }`. |
| `noAny` | Report every `any` type (`types.any`). |
| `declarations` | `true` parses definition-file syntax (`declare`, `declare extern type`), `false` doesn't; left out, a source that only parses with it is taken for a definitions file. |
| `only` | Only these rules, by id. An unknown id is an error. |
| `fragment` | The source is an excerpt, such as a block on a docs page: no local is taken for a module constant, since what the rest of the module does with it isn't there to see. |

### style.checkFile

{{ api_signature(value="style.checkFile(path: string, options: Options?) -> { Finding }") }}

`style.check` on a file. A `.md` file is checked one ```` ```luau ```` block at a time, as a fragment, with each finding's lines counted from the top of the page.

### style.files

{{ api_signature(value="style.files(paths: { string }) -> { string }") }}

The `.jsl` and `.lua` files under each path, sorted. A path that's a file is kept whatever its extension; folders named `target` and dot-folders are skipped.

### style.run

{{ api_signature(value="style.run(paths: { string }, options: Options?) -> (number, number)") }}

Checks every file `style.files(paths)` finds, prints each finding with `style.format`, and returns how many errors and how many warnings there were.

### style.format

{{ api_signature(value="style.format(finding: Finding, path: string?) -> string") }}

One finding on one line: `path:line:column: severity[rule]: message`, or without the path.

### style.markdownBlocks

{{ api_signature(value="style.markdownBlocks(text: string) -> { { line: number, source: string } }") }}

A Markdown page's ```` ```luau ```` blocks, each with the line its opening fence is on.

### style.rules

{{ api_signature(value="style.rules: { [string]: { severity: 'error' | 'warning', summary: string } }") }}

Every rule by id, frozen. [Luau Style](@/docs/contributing/luau-style.md#the-checker) has the table.
