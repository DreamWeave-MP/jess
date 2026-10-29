---
title: 'Do Work in a Worker'
description: 'Send work to an isolated Luau state and receive the reply without blocking an application frame.'
weight: 60
extra:
  kind: guide
---

This application sends a number to a worker and checks for the reply during later frames. The worker has its own Luau state. The application stays responsive because `receive()` without a timeout does not wait for a reply.

Create `worker/jess.toml`:

```toml
manifest_version = 1

[package]
id = "com.example.worker"
version = "0.1.0"
jess_api = "1"
include = ["src"]
default_target = "worker"

[managed.main]
module = "src/worker.jsl"

[targets.worker]
kind = "application"
entry = "src/main.jsl"
capabilities = ["worker.spawn"]
```

The entry `worker/src/main.jsl` is small:

```luau
--!strict
print 'entry'
```

The managed module `worker/src/worker.jsl` owns the worker handle:

```luau
--!strict
local application = require '@jess/application'
local workers = require '@jess/worker'
local worker: Worker?

return {
  on = {
    host = {
      startup = function()
        worker = workers.spawn { source = 'return function(value) return value * 2 end' }
        worker:send(21)
        print 'queued'
      end,
      update = function()
        if not worker then return end

        local reply = worker:receive()
        if reply == nil then return end
        if type(reply) ~= 'number' then error('unexpected reply', 0) end

        print(`result: {reply}`)
        worker:close()
        application.exit(0)
      end,
    },
  },
}
```

Check it and run the target with the declared capability:

```sh
jess package check worker
jess package run --capability worker.spawn worker
```

It prints `entry`, `queued`, and `result: 42`, in that order. The worker receives a copy of the message and returns a reply; it does not share the module's local variables. `close` asks it to stop after queued messages. Domain teardown also owns worker cleanup.

The example keeps polling only while it has work outstanding. A real application needs a policy for worker failure, cancellation, and work that never completes. During development, `--frames` can bound a run. The worker source is a string here to keep the example small; it is compiled when spawned. [jess.worker](@/docs/api/worker.md) specifies transferable values, queue limits, errors, and optional service sharing. The fixture is `tests/fixtures/docs/worker` in the Jess repository.
