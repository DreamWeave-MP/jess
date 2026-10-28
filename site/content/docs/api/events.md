---
title: 'jess.events'
description: 'Emit named application events for the handlers a domain bound to a name, delivered at its next event drain.'
weight: 30
extra:
  kind: api
---

{{ api_signature(value="jess.events: { emit }") }}

Also `require('@jess/events')`. Named events are how a domain's code tells the rest of the domain something happened, and, when a host routes them, how one domain tells another. Their handlers are the `on.event` tables of an [application's](@/docs/scripting/applications.md) managed modules. Only a domain's own code emits: under `jess run`, a command target, or in a worker there is no queue to emit into, and `emit` errors.

## jess.events.emit

{{ api_signature(value="jess.events.emit(name: string, payload: unknown?) -> ()") }}

Queues the event `name` with `payload` and returns. No handler runs inside `emit`. At the domain's next event drain, the end of the current frame, every handler bound to `name` gets the payload, in context order; events are delivered in the order they were emitted, and one emitted during a drain waits for the next. A handler that raises is reported with its context and event name, and the rest still run. An event nothing handles is counted as unhandled and dropped.

`payload` must be transferable: nil, booleans, numbers, strings, vectors, buffers, and tables of those, nested at most 32 deep. It is copied out of the emitting code when you emit, so changing your table afterwards changes nothing. Every handler of the event then gets the same table, built once and frozen all the way down: read it, and copy what you want to keep changing. Freezing doesn't reach into a buffer, so a payload that holds one is built for each handler instead, frozen the same way, with buffers of its own: what one handler writes into a buffer, the next never reads. If the host routed `name` to another domain, a copy is posted there too, stamped with that domain's generation.

A name that is also a host callback is still just an event: `emit('update')` reaches `on.event.update` handlers, never `on.host.update`.

The queue is bounded. By default a domain holds at most 65,536 waiting events and 64 MiB of payload, where payload means the storage every value takes, not only its strings and buffers (a list of a million empty tables is not free) and each queued event's name, and one drain delivers at most 4,096 of them, leaving the rest, in order, for the next frame. An event that doesn't fit isn't queued, quietly or otherwise: `emit` raises, the refusal is reported as a diagnostic, and nothing is routed. A payload is read no further than the limit, so a vast table is refused part way in rather than copied and then measured. A handler that answers each event with two more fills the queue and then fails, instead of eating the machine.

- **Errors:** `events.emit: an event needs a name`; `events.emit: an event name is at most 256 bytes, and this one is <length>`; `events.emit: '<name>' wasn't queued: <domain>'s event queue is full: 65536 events are waiting, the most it holds` (or holds at most so many bytes of payload); `payload.<path>: a function can't cross a worker or event boundary` (or a thread, a userdata, a table nested more than 32 deep), also reported as a diagnostic; outside a domain, `events.emit: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one`; in a worker, `events.emit is for a domain's own code; a worker handles messages and answers them`.

```luau
--!strict
local events = require('@jess/events')

local deaths = 0

local function died(player: string)
  deaths += 1
  events.emit('playerDied', { player = player, deaths = deaths })
end

died('ada')
```

## jess.window

One named event is the host's own. Under an application's host, a [window](@/docs/api/gpu.md#windows) something happened to (a resize, a close, a key, the pointer) is announced as `jess.window`, with the payload `{ title: string, key: string? }`: the window's title, and the key it was opened with, if any. The news itself isn't in the payload; it waits in the window, and the handler reads it with `window:events()`. A managed module that keeps its window:

```luau
--!strict
local window = jess.gpu.window(jess.gpu.device(), { title = 'editor', key = 'main' })

return {
  on = {
    event = {
      ['jess.window'] = function(payload: { title: string, key: string? })
        if payload.key ~= window.key then return end

        for _, event in window:events() do
          print(event.kind)
        end
      end,
    },
  },
}
```

A window is announced once until its events are read, at most once a frame, in the frame its news came in, so nothing runs while nothing happens. A handler that leaves a window's news unread hears nothing more of that window until something reads it. Only the host posts the event: a script that runs its own frames with `jess.gpu.pump` reads `events()` when it likes, and nothing is announced to it.

The name isn't reserved. `emit('jess.window', { title = 'not a window' })` is queued and delivered like any other event, to the same handlers. A handler treats the payload as a reason to look, as the one above does, never as the news itself: a window nothing happened to has no events to read.

Related: [Applications](@/docs/scripting/applications.md#named-events), [jess.interfaces](@/docs/api/interfaces.md), [jess.context](@/docs/api/context.md).
