---
title: 'jess.worker'
description: 'Spawn isolated Luau workers beside a ScriptDomain and talk to them in messages.'
weight: 160
extra:
  kind: api
---

{{ api_signature(value="jess.worker: { spawn, send }") }}

Also `require('@jess/worker')`. A worker is a second Luau state the [ScriptDomain](@/docs/contributing/script-domains.md) owns, with its own globals, its own modules, and its own garbage collector, able to run while the domain's own code does. It sees nothing of the domain but the messages it is sent. Under `jess run` there is no domain, so `spawn` errors with `worker.spawn: no ScriptDomain owns this runtime`.

## jess.worker.spawn

{{ api_signature(value="jess.worker.spawn(options: WorkerOptions) -> Worker") }}

Spawns a worker and returns its handle. Needs the `worker.spawn` capability.

- `source`: the worker module's Luau source. It is a chunk that returns the message handler, `function(message) ... return reply end`; the handler is called once per message, and the value it returns is the reply (`nil` is no reply).
- `name`: for diagnostics and profiles; by default, where `spawn` was called.
- `capabilities`: the names the worker holds, a subset of the calling context's. By default it inherits them all.
- `shareServices`: `true` gives the worker the domain's services (under a host, its live input and its localization registry), so `jess.l10n` in the worker formats from the application's registry. Off by default: a worker exists to run apart from the domain's state, and its capabilities still say what it may do with what it sees.

The worker's runtime comes from the same plan as the domain's, so a worker under jess has the whole jess API: it can open plugins, read the VFS, and check NIFs, with its own state.

- **Errors:** `unknown worker.spawn option '<key>': expected capabilities, name, shareServices, or source`; `worker.spawn needs source, the worker module's Luau source`; `capability 'worker.spawn' denied to <domain> ...`; `<domain> cannot delegate capabilities it does not hold: <names>`; inside a worker, `worker.spawn is for a domain's own code`.

```luau
--!strict
local worker = require('@jess/worker')

local counter = worker.spawn {
  name = 'counter',
  capabilities = {},
  source = [[
    return function(message)
      local plugin = jess.plugin.open(message.esp, { openmw = false })
      return { records = #plugin }
    end
  ]],
}

counter:send { esp = 'Morrowind.esm' }
local reply = counter:receive(20)
print(reply.records)
counter:join()
```

A worker whose module does not load, does not return a function, or raises in its handler reports a structured failure to the host, attributed to the domain and worker, and (by default) stops; the domain is untouched. A worker's handler runs inside the budget the host gave it.

## Worker

{{ api_signature(value="type Worker = { send, receive, close, join }") }}

### Worker:send

{{ api_signature(value="worker:send(message: unknown) -> ()") }}

Queues a message for the worker's handler. Messages arrive in order.

Only transferable values cross: `nil`, booleans, numbers, integers, strings, vectors, buffers, tables of those with string or number keys, and resource handles. Anything else is an error naming where it was, and nothing is sent.

- **Errors:** `message.<path>: a <type> can't cross a worker or event boundary`; `the worker is stopped; it takes no more messages`.

```luau
--!strict
local worker = require('@jess/worker')

local echo = worker.spawn { source = 'return function(message) return message end' }
echo:send { name = 'Fargoth', bytes = buffer.fromstring('abc'), at = vector.create(1, 2, 3) }
local reply = echo:receive(20)
print(reply.name, buffer.tostring(reply.bytes), reply.at)
```

A buffer arrives as a fresh buffer of the same bytes: writing the copy changes nothing on the other side.

### Worker:receive

{{ api_signature(value="worker:receive(timeoutSeconds: number?) -> unknown") }}

The worker's next reply, waiting up to `timeoutSeconds` for one; without a timeout it does not wait. `nil` when there is none in that time, or when the worker has ended with nothing left to say. A failure the worker reported comes back as an error, once.

- **Errors:** the worker's failure, as `<domain> <generation> <worker> (<module>) failed at <stage>: <message>`.

### Worker:close

{{ api_signature(value="worker:close() -> ()") }}

Asks the worker to stop after the messages already queued for it.

### Worker:join

{{ api_signature(value="worker:join(timeoutSeconds: number?) -> string") }}

Closes the worker and waits for it to end, up to `timeoutSeconds` (without a timeout, as long as it takes), then returns its final state: `stopped` or `failed`. Replies it sent before ending stay readable through `receive`.

- **Errors:** `<worker> did not answer in time`.

## jess.worker.send

{{ api_signature(value="jess.worker.send(message: unknown) -> ()") }}

From inside a worker: sends a message to the domain, besides what the handler returns. For progress, or for more than one reply per message.

- **Errors:** in a domain's own code, `worker.send is for a worker's own code; a domain sends through the handle worker.spawn returned`.

```luau
--!strict
local worker = require('@jess/worker')

return function(count: number)
  for index = 1, count do
    worker.send { progress = index }
  end

  return 'done'
end
```

Related: [jess.context](@/docs/api/context.md), [Script Domains](@/docs/contributing/script-domains.md).
