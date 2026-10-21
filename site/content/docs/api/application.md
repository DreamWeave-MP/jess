---
title: 'jess.application'
description: 'Ask the host running an application target to stop, with an exit status.'
weight: 40
extra:
  kind: api
---

{{ api_signature(value="jess.application: { exit }") }}

Also `require('@jess/application')`. What an [application's](@/docs/scripting/applications.md#running-and-stopping) code asks of the host running it. A script under `jess run` or a command target ends when it returns, so there is nothing to ask, and `exit` errors.

## jess.application.exit

{{ api_signature(value="jess.application.exit(status: number?) -> ()") }}

Asks the host to stop after the current frame, with `status` (0 without one) as the process's exit status. It returns; the frame finishes, then the `shutdown` handlers run and the application is torn down. A later call in the same frame replaces the status. A restart forgets the request.

- **Errors:** `application.exit: the status is an integer from 0 to 255, not <status>`; `application.exit: needs an application host; jess run, jess test, and command targets end when their script returns`; in a worker, `application.exit is for the domain's own code; a worker asks the domain in a message`.

```luau
--!strict
local application = require('@jess/application')

local frames = 0

local function update(deltaSeconds: number)
  frames += 1

  if frames == 600 then application.exit(0) end
end

update(1 / 60)
```

Related: [Applications](@/docs/scripting/applications.md), [jess package run](@/docs/cli/package.md#jess-package-run).
