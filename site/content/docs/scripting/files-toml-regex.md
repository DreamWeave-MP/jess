---
title: 'Files, TOML, and Regex'
description: 'The standard-library gaps jess fills: jess.fs, jess.toml, jess.regex.'
weight: 90
extra:
  kind: guide
---

Luau on its own can't open a file, read a config, or match a regex. Scripts that write logs, read settings, or match record ids need all three, so jess has them: `jess.fs`, `jess.toml`, and `jess.regex`. This page is recipes; the [fs](@/docs/api/fs.md), [toml](@/docs/api/toml.md), and [regex](@/docs/api/regex.md) references have every function.

## Files

`jess.fs` is plain file access with host paths: the paths your operating system uses, not VFS paths. (For files the game sees, use [`openmw:readFile`](@/docs/scripting/openmw-and-vfs.md#content-and-files).)

### Walk a folder

There's no recursive listing built in, but it's six lines:

```luau
--!strict
-- Every file under a folder, recursively, as paths.
local function walk(folder: string, found: { string }?): { string }
  local files = found or {}
  for _, name in jess.fs.list(folder) do
    local path = jess.fs.join(folder, name)
    if jess.fs.isDir(path) then
      walk(path, files)
    else
      table.insert(files, path)
    end
  end
  return files
end

local root = ... or '.'
local files = walk(root)

local total = 0
for _, path in files do
  total += jess.fs.size(path) or 0
end
print(`{#files} files, {total} bytes under {root}`)
```

`list` returns entry names in whatever order the filesystem gives them; sort them if order matters. `join` puts paths together with the right separator.

### Write a log

```luau
--!strict
local logs = jess.fs.join(jess.fs.cwd(), 'logs')
jess.fs.createDir(logs)

local log = jess.fs.join(logs, 'run.txt')
jess.fs.write(log, `ran at {os.date '!%Y-%m-%dT%H:%M:%SZ'}\n`)

print(jess.fs.fileName(log), jess.fs.exists(log))
```

- `read(path)` returns a file's contents as a string, or `nil` when there's no such file. That makes "use the config if it's there" a one-liner: `jess.fs.read(path) or ""`.
- `write(path, data)` writes a string or a `buffer`, replacing the file. There's no append: read, concatenate, write.
- `createDir` creates missing parents too, and doesn't mind if the folder exists.
- `remove(path)` returns whether there was a file to remove; `copy(from, to)` copies one.
- `exists`, `isDir`, `isFile`, and `size` ask about a path; `size` is `nil` for a missing file.
- `cwd()`, `join(...)`, `fileName(path)`, and `parent(path)` handle paths.

Errors name the path, so a failed write tells you which one.

## TOML

### Read a settings file

`jess.toml.decode(text)` turns a TOML document into tables. A settings loader with defaults, typed all the way:

```luau
--!strict
type Settings = { radius: number, verbose: boolean, skip: { string }, folder: string }

local function loadSettings(path: string): Settings
  local settings: Settings = { radius = 1.5, verbose = false, skip = {}, folder = '.' }
  local text = jess.fs.read(path)
  if not text then return settings end

  local raw = jess.toml.decode(text)
  if type(raw.radius) == 'number' then settings.radius = raw.radius end
  if type(raw.verbose) == 'boolean' then settings.verbose = raw.verbose end

  if type(raw.skip) == 'table' then
    for _, pattern in raw.skip :: { TomlValue } do
      if type(pattern) == 'string' then table.insert(settings.skip, pattern) end
    end
  end
  local output = raw.output
  if type(output) == 'table' and type((output :: TomlTable).folder) == 'string' then
    settings.folder = (output :: TomlTable).folder :: string
  end

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

Decoded values are `TomlValue`s, which could be any TOML type, so check each with `type(...)` before using it. That's also your validation: a value of the wrong type falls back to the default instead of blowing up three functions later. Dates come back as strings, and integers past 2^53 lose precision, like any Luau number.

### Key order and floats

A Luau table forgets two things a TOML file says: the order of its keys, and whether `1.0` was written as a float. `decode` returns a second value, the **layout**, that remembers them. For each decoded table (looked up by the table itself), `keys` lists its keys in document order and `floats` marks the values written as floats:

```luau
--!strict
local settings, layout = jess.toml.decode(jess.fs.read 'settings.toml' or '')

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

`jess.toml` only decodes. To write a document, hand plain data to `vfstool.serialize`, one of the [downstream libraries](@/docs/scripting/downstream-libraries.md):

```luau
--!strict
local settings = {
  radius = 1.5,
  verbose = false,
  skip = { '^T_', '_unique$' },
  output = { folder = 'out' },
}

jess.fs.write('settings.toml', vfstool.serialize(settings, 'toml'))
```

```toml
skip = [
    "^T_",
    "_unique$",
]
radius = 1.5
verbose = false

[output]
folder = "out"
```

It picks the key order, not you, and writes whole numbers as integers (`1`, not `1.0`). When the exact output matters, as it does for a file people edit by hand, build the text yourself. The same function writes `"json"` and `"yaml"`.

## Regular expressions

Luau patterns aren't regexes: no alternation, no groups you can name, no `{2,3}`. `jess.regex.new(pattern, options)` compiles a real one, in Rust's [`regex` syntax](https://docs.rs/regex/latest/regex/#syntax). It's the same engine and syntax as selection specs and S3LightFixes' patterns.

```luau
--!strict
local torch = jess.regex.new('^light_(?P<color>[a-z]+)_(\\d+)$', { caseInsensitive = true })

print(torch:isMatch 'Light_Red_128')
local groups = torch:captures 'Light_Red_128'
if groups then print(groups[0], groups.color, groups[2]) end
print(torch:find 'light_blue_64')
print(torch:replace('light_red_64', 'light_${color}_256'))
```

```
true
Light_Red_128	Red	128
1	13	light_blue_64
light_red_256	1
```

- Backslashes are doubled inside Luau strings: `"\\d+"` is the regex `\d+`. Luau's raw strings, `[[\d+]]`, avoid that.
- Positions are 1-based with inclusive ends, like `string.find`'s. Text is matched as bytes.
- `captures` returns the first match's groups: `[0]` is the whole match, `[1]` and on the numbered groups, and named groups (`(?P<name>...)`) by name as well. Groups that didn't take part are missing.
- `replace` and `replaceAll` return the new text and how many matches they replaced. A string replacement expands `$1`, `${name}`, and `$$`.

The options are `caseInsensitive`, `multiLine` (`^` and `$` match at every line), and `dotAll` (`.` matches newlines too).

### Every match

```luau
--!strict
local number = jess.regex.new '\\d+'

for text, start, finish in number:matches 'a1 b22 c333' do
  print(text, start, finish)
end

-- A function replacement gets the captures and returns the new text.
local doubled, count = number:replaceAll(
  'radius 64, 128',
  function(captures: RegexCaptures): string? return tostring(tonumber(captures[0]) :: number * 2) end
)
print(doubled, count)

print(table.concat(jess.regex.new('\\s*[,;]\\s*'):split 'a, b;c ,d', '|'))
```

```
1	2	2
22	5	6
333	9	11
radius 128, 256	2
a|b|c|d
```

A replacement function that returns `nil` or `false` keeps that match as it was, and it isn't counted. `replaceAll`'s third argument limits how many it replaces, and `split`'s second how many pieces it makes.

### Lines and literals

```luau
--!strict
local content = jess.regex.new('^content=(.+)$', { multiLine = true })
local cfg = 'data=foo\ncontent=Morrowind.esm\ncontent=Tribunal.esm\n'

for line in content:matches(cfg) do
  print(line)
end

-- escape: match text exactly, dots and brackets included.
local literal = jess.regex.new(jess.regex.escape 'a.b (c)')
print(literal:isMatch 'a.b (c)', literal:isMatch 'axb (c)')
```

```
content=Morrowind.esm
content=Tribunal.esm
true	false
```

`matches` yields each whole match; call `captures(text, start)` on a match's start position for its groups. `jess.regex.escape` quotes text so it matches literally, which is what you want when building a pattern out of a record id or a file name.

## Related

- The [fs](@/docs/api/fs.md), [toml](@/docs/api/toml.md), and [regex](@/docs/api/regex.md) references.
- [Selections](@/docs/scripting/selections.md), whose specs use the same regex syntax.
