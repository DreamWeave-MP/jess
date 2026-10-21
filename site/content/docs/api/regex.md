---
title: 'Regular expressions'
description: 'Regexes are bytes.regex, from L3i''s @dream/bytes: Rust''s regex crate over bytes, in linear time, with 0-based positions.'
weight: 205
extra:
  kind: api
---

Scripts compile regexes with `bytes.regex(pattern, options)`, from L3i's
[`@dream/bytes`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#regular-expressions-bytes-regex):
Rust's `regex` crate, matched over bytes, in linear time whatever the pattern. jess has no regex
binding of its own. The
[Regular expressions recipes](@/docs/scripting/files-toml-regex.md#regular-expressions) use it.

The one thing to carry in your head is positions. `string.find` counts from 1 with the end
included; `bytes.regex` counts from 0 with the end excluded, as `buffer` offsets do. A whole
match of `'light_blue_64'` is `1, 13` to `string.find` and `0, 13` to `regex:find`, and
`string.sub(text, start + 1, finish)` is the matched text.

```luau
--!strict
local bytes = require '@dream/bytes'

local version = bytes.regex '^(?P<major>\\d+)\\.(?P<minor>\\d+)\\.(?P<patch>\\d+)$'
local captures = version:captures '1.2.1'
if not captures then error 'not a version' end

print(captures.major, captures.minor, captures.patch) -- 1  2  1
```

What it has: `isMatch`, `find`, `match`, `captures`, `matches`, `replace`, `replaceAll`, and
`split`, with `$1` and `${name}` in replacements and a function in their place; `pattern()` and
`tostring(regex)`; `bytes.regexEscape(text)` for a literal; `unicode = false` in the options for
byte-wise classes. The full reference is on [L3i's site](@/docs/api/l3i.md).
