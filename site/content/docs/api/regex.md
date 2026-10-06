---
title: 'jess.regex'
description: 'Byte regexes with string.find-style positions.'
weight: 120
extra:
  kind: api
---

{{ api_signature(value="jess.regex: { new, escape }") }}

Rust regular expressions for scripts, since Luau patterns aren't regexes. Patterns use the [`regex` crate's syntax](https://docs.rs/regex/latest/regex/#syntax): no backreferences or lookaround, and matching always runs in linear time.

{% usage_note(title="Bytes and positions") %}
Text is matched as bytes, so any Luau string works, including legacy-encoded record text. Positions are 1-based byte positions with inclusive ends, like `string.find`'s. Methods that take `init` start looking there; a negative `init` counts from the end, and one past the end starts at the end.
{% end %}

## jess.regex.new

{{ api_signature(value="jess.regex.new(pattern: string, options: RegexOptions?) -> Regex") }}

Compiles a pattern.

- **Parameters:** `pattern`, in `regex` syntax. `options`:
  - `caseInsensitive`: ignore case (like `(?i)`).
  - `multiLine`: `^` and `$` match at line starts and ends, not just the text's (like `(?m)`).
  - `dotAll`: `.` matches `\n` too (like `(?s)`).
- **Errors:** `regex parse error:` with the pattern and what's wrong.

```luau
--!strict
local torch = jess.regex.new('^light_(?P<color>\\w+)_(\\d+)$', { caseInsensitive = true })
print(torch:isMatch 'Light_Red_128') -- true
```

## jess.regex.escape

{{ api_signature(value="jess.regex.escape(text: string) -> string") }}

Returns `text` with every regex metacharacter escaped, to match it literally.

```luau
--!strict
local id = 'a.b*c'
local exact = jess.regex.new('^' .. jess.regex.escape(id) .. '$')
print(jess.regex.escape(id), exact:isMatch 'a.b*c', exact:isMatch 'aXbbc') -- a\.b\*c  true  false
```

## Regex

A compiled regex. Its methods use `:`, and `tostring(regex)` gives its pattern.

### regex.pattern

{{ api_signature(value="regex.pattern: string") }}

The pattern it was compiled from.

### regex:isMatch

{{ api_signature(value="regex:isMatch(text: string) -> boolean") }}

Whether it matches anywhere in `text`.

```luau
--!strict
print(jess.regex.new('^x$', { multiLine = true }):isMatch 'a\nx\nb') -- true
```

### regex:find

{{ api_signature(value="regex:find(text: string, init: number?) -> (number?, number?, string?)") }}

Returns the first match's start, end, and text, or three `nil`s.

```luau
--!strict
local digits = jess.regex.new '\\d+'
print(digits:find 'abc 12 def 345') -- 5  6  12
print(digits:find('abc 12 def 345', 8)) -- 12  14  345
print(digits:find('abc 12 def 345', -3)) -- 12  14  345
```

### regex:match

{{ api_signature(value="regex:match(text: string, init: number?) -> string?") }}

Returns the first match's text, or `nil`.

```luau
--!strict
print(jess.regex.new('\\d+'):match('abc 12 def 345', 7)) -- 345
```

### regex:captures

{{ api_signature(value="regex:captures(text: string, init: number?) -> RegexCaptures?") }}

Returns the first match's groups, or `nil`: `[0]` is the whole match, `[1]` and on the numbered groups, and named groups (`(?P<name>...)`) are there by name too. Groups that didn't take part are missing.

```luau
--!strict
local torch = jess.regex.new('^light_(?P<color>\\w+)_(\\d+)$', { caseInsensitive = true })
local groups = torch:captures 'Light_Red_128'
if groups then
  print(groups[0], groups.color, groups[1], groups[2]) -- Light_Red_128  Red  Red  128
end
```

### regex:matches

{{ api_signature(value="regex:matches(text: string) -> () -> (string?, number?, number?)") }}

An iterator over every match, giving its text, start, and end. An empty match moves on one byte, so the loop always ends.

```luau
--!strict
for text, start, finish in jess.regex.new('\\d+'):matches 'a1b22c333' do
  print(text, start, finish) -- 1 2 2, 22 4 5, 333 7 9
end
```

### regex:replace

{{ api_signature(value="regex:replace(text: string, replacement: string | (RegexCaptures) -> (string | false)?) -> (string, number)") }}

Replaces the first match, returning the new text and how many matches were replaced (0 or 1).

A string replacement expands `$1` and `${name}` to groups, and `$$` to `$`. Write `${1}x` rather than `$1x`, which names a group `1x`. A function gets the match's captures and returns the replacement text; `nil` or `false` keeps the match as it was, and it doesn't count as replaced.

- **Errors:** a replacement that's neither a string nor a function, or a function returning something other than a string, `nil`, or `false`.

```luau
--!strict
print(jess.regex.new('\\d+'):replace('a1b22c333', '#')) -- a#b22c333  1
```

### regex:replaceAll

{{ api_signature(value="regex:replaceAll(text: string, replacement: string | (RegexCaptures) -> (string | false)?, limit: number?) -> (string, number)") }}

Replaces every match, or only the first `limit` replacements (0 or `nil` means all), returning the new text and how many it replaced. Matches a function keeps don't count toward `limit`.

```luau
--!strict
local digits = jess.regex.new '\\d+'
print(digits:replaceAll('a1b22c333', '#')) -- a#b#c#  3
print(digits:replaceAll('a1b22c333', '#', 2)) -- a#b#c333  2
print(
  digits:replaceAll(
    'a1b22c333',
    function(groups) return if groups[0] == '22' then false else '<' .. groups[0] .. '>' end
  )
) -- a<1>b22c<333>  2
local torch = jess.regex.new '^light_(?P<color>\\w+)_(\\d+)$'
print(torch:replaceAll('light_red_64', 'light_${color}_128')) -- light_red_128  1
```

### regex:split

{{ api_signature(value="regex:split(text: string, limit: number?) -> { string }") }}

Splits `text` around the matches, into at most `limit` parts when given (the last part holds the rest).

```luau
--!strict
local comma = jess.regex.new '\\s*,\\s*'
print(table.concat(comma:split 'a , b,c ,d', '|')) -- a|b|c|d
print(table.concat(comma:split('a , b,c ,d', 2), '|')) -- a|b,c ,d
```

Related: [Files, TOML, and Regex](@/docs/scripting/files-toml-regex.md). Selector patterns ([Selection and Selectors](@/docs/api/selection.md)) use the same syntax.
