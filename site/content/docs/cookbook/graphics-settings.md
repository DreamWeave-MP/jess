---
title: 'Graphics Settings'
description: 'Change an application frame limit and its window vsync while it runs, and read its frame statistics.'
weight: 105
extra:
  kind: guide
---

A settings menu's two graphics options, without the menu: the frame limit is the application's pace, and vsync is its window's. Both change while the application runs, without restarting it or making its window again. Like [Open a window](@/docs/cookbook/window.md), it needs a display and a GPU adapter.

Create `settings/jess.toml`. The manifest's `pace` is where the application starts, the display's refresh:

```toml
manifest_version = 1

[package]
id = "com.example.settings"
version = "0.1.0"
jess_api = "1"
include = ["src"]
default_target = "settings"

[managed.settings]
module = "src/settings.jsl"

[targets.settings]
kind = "application"
entry = "src/main.jsl"
capabilities = ["gpu.device", "gpu.window"]
pace = "display"
```

`settings/src/main.jsl` says which key does what:

```luau
--!strict
print 'V: vsync, 1: display, 2: 30 a second, 3: 120 a second, 4: unpaced, S: stats'
```

`settings/src/settings.jsl` draws a fullscreen pass whose color changes every frame, so every frame gives the window something to draw, and answers the keys:

```luau
--!strict
local application = require '@jess/application'
local render = require '@jess/render'
local shader = require '@jess/shader'

local program = shader.program {
  pass = { shade = shader.float },
  vertex = function(input)
    local index = input.builtin.vertexIndex
    local horizontal = shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2)))
    local vertical = shader.float(shader.bitAnd(index, shader.uint(2)))
    return { position = shader.vec4(shader.vec2(horizontal, vertical) * 2 - 1, 0, 1) }
  end,
  fragment = function(input) return shader.vec4(input.pass.shade, 0.2, 0.4, 1) end,
}

local device = jess.gpu.device()
local window = jess.gpu.window(device, { title = 'graphics settings', width = 320, height = 200 })
local format = assert(window.format)
local pipeline = device:pipeline(program, { colors = { format } })
local shading = render.block(program.layout, 'pass')
local shadingBuffer = device:buffer(shading.size, 'uniform')

window:show(
  device:plan {
    viewport = { width = window.width, height = window.height },
    targets = { { name = 'screen', format = format, imported = true } },
    passes = {
      {
        name = 'fill',
        kind = 'fullscreen',
        pipeline = pipeline,
        pass = pipeline:bind('pass', { [0] = shadingBuffer }),
        colors = { 'screen' },
      },
    },
  },
  'screen'
)

local Paces: { [string]: ApplicationPace } = {
  Digit1 = 'display',
  Digit2 = 30,
  Digit3 = 120,
  Digit4 = 'unpaced',
}

local function report()
  local pacing, stats = application.pace(), application.frameStats()
  local held = if pacing.override == nil then '' else `, held at {pacing.override}`
  print(string.format(
    'pace %s (asked %s%s), vsync %s, %.0f frames a second, work %.2f ms',
    tostring(pacing.effective),
    tostring(pacing.requested),
    held,
    tostring(window:vsync()),
    stats.rate,
    stats.work * 1000
  ))
end

local elapsed = 0

return {
  on = {
    host = {
      update = function(deltaSeconds: number)
        elapsed += deltaSeconds
        shading:set('shade', 0.5 + 0.5 * math.sin(elapsed))
        shading:upload(shadingBuffer)
      end,

      shutdown = report,
    },

    event = {
      ['jess.window'] = function()
        for _, event in window:events() do
          if event.kind == 'closed' then
            application.exit(0)
          elseif event.kind == 'key' and event.pressed and not event.repeated then
            local pace = Paces[event.key]
            if pace then
              application.setPace(pace)
            elseif event.key == 'KeyV' then
              window:vsync(not window:vsync())
            elseif event.key == 'KeyS' then
              report()
            end
          end
        end
      end,
    },
  },
}
```

From the folder containing `settings`, run:

```sh
jess package check settings
jess package run settings
```

On a 120 Hz display, three seconds of it and then closing the window prints:

```text
V: vsync, 1: display, 2: 30 a second, 3: 120 a second, 4: unpaced, S: stats
pace display (asked display), vsync true, 120 frames a second, work 0.22 ms
```

What each key is doing:

- **`1` to `4` ask for a pace** with `application.setPace`. It takes effect at the end of the frame that asks: the next frame's deadline is counted from when the last one began, and no time is lost or invented, so `update` and the fixed step carry on as if nothing happened. `S` then reports `30 frames a second` for `2`, and so on.
- **`V` toggles the window's vsync** with `window:vsync`. The window's surface is reconfigured before its next image; the window stays where it is. Turning vsync off also changes what display pacing means, since the window no longer waits on its display: `pace` reports `effective` as the display's rate, on a timer, while `requested` is still `display`. A surface that can only wait (some compositors offer nothing else) keeps waiting, and `window:vsync(false)` returns `true` to say so.
- **`--pace` wins.** `jess package run settings --pace 60` holds the application at 60 frames a second whatever the keys ask, and `S` shows the request beside it: `pace 60 (asked display, held at 60)`. The request is kept, and is what paces again if a host lifts the override.

`application.frameStats` reads what the host recorded and waits on nothing, so calling it every frame costs a table. Its `interpolation` is how far the domain's time is into the next fixed step, for drawing between the last two. [Frames and steps](@/docs/scripting/applications.md#frames-and-steps) has the whole policy.

The fixture is `tests/fixtures/docs/graphics-settings`; the documentation suite type-checks it without opening a device, and the same calls are covered by the integration tests. It was run with a display.
