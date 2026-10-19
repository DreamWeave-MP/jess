---
title: 'jess.context'
description: 'Bind handlers to the phases a ScriptDomain schedules for a context.'
weight: 150
extra:
  kind: api
---

{{ api_signature(value="jess.context: { bind }") }}

Also `require('@jess/context')`. This is the script side of a [ScriptDomain](@/docs/contributing/script-domains.md): a host that embeds jess declares contexts, each with the phases it may run in, and a script binds its handlers to them. Under `jess run` there is no domain, so `bind` errors with `context.bind: needs a ScriptDomain, and none owns this runtime; jess run, jess test, and command targets run scripts outside one`; the examples here are a domain's main script.

## jess.context.bind

{{ api_signature(value="jess.context.bind(name: string, handlers: ContextHandlers) -> ()") }}

Binds one handler per phase to the context the host declared as `name`. A later `bind` for the same phase replaces the earlier handler; other phases keep theirs.

`handlers` is keyed by phase name. Each handler gets the phase's payload: seconds for `update`, `fixedUpdate`, and `lateUpdate`; the event for `event`; whatever the host passed for `input`, `renderPrepare`, `message`, `timer`, and `onDemand`; nothing for `load`, `startup`, and `shutdown`.

| Phase | When |
| --- | --- |
| `load` | Once, after the domain's code has loaded. |
| `startup` | Once, when the domain starts. |
| `fixedUpdate` | Zero or more fixed steps per advance, bounded by the host's catch-up limit. |
| `update`, `lateUpdate` | Once per advance. |
| `input`, `renderPrepare`, `message`, `timer`, `onDemand` | When the host or a layer above asks. |
| `event` | When an event queued for the context is delivered. |
| `shutdown` | Once, when the domain stops or restarts. |

Binding never widens authority. The host's declaration says which phases a context has and which capabilities it holds; a script can only fill in the handlers.

- **Errors:** `<domain> has no context '<name>'`; `<context> does not schedule the <phase> phase`; `'<key>' is not a scheduler phase`; `context.bind: the <phase> handler is a function, not <type>`.

```luau
--!strict
local context = require('@jess/context')

local ticks = 0

context.bind('game', {
  startup = function()
    print('starting')
  end,

  update = function(deltaSeconds: number)
    ticks += 1
  end,

  event = function(payload)
    print(`event {payload.kind}`)
  end,

  shutdown = function()
    print(`stopping after {ticks} ticks`)
  end,
})
```

A handler that raises is reported to the host with the domain, generation, context, and phase it ran in, and where it was bound; the phase goes on to the next context, and the domain keeps running.

Related: [jess.worker](@/docs/api/worker.md), [Script Domains](@/docs/contributing/script-domains.md).
