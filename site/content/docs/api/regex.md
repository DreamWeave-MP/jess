---
title: 'Regular expressions'
description: 'Regexes are bytes.regex: what jess.regex was, and what each of its calls is now.'
weight: 120
extra:
  kind: api
---

Scripts compile regexes with `bytes.regex(pattern, options)`, from l3i's
[`@dream/bytes`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#regular-expressions-bytes-regex):
Rust's `regex` crate, matched over bytes, in linear time. jess had a second binding of the same
crate, `jess.regex`, with `string.find`-style positions; it is gone. The
[Regular expressions recipes](@/docs/scripting/files-toml-regex.md#regular-expressions) use
`bytes.regex`.

The one difference to carry over is positions. `jess.regex` counted from 1 with the end
included, as `string.find` does; `bytes.regex` counts from 0 with the end excluded, as `buffer`
offsets do. `'light_blue_64'` matched whole is `1, 13` in the old and `0, 13` in the new.

| `jess.regex` | `bytes.regex` |
| --- | --- |
| `jess.regex.new(pattern, options)` | `bytes.regex(pattern, options)`; `unicode = false` replaces a `(?-u)` prefix |
| `jess.regex.escape(text)` | `bytes.regexEscape(text)` |
| `regex.pattern` | `regex:pattern()`, or `tostring(regex)` |
| `regex:isMatch(text)` | `regex:isMatch(text)` |
| `regex:find(text, init)` | `regex:find(text, offset)`: 0-based start and exclusive end, and no text (`string.sub(text, start + 1, finish)` is it) |
| `regex:match(text, init)`, `regex:captures(text, init)` | `regex:match(text, offset)`, `regex:captures(text, offset)` |
| `regex:matches(text)` | `regex:matches(text)`: each match's text, start, and end, 0-based |
| `regex:replace(...)`, `regex:replaceAll(...)`, `regex:split(...)` | the same, with the same `$1`, `${name}`, and function replacements |

A negative `init`, which counted from the end, has no counterpart: compute the offset.
