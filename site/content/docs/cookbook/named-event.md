---
title: 'Send a Named Event'
description: 'Queue an event in an application and handle it without re-entering the sender.'
weight: 40
extra:
  kind: guide
---

An application module can announce that something happened without calling each listener itself. This recipe changes the managed module from [Your First Application](@/docs/getting-started/first-application.md). Keep its manifest and `src/main.jsl`; replace `src/counter.jsl` with:

```luau
--!strict
local application = require '@jess/application'
local events = require '@jess/events'

return {
  on = {
    host = {
      startup = function()
        events.emit('ready', { message = 'hello' })
        print 'queued'
      end,
    },
    event = {
      ready = function(payload: { message: string })
        print(payload.message)
        application.exit(0)
      end,
    },
  },
}
```

Run `jess package check counter`, then `jess package run counter` from the parent folder. It prints:

```text
entry
queued
hello
```

`emit` queues the event and returns; it does not call the handler inside `startup`. The host drains the queue after the frame, and the `ready` handler receives the payload and requests exit. Another managed module in the same domain can declare its own `ready` handler too.

The payload is copied across the event boundary and must contain transferable values. A handler cannot use it as a shared mutable table. Events emitted while a queue is being drained wait for the next drain, so do not assume an event emitted by a handler runs recursively. A plain `jess run` script has no domain event queue; this example needs the application target.

The [events reference](@/docs/api/events.md) covers payloads, ordering, queue limits, and errors. [Applications](@/docs/scripting/applications.md#named-events) explains routing between domains and the difference between named events and host callbacks.
