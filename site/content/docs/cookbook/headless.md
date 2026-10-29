---
title: 'Run a Headless Application'
description: 'Keep an application running without a window and let its shutdown handler finish.'
weight: 45
extra:
  kind: guide
---

An application target has a host that advances frames until the application asks to exit, its frame limit is reached, or the process is interrupted. This one asks for no capabilities and opens no window. It keeps its state in a managed module, which also owns its shutdown handler.

Create `service/jess.toml`:

```toml
manifest_version = 1

[package]
id = "com.example.headless"
version = "0.1.0"
jess_api = "1"
include = ["src"]
default_target = "service"

[managed.service]
module = "src/service.jsl"

[targets.service]
kind = "application"
entry = "src/main.jsl"
```

Create `service/src/main.jsl`:

```luau
--!strict
print 'entry'
```

Create `service/src/service.jsl`:

```luau
--!strict
local frames = 0

return {
  on = {
    host = {
      startup = function() print 'ready' end,
      update = function() frames += 1 end,
      shutdown = function() print(`stopped after {frames} frames`) end,
    },
  },
}
```

Check the package and run three frames:

```sh
jess package check service
jess package run service --frames 3
```

The output is `entry`, `ready`, and `stopped after 3 frames`, each on its own line. Without `--frames`, the application keeps running; Ctrl+C asks the host to run shutdown handlers and exit. On Unix, SIGTERM does the same. The example has no external dependencies, credentials, devices, or persistent state; an actual service must define how it saves state and handles failures. [Deployment and Operations](@/docs/reference/deployment.md) covers supervisors and shutdown, and [Applications](@/docs/scripting/applications.md) describes the lifecycle. The executable fixture is `tests/fixtures/docs/headless`.
