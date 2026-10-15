---
title: 'Files, TOML, and Regex'
description: 'The standard-library gaps jess fills: @dream/fs, @jess/data/toml, bytes.regex.'
weight: 90
extra:
  kind: guide
---

Luau on its own can't open a file, read a config, or match a regex. Scripts that write logs, read settings, or match record ids need all three, so jess has them: `@dream/fs`, `@jess/data/toml`, and `bytes.regex`. This page is recipes; the [fs](@/docs/api/fs.md), [toml](@/docs/api/toml.md), and [regex](@/docs/api/regex.md) references have every function.

## Files

[`@dream/fs`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-fs) is file
access with host paths: the paths your operating system uses, not VFS paths. Reading needs
`filesystem.read` and changing the disk `filesystem.write`, both a `--capability` away for
[`jess run`](@/docs/cli/run.md#capabilities); taking paths apart and joining them needs neither.

### Walk a folder

```luau
--!strict
local fs = require('@dream/fs')

local root = ... or '.'
local walk = assert(fs.walk(root, { include = 'files', metadata = true, sort = true }))

local total = 0
for index in walk.paths do
  total += (walk.sizes :: { number })[index]
end
print(`{#walk.paths} files, {total} bytes under {root}`)
```

```text
$ jess run --capability filesystem.read walk.jsl data
2 files, 8 bytes under data
```

`walk` returns parallel arrays (`paths`, relative to the root and `/`-separated everywhere,
`kinds`, and with `metadata` the `sizes`), so a walk of a hundred thousand files makes a handful
of tables. `list(folder)` is one folder's names, sorted.

### Write a log

```luau
--!strict
local fs = require('@dream/fs')

local logs = fs.join(assert(fs.cwd()), 'logs')
assert(fs.mkdir(logs, { recursive = true }))

local log = fs.join(logs, 'run.txt')
assert(fs.writeFile(log, `ran at {os.date '!%Y-%m-%dT%H:%M:%SZ'}\n`, { atomic = true }))

print(fs.fileName(log), fs.exists(log))
```

```text
$ jess run --capability filesystem.read --capability filesystem.write log.jsl
run.txt	true
```

- A function that fails answers `nil`, a message naming the path, and a kind (`notFound`,
  `alreadyExists`, ...) instead of raising, so `assert` is how a script says a failure is fatal,
  and `fs.readFileString(path) or ''` is "use the config if it's there".
- `writeFile(path, data, options)` writes a string or a `buffer`; `atomic` writes beside the file
  and renames it over, `createParents` makes the folders, `append` appends.
- `mkdir` with `recursive` makes missing parents and doesn't mind a folder that exists; `remove`
  takes `recursive` for a folder; `copy`, `rename`, and `stat` do what they say.
- `join(...)`, `parent(path)`, `fileName(path)`, and `extension(path)` handle paths and touch no
  disk.

## TOML

### Read a settings file

`toml.decode(text)`, from the built-in `@jess/data/toml`, turns a TOML document into tables. A settings
loader with defaults, typed all the way:

```luau
--!strict
local toml = require('@jess/data/toml')

type Settings = { radius: number, verbose: boolean, skip: { string }, folder: string }

local function loadSettings(path: string): Settings
  local settings: Settings = { radius = 1.5, verbose = false, skip = {}, folder = '.' }
  local text = require('@dream/fs').readFileString(path)
  if not text then return settings end

  local raw = toml.decode(text)
  if type(raw.radius) == 'number' then settings.radius = raw.radius end
  if type(raw.verbose) == 'boolean' then settings.verbose = raw.verbose end

  if type(raw.skip) == 'table' then
    for _, pattern in raw.skip do
      if type(pattern) == 'string' then table.insert(settings.skip, pattern) end
    end
  end
  local output = raw.output
  if type(output) == 'table' and type(output.folder) == 'string' then settings.folder = output.folder end

  return settings
end

local settings = loadSettings 'settings.toml'
print(settings.radius, settings.verbose, #settings.skip, settings.folder)
```

With this `settings.toml`:

```toml
radius = 1.0
verbose = true
skip = ["^T_", "_unique$"]

[output]
folder = "out"
```

```
1	true	2	out
```

Decoded values could be any TOML type, so check each with `type(...)` before using it. That's also your validation: a value of the wrong type falls back to the default instead of blowing up three functions later. Dates come back as strings, and integers past 2^53 as exact `integer`s.

### Key order and floats

A Luau table forgets what a TOML file says about the order of its keys, whether `1.0` was written as a float, and which strings were datetimes. `decode` returns a second value, the **layout**, that remembers them. For each decoded table (looked up by the table itself), `keys` lists its keys in document order, `floats` marks the values written as floats and `datetimes` the datetimes:

```luau
--!strict
local toml = require('@jess/data/toml')
local settings, layout = toml.decode(require('@dream/fs').readFileString 'settings.toml' or '')

for _, key in layout[settings].keys or {} do
  print(key, layout[settings].floats[key] == true)
end
```

```
radius	true
verbose	false
skip	false
output	false
```

This is how `@dreamwind/tools/s3lightfixes` applies light overrides in the order `lightconfig.toml` lists them.

### Write TOML

`toml.encode` writes a value built with `@jess/data/value`: a `struct` keeps its fields in the order
given, `array` makes a list, and `float` keeps a whole number a float.

```luau
--!strict
local toml = require('@jess/data/toml')
local values = require('@jess/data/value')

local settings = values.struct {
  { 'radius', 1.5 },
  { 'scale', values.float(2) },
  { 'verbose', false },
  { 'skip', values.array { '^T_', '_unique$' } },
  { 'output', values.struct { { 'folder', 'out' } } },
}

assert(require('@dream/fs').writeFile('settings.toml', toml.encode(settings), { atomic = true }))
```

```toml
radius = 1.5
scale = 2.0
verbose = false
skip = [
    "^T_",
    "_unique$",
]

[output]
folder = "out"
```

Values come before tables, as Rust's `toml` crate lays a document out. `@jess/json` and `@jess/yaml`
write the same values.

## Regular expressions

Luau patterns aren't regexes: no alternation, no groups you can name, no `{2,3}`. `bytes.regex(pattern, options)`, from l3i's [`@dream/bytes`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#regular-expressions-bytes-regex), compiles a real one, in Rust's [`regex` syntax](https://docs.rs/regex/latest/regex/#syntax).

```luau
--!strict
local bytes = require('@dream/bytes')
local torch = bytes.regex([[^light_(?<color>[a-z]+)_(\d+)$]], { caseInsensitive = true })

print(torch:isMatch 'Light_Red_128')
local groups = torch:captures 'Light_Red_128'
if groups then print(groups[0], groups.color, groups[2]) end
print(torch:find 'light_blue_64')
print(torch:replace('light_red_64', 'light_${color}_256'))

```

```
true
Light_Red_128	Red	128
0	13
light_red_256	1
```

- Luau's raw strings, `[[\d+]]`, keep backslashes as they are; in a quoted string, `"\\d+"` is the regex `\d+`.
- Positions are 0-based with the end exclusive, as `buffer` offsets are: `find` answers where the match starts and the offset just past it. Text is matched as bytes.
- `captures` returns the first match's groups: `[0]` is the whole match, `[1]` and on the numbered groups, and named groups (`(?<name>...)`) by name as well. Groups that didn't take part are missing.
- `replace` and `replaceAll` return the new text and how many matches they replaced. A string replacement expands `$1`, `${name}`, and `$$`.

The options are `caseInsensitive`, `multiLine` (`^` and `$` match at every line), `dotAll` (`.` matches newlines too), and `unicode = false`, for text in another encoding.

### Every match

```luau
--!strict
local bytes = require('@dream/bytes')
local number = bytes.regex [[\d+]]

for text, start, finish in number:matches 'a1 b22 c333' do
  print(text, start, finish)
end

-- A function replacement gets the groups and returns the new text.
local function double(groups: { [number | string]: string }): string?
  return tostring(tonumber(groups[0]) :: number * 2)
end

local doubled, count = number:replaceAll('radius 64, 128', double)
print(doubled, count)

print(table.concat(bytes.regex([[\s*[,;]\s*]]):split 'a, b;c ,d', '|'))

```

```
1	1	2
22	4	6
333	8	11
radius 128, 256	2
a|b|c|d
```

A replacement function that returns `nil` or `false` keeps that match as it was, and it isn't counted. `replaceAll`'s third argument limits how many it replaces, and `split`'s second how many pieces it makes.

### Lines and literals

```luau
--!strict
local bytes = require('@dream/bytes')
local content = bytes.regex([[^content=(.+)$]], { multiLine = true })
local cfg = 'data=foo\ncontent=Morrowind.esm\ncontent=Tribunal.esm\n'

for line in content:matches(cfg) do
  print(line)
end

-- regexEscape: match text exactly, dots and brackets included.
local literal = bytes.regex(bytes.regexEscape 'a.b (c)')
print(literal:isMatch 'a.b (c)', literal:isMatch 'axb (c)')

```

```
content=Morrowind.esm
content=Tribunal.esm
true	false
```

`matches` yields each whole match and where it is; call `captures(text, start)` with a match's start for its groups. `bytes.regexEscape` quotes text so it matches literally, which is what you want when building a pattern out of a record id or a file name.

## Related

- The [fs](@/docs/api/fs.md), [toml](@/docs/api/toml.md), and [regex](@/docs/api/regex.md) references.
- [Selections](@/docs/scripting/selections.md), whose specs use the same regex syntax.
