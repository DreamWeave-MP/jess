---
title: 'Open a Window'
description: 'Present a static fullscreen pass and handle the window close event.'
weight: 100
extra:
  kind: guide
---

This application shows a solid color until its window closes. It needs a working display and a GPU adapter, including a software adapter if the platform supplies one. It declares `gpu.device` and `gpu.window`; the CLI admits those capabilities when a target declares them. A headless machine without a display cannot run this example, though it can type-check it.

Create `window/jess.toml`:

```toml
manifest_version = 1

[package]
id = "com.example.window"
version = "0.1.0"
jess_api = "1"
include = ["src"]
default_target = "window"

[managed.view]
module = "src/view.jsl"

[targets.window]
kind = "application"
entry = "src/main.jsl"
capabilities = ["gpu.device", "gpu.window"]
```

`window/src/main.jsl` is the target entry:

```luau
--!strict
print 'close the window to stop'
```

`window/src/view.jsl` creates the device, window, and retained plan once:

```luau
--!strict
local application = require '@jess/application'
local shader = require '@jess/shader'
local device = jess.gpu.device()
local window = jess.gpu.window(device, { title = 'Jess example', width = 320, height = 200 })
local format = assert(window.format)

local fill = shader.program {
  vertex = function(input)
    local index = input.builtin.vertexIndex
    local horizontal = shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2)))
    local vertical = shader.float(shader.bitAnd(index, shader.uint(2)))
    return { position = shader.vec4(shader.vec2(horizontal, vertical) * 2 - 1, 0, 1) }
  end,
  fragment = function() return shader.vec4(0.1, 0.2, 0.4, 1) end,
}

local plan = device:plan {
  viewport = { width = window.width, height = window.height },
  targets = { { name = 'screen', format = format, imported = true } },
  passes = {
    { name = 'fill', kind = 'fullscreen', pipeline = device:pipeline(fill, { colors = { format } }), colors = { 'screen' } },
  },
}
window:show(plan, 'screen')

return {
  on = {
    event = {
      ['jess.window'] = function()
        for _, event in window:events() do
          if event.kind == 'closed' then application.exit(0) end
        end
      end,
    },
  },
}
```

From the folder containing `window`, run:

```sh
jess package check window
jess package run window
```

The entry prints `close the window to stop`. The application host advances frames and presents the retained plan. Closing the window produces a `jess.window` event; the handler drains its events and requests application exit. There is no per-frame Luau update in this static scene. For a bounded diagnostic run, add `--frames 3` after the package path.

The fixture is `tests/fixtures/docs/window`; the documentation suite type-checks it without opening a device. This example was also run with a display for three frames. The [application window guide](@/docs/scripting/applications.md#a-window) explains animation and resizing, while the [GPU](@/docs/api/gpu.md) and [shader](@/docs/api/shader.md) references specify the resources and their lifetimes.
