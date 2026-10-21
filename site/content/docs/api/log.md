---
title: '@jess/log'
description: 'Structured logging for scripts and packages: loggers with context, child loggers, and capture for tests.'
weight: 100
extra:
  kind: api
---

{{ api_signature(value="require('@jess/log'): { new, debug, info, warn, error, capture }") }}

A [built-in module](@/docs/scripting/built-in-modules.md): `require('@jess/log')` is the same frozen table in every script, typed as `Module__jess_log`. A record has a level, a target, a message, and fields (strings, finite numbers, booleans). Records leave the VM through one sink the host owns: `jess` writes them to stderr as one JSON object per line, `debug` ones excepted; `jess test` collects them per case; a Rust host picks its own with `jess::lua::stdlib::set_log_sink`, or, for code running in a [script domain](@/docs/contributing/script-domains.md#diagnostics-and-profiling), gives the domain a log stream (`DomainSpec::logs`) that collects every context's and worker's records with who logged them. Scripts can't reach the sink, only the module.

```luau
--!strict
local log = require '@jess/log'
local logger = log.new('site', { package = 'com.example.site' })
logger.info('rendered', { page = 'docs/index.md', bytes = 4096 })
logger.child({ step = 'sass' }).warn 'brand.sass has no tokens'
```

```text
{"level":"info","target":"site","message":"rendered","fields":{"bytes":4096.0,"package":"com.example.site","page":"docs/index.md"},"source":"build.jsl"}
{"level":"warn","target":"site","message":"brand.sass has no tokens","fields":{"package":"com.example.site","step":"sass"},"source":"build.jsl"}
```

`source` is the chunk that logged, as the VM sees it, not something the script said.

### log.new

{{ api_signature(value="log.new(target: string, fields: JessLogFields?) -> JessLogger") }}

A logger for `target`, carrying `fields` in every record it writes. The table is copied and frozen; changing it afterwards changes nothing.

### JessLogger.debug, info, warn, error

{{ api_signature(value="logger.info(message: string, fields: JessLogFields?) -> ()") }}

Writes one record at that level. `fields` are merged over the logger's, the call's winning. A field that isn't a string, a finite number, or a boolean is an error, as is a message that isn't a string.

### JessLogger.child

{{ api_signature(value="logger.child(fields: JessLogFields) -> JessLogger") }}

A logger with the same target and more context.

### log.debug, info, warn, error

The module's own functions log with the target `jess`, for a script that doesn't want a logger of its own.

### log.capture

{{ api_signature(value="log.capture(body: () -> ()) -> { JessLogRecord }") }}

Runs `body` and returns what was logged while it ran, on this thread, as frozen records: the sink never sees them. Captures nest, and an outer capture holds what inner ones did. An error in `body` is raised again unchanged, once the capture is closed. `@jess/test` runs every case inside one, which is how a case's `logs` get into its result.

```luau
--!strict
local log = require '@jess/log'
local records = log.capture(function() log.new('a').warn('careful', { count = 2 }) end)
print(#records, records[1].level, records[1].target, records[1].fields.count)
-- 1  warn  a  2
```

## Types

`types/jess.d.jsl` declares these (see [Built-in Modules](@/docs/scripting/built-in-modules.md) for how the modules get typed):

```luau
type JessLogFields = { [string]: string | number | boolean }
type JessLogger = {
  debug: (message: string, fields: JessLogFields?) -> (),
  info: (message: string, fields: JessLogFields?) -> (),
  warn: (message: string, fields: JessLogFields?) -> (),
  error: (message: string, fields: JessLogFields?) -> (),
  child: (fields: JessLogFields) -> JessLogger,
}
type JessLogRecord = { level: string, target: string, message: string, fields: JessLogFields }
```
