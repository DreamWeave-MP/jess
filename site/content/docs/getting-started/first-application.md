---
title: 'Your First Application'
description: 'Build a small application target, watch its lifecycle, and stop it deliberately.'
weight: 25
extra:
  kind: guide
---

The [first package](@/docs/getting-started/first-package.md) runs a command to completion. An application target has a host that advances frames and calls managed modules. This one counts three frames and exits. It needs no window or special capability, so you can see the lifecycle without needing a graphics setup.

Create a folder named `counter` with these files:

```text
counter/
  jess.toml
  src/main.jsl
  src/counter.jsl
```

`jess.toml` describes the application and its managed module:

```toml
manifest_version = 1

[package]
id = "com.example.counter"
version = "0.1.0"
jess_api = "1"
include = ["src"]
default_target = "counter"

[managed.counter]
module = "src/counter.jsl"

[targets.counter]
kind = "application"
entry = "src/main.jsl"
```

The target's entry is `src/main.jsl`:

```luau
--!strict
print 'entry'
```

The managed module, `src/counter.jsl`, returns its handlers:

```luau
--!strict
local application = require '@jess/application'
local frames = 0

return {
  on = {
    host = {
      startup = function() print 'startup' end,
      update = function()
        frames += 1

        if frames == 3 then application.exit(0) end
      end,
      shutdown = function() print(`stopped after {frames} frames`) end,
    },
  },
}
```

From the folder containing `counter`, check and run it:

```sh
jess package check counter
jess package run counter
```

The check reports two files and no errors. The run prints:

```text
entry
startup
stopped after 3 frames
```

Jess loads the managed module, runs the target entry, calls startup, and advances frames. The third update requests exit with status zero; shutdown runs as the application stops. The entry is not the frame loop, and returning from it does not stop the application.

Try changing the count, then run it again. For a bounded diagnostic run, `jess package run counter --frames 2` stops after two frames even though this module requested exit on its third. The `--frames` option is for application targets, not command targets.

This example deliberately has only one managed module. [Applications](@/docs/scripting/applications.md) covers the full startup order, multiple packages, named events, interfaces, and windows. [Capabilities and Trust](@/docs/getting-started/authority.md) explains what changes when an application needs access outside the runtime.
