---
title: 'Performance'
description: 'Measure before you optimize: the profiler in the binary, what native code costs, and the work a fast script avoids.'
weight: 60
extra:
  kind: guide
---

A script is fast when it does less work, and you only know which work to drop after you've measured. jess puts the measuring tools in the binary so there's no excuse: a profiler, a disassembler, and the generated assembly are each one flag away from `jess run`. This page is the mechanism. Nothing here is a number, because a number from one machine is a promise about yours, and jess doesn't make those.

## Measure first

`os.clock()` is a wall clock with sub-millisecond resolution, so time spent waiting on a file or a child process counts too. Wrap the part you care about:

```luau
--!strict
local function time(label: string, run: () -> ())
  local started = os.clock()
  run()
  print(string.format('%-32s %8.3f s', label, os.clock() - started))
end

local fs = require '@dream/fs'

time('walk', function() assert(fs.walk('.', { include = 'files' })) end)
```

For where the time goes inside the script, `--profile`:

```luau
--!strict
--!native
local function sumSquares(count: number): number
  local total = 0
  for index = 1, count do
    total += index * index
  end

  return total
end

print(sumSquares(1000000))
```

```text
$ jess run --profile squares.jsl
333333833333127550
jess profile: 71.941 ms script self-time, 1 timed calls, 0 overhead samples (0.000 ms), 528 bytes grown
jess profile: 31251 safepoint samples
   31250  squares.jsl:6
       1  jess:2
jess profile: 1000003 native blocks executed, 1 VM exits
```

Three reports. The first is the script's own time, how many calls the runtime timed, and how much the heap grew. The second is a sampling profile: at every safepoint the VM reaches, the line it was on, so the lines at the top are where the script lives. The third is native code: how many compiled blocks ran and how often execution fell back into the interpreter. A hot loop that shows many VM exits is a loop the native compiler couldn't keep, which is worth knowing before you mark anything `--!native`.

`--disassemble FILE` writes Luau's bytecode for the script, and `--emit-asm FILE` the native assembly, without running it. Both are for the ten minutes when you need to know what the compiler did with your loop, and not for any other time.

## Native code

`--!native` on the line after `--!strict` asks Luau to compile the module to native code, and `@native` before one function compiles just that function. It pays off for code that spends its time in Luau: tight loops over numbers, buffers, and strings. For code that spends its time in native calls, reading files, hashing, decompressing, matching regexes, it changes little, because the time was never in the Luau.

It never changes what code does, but it isn't free: a marked module is compiled to machine code every time it loads, which for a large module that runs once costs more than it will ever save. So mark what a measurement shows is hot and nothing else. jess's own Luau and the built-in package follow that rule, and the [Luau style page](@/docs/contributing/luau-style.md#types) states it.

## The safe environment

jess marks the global environment safe, the way Luau's own sandbox does, so builtins like `math.floor` and `buffer.readi32` are looked up once, when the script loads, and called on Luau's fast path. Without that, every call is a table lookup and a full function call, many times slower in a loop. The price is that globals are read-only: `_G.answer = 42` is `attempt to modify a readonly table`, and a builtin replaced after a script loaded wouldn't reach code already compiled against the original. Don't monkey-patch builtins. Keep state in locals or in a module that returns a table.

## Do less

The profiler tells you where the time is. Most of the time, it is in one of these:

**Reading what you won't use.** Ask for what you need. `@dream/fs`'s `walk` answers parallel arrays, `paths`, `kinds`, and with `metadata` the `sizes`, so a walk of a hundred thousand files makes a handful of tables rather than a table per file. A decoder that takes a buffer and a range reads the range. Read the header, decide, then read the body.

**Copying instead of looking.** A `buffer` read is a number; a table built from a buffer is an allocation per field and a garbage collection later. Read fields straight from the buffer with `@dream/bytes` when you need a field or two, and decode into tables when you need the whole thing more than once.

**Building strings in a loop.** `..` makes a new string every time. Collect pieces in a table and `table.concat` once, or write to a `buffer` and hand the buffer to whatever wanted the bytes.

**Writing more than once.** Build the output, then write it once, atomically. A file written in a hundred pieces is a hundred system calls and a torn file if the script dies in the middle.

**Doing it on one core.** [`jess.worker`](@/docs/api/worker.md) runs a second Luau state beside the first, with its own heap and its own garbage collector, talking in messages. It needs a script domain, so it's for [applications](@/docs/scripting/applications.md) and embedders, not `jess run`; a script that wants parallelism is a script that wants to be a target.

## Related

- [Luau Pitfalls](@/docs/scripting/luau-pitfalls.md): the type checker and runtime surprises that cost time of a different kind.
- [Benchmarks](@/docs/contributing/benchmarks.md): the harness jess's own measurements come from, and why it refuses a build it can't verify.
