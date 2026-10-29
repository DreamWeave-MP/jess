---
title: 'Applications'
description: 'Application targets: managed modules loaded into one ScriptDomain, host callbacks, named events, domain-local interfaces that later packages override on purpose, and windows.'
weight: 50
extra:
  kind: guide
---

A command target is a script with a package around it: it runs, it returns, it's done. An **application target** keeps running. jess builds it a host of its own, loads the managed modules of every package in its graph into one [ScriptDomain](@/docs/contributing/script-domains.md), and drives frames until the application says it's finished.

A game, an editor, and a launcher are all "some packages, some callbacks, some frames"; what the callbacks mean is the packages' business.

## The manifest

Two tables, both already in the manifest format you know:

```toml
[managed.inventory]
module = "src/inventory.jsl"
interface = "game.inventory"

[targets.game]
kind = "application"
entry = "src/main.jsl"
```

`[managed.<id>]` can sit in any package, not only the one with the target. Every package the target's graph selects gets its managed modules loaded. `module` is a private file: it doesn't need to be in `[modules]`, and other packages can't `require` it. Its own package's files can, relatively, and they get the instance the loader ran: a managed module loads once per domain, like any package module. `interface` is optional. Leave it out and the module is handler-only.

`[targets.<id>]` with `kind = "application"` names the entry and declares capabilities, exactly like a command target.

## A managed module

A managed module returns a table. That table is the whole contract: no globals, no registration calls.

```luau
--!strict
local journal = require('@example/journal')

local inventory = {
  getArmorType = function(item: string): string return 'light' end,
  getWeight = function(item: string): number return 30 end,
}

return {
  on = {
    host = {
      startup = function() journal.write('inventory startup') end,
      update = function(deltaSeconds: number) end,
    },

    event = {
      playerDied = function(payload: { player: string })
        journal.write(`inventory drops {payload.player}'s loot`)
      end,
    },
  },

  interface = inventory,
}
```

`on.host` holds the callbacks the host calls. These names, and no others:

| Callback | Called | With |
| --- | --- | --- |
| `load` | once, after the application's entry has run | nothing |
| `startup` | once, when the application starts | nothing |
| `fixedUpdate` | zero or more times a frame, one fixed step each | the step in seconds |
| `update`, `lateUpdate` | once a frame | the frame's seconds |
| `input`, `renderPrepare`, `onDemand` | when the host asks | whatever the host passes |
| `shutdown` | once, when the application stops or restarts | nothing |
| `interfaceOverride` | once, by the loader, if this module's interface replaces another | the table it replaces |

`on.event` holds handlers for **named events**, keyed by event name. Names are case-sensitive strings of up to 256 bytes; dots and anything else go in quoted keys, `['game.playerDied'] = function(payload) ... end`.

`interface` is a plain table, and it is required exactly when the manifest declares an `interface`.

Everything is checked when the module returns, before anything is bound, and a mistake stops the launch naming the package, the managed id, the file, and the key:

```text
error: org.example.app target app failed to load:
error in org.example.app:app during load: managed 'main' of org.example.app (org.example.app@1.0.0/src/main.jsl): on.host.updat is not a host callback; expected load, startup, fixedUpdate, update, lateUpdate, input, renderPrepare, onDemand, shutdown, interfaceOverride
```

There is no "unknown key, probably fine". `on.host.event` is refused, because raw events are [`@jess/context`](@/docs/api/context.md)'s and named ones go in `on.event`. `message` and `timer` are refused because nothing in the kernel dispatches those phases yet, and a handler that never runs is worse than an error. A handler that isn't a function, an `on.event` keyed by a number, an interface the manifest doesn't declare, a declared interface the module didn't return, and an `interfaceOverride` with no interface to override all fail the same way.

Each managed module gets a context of its own, named `<package>/<managed id>`, scheduling exactly the phases it declared, plus the event phase when it has named events. Two modules that both declare `update` are two contexts with one handler each; nobody's handler replaces anybody's.

## How an application starts

`jess package run`, `jess workspace run`, and `jess::lua::package::launch` all do this, in this order:

1. **Check the grant.** A capability the target declares and you didn't grant stops it here, before anything is built.
2. **Build the host.** One `Host`, one domain, holding exactly the target's capabilities.
3. **Resolve, admit, mount, activate.** Inside the domain's load, the target's package resolves with its dependencies (lock pins and package paths as for any target), every layer it declares is planned, and every entrypoint's capabilities are checked against what the target declares; only then is the plan mounted in the domain's one primary runtime. Up to here no package code has run. Then the layers' entrypoints run, as for any activation.
4. **Load the managed modules.** Packages in dependency order (a package after everything it depends on; among packages free to go next, the smallest id first), each package's modules by managed id. Each module runs, its return value is checked, its context is declared, and its handlers are bound.
5. **Compose the interfaces**, in that same order (below), and publish them all at once.
6. **Run the entry**, the target's private `entry`, with the target's arguments as `...` and `arg`. It runs inside the domain, after the interfaces exist, so it can look them up. What it returns is ignored: an application's status comes from `application.exit`.
7. **`load` handlers**, then **`startup` handlers**, then frames.

The order is the plan's, never the order you listed packages in or the filesystem listed them. A failure in steps 3 to 7 stops the launch, listing every failure the load reported, each by the module's context (or the application's, for a module that wouldn't load) and where its handler was defined; a failure in a `startup`, `update`, or event handler is reported and the application runs on, as any domain does.

## Interfaces

An interface is a plain Luau table, published under an id, for the code in one domain:

```luau
--!strict
local interfaces = require('@jess/interfaces')

local inventory = interfaces.get('game.inventory')
print(inventory.getArmorType('cuirass'))
```

`interfaces.get` raises for an id nobody provides, naming what the domain does have and where you asked; `interfaces.tryGet` returns `nil` for an interface that is genuinely optional. Both work once composition is done: in the entry, in any handler. Asking while modules are still loading is an error that says so, because a half-built registry is a lie.

Interfaces are **domain-wide on purpose**. Any code in the domain can look up any interface; that's what they're for. It isn't a hole in [`require`'s rules](@/docs/scripting/packages.md#modules-and-require): those still decide which modules a package can load, and an interface only exposes the table its provider chose to return. They are also **domain-local, full stop**. They are tables in one Luau VM. A worker has none, another domain has its own, and nothing is proxied, copied, or serialized. Domains talk in events.

### Overrides

When several managed modules provide the same id, the later one in load order wins. Load order is dependency order, so the package that depends on another is the one that overrides it, which is almost always what you meant. Two providers that don't depend on each other are ordered by package id; if that isn't what you want, make one depend on the other and say so in the manifest.

Notice what that means: position in the load order is the whole of the authority to override. Any package in the graph that declares the same id and loads later replaces the earlier provider, with no say from it and no `interfaceOverride` needed. The graph is yours, and so is reading what it claims: `jess package inspect` lists every `[managed]` interface a package provides.

Winning replaces the whole table. jess merges nothing. A provider that wants to keep some of what it replaces says which parts, in `interfaceOverride`:

```luau
--!strict
type Inventory = {
  getArmorType: (item: string) -> string,
  getWeight: (item: string) -> number,
}

local armor: { getArmorType: (item: string) -> string, getWeight: ((item: string) -> number)? } = {
  getArmorType = function(item: string): string return 'heavy' end,
}

return {
  on = {
    host = {
      interfaceOverride = function(previous: Inventory)
        armor.getWeight = previous.getWeight
      end,
    },
  },

  interface = armor,
}
```

The rules:

- `interfaceOverride` runs once per replacing provider, immediately before its table is published, with the **effective** previous table: if three packages chain, the third sees what the second kept.
- The first provider of an id has nothing to replace, and its `interfaceOverride` is never called.
- Without `interfaceOverride`, the previous table is simply gone. Copy, wrap, or drop members as you like; it's your table.
- An `interfaceOverride` that raises stops the launch, naming the provider and the id. Nothing is published, so nothing can see a half-overridden interface.
- Once a provider's turn is done, its table is frozen, like a package export. A consumer can't monkeypatch an interface for the whole domain, and neither can the next provider: it gets `previous` to read, not to edit.

Overrides happen once, at load. Swapping an interface while the application runs is not a thing yet.

## Named events

```luau
--!strict
local events = require('@jess/events')

events.emit('playerDied', { player = 'Rowan' })
```

`emit` queues the event and returns. No handler runs inside `emit`, so emitting from a handler never re-enters one. The domain's next event drain, at the end of the current frame, delivers it to every handler bound to that name, in context order (load order). Events are delivered in the order they were emitted. One emitted while the queue drains waits for the next drain, which means the next frame. A handler that raises is reported, attributed to its context and event, and the next handler runs anyway. An event nobody handles is counted in the frame's report and dropped; nothing waits for a listener that will never come.

The queue is bounded: 65,536 waiting events and 64 MiB of payload, counting the storage of every value, not only strings and buffers, with at most 4,096 delivered per drain and the rest left, in order, for the next frame. An event that doesn't fit makes `emit` raise, and is reported; nothing is dropped behind your back, and a handler that answers every event with two more hits the wall in a few frames instead of taking the machine down with it.

The payload has to be **transferable**: nil, booleans, numbers, strings, vectors, buffers, and tables of those, nested at most 32 deep. No functions, no userdata, no threads. It's copied when you emit, and every handler of the event gets the same frozen table, or, when it holds a buffer, a frozen table and buffers of its own: one handler can't change what the next one reads. The same rules hold whether the event stays in the domain or crosses to another one, so an event that works locally doesn't break the day a host routes it somewhere. A payload that can't cross raises, naming the path to the problem:

```text
payload.onDeath: a function can't cross a worker or event boundary
```

A table that contains itself is refused the same way, as too deep, with a path that repeats until you notice.

Script events and host callbacks never meet. `events.emit('update')` delivers to `on.event.update` handlers if there are any, and never runs an `on.host.update`. `events.emit('interfaceOverride')` is just an event with a silly name. A script can't enter a host phase at all; the host owns the scheduler.

One named event comes from the host rather than from a script: [`jess.window`](@/docs/api/events.md#jess-window), posted when something happened to one of the application's windows. It is delivered like any other, at the frame's drain, to `on.event['jess.window']` handlers; [A window](#a-window) handles one.

### Between domains

Each domain has its own queue, and nothing is broadcast. A host that wants one domain's events in another routes them, explicitly, by name:

```rust
// Every `playerDied` the game emits is also posted to the server, as its payload.
game.domain().route("playerDied", server.domain().mailbox()?)?;
```

A native host can also post a named event into any domain from any thread with `DomainMailbox::post_named`. Either way it is stamped with the recipient's generation, so a restart drops what was queued for the old code. Interfaces never cross; only the event's payload does. A routed copy leaves when the event is emitted, not when the emitter's frame or load finishes: one emitted during a load that later fails has already been posted.

## Running and stopping

```sh
jess package run game --package-path packages                       # until it stops itself
jess package run game --package-path packages --frames 600          # or after 600 frames
jess workspace run org.example.game:game -- some arguments
```

Frames run at 60 a second. The fixed step is 1/60 s, catching up at most 5 steps a frame; `update` gets the real time since the last frame. An application stops when:

- its code calls [`application.exit(status)`](@/docs/api/application.md): it stops after the current frame, and `status` is the process's exit status;
- `--frames` runs out: status 0;
- you press Ctrl+C: status 130. On Unix SIGTERM does the same, and a signal the process inherited as ignored (a background job's SIGINT) stays ignored; on Windows Ctrl+Break does the same. The second one ends the process immediately, for an application whose shutdown handler is the problem.

Every way out runs the `shutdown` handlers. Diagnostics go to stderr as they happen, one line each, with the context by name, the phase or event, and where the handler was defined. A handler that fails the same way every frame is one line, then `(repeated N more times)` when it stops, at exit, or once a minute while it doesn't:

```text
jess: error in org.example.app/main during update (org.example.app@1.0.0/src/main.jsl:7): org.example.app@1.0.0/src/main.jsl:9: frame two broke
```

## The example

`tests/lua/application` in the jess repository is a small application you can copy: four packages and a workspace that holds them.

- `org.example.journal` exports `@example/journal`, which the others write what they did to;
- `org.example.inventory` provides `game.inventory`: light armor, real weights;
- `org.example.armor` depends on the inventory and overrides `game.inventory`: heavy armor, keeping the weights through `interfaceOverride`;
- `org.example.game` is the application: a director that looks the inventory up, reports a death on frame 2 and stops on frame 4, and a scoreboard that answers each death with a `gameOver` event.

```text
$ jess workspace --workspace tests/lua/application run org.example.game:game -- print
armor replaces light armor, keeping its weights
entry: heavy helm, weighs 5
director load
inventory startup
director startup: heavy cuirass, weighs 30
director update 1 (number)
director update 2 (number)
inventory drops Rowan's loot
armor dents for Rowan
scoreboard: death 1, Rowan on frame 2
director update 3 (number)
director game over: Rowan died
director update 4 (number)
director shutdown
```

Read it top to bottom and it's the whole contract: the override ran once, during load; the entry saw the composed interface; `load` before `startup`; three subscribers to `playerDied` in load order; the scoreboard's `gameOver` a frame later; shutdown last. Pack the four packages into JPKs and run `game.jpk` with `--package-path` pointing at them, and the output is the same, byte for byte.

## A window

An application with a window is the same machinery and one more capability. The target declares `gpu.window` beside `gpu.device`; a managed module opens a window, gives it a [plan](@/docs/api/gpu.md#plans) to show, changes what the plan draws from its handlers, and answers the window's news from a handler of the named event [`jess.window`](@/docs/api/events.md#jess-window). The host does the rest every frame: the window system's news first, then the frame, then one [commit](@/docs/api/gpu.md#commits) of everything the frame changed about what the GPU draws, which the windows draw on a thread of their own.

```text
triangle/
  jess.toml
  src/main.jsl
  src/viewer.jsl
```

```toml
manifest_version = 1

[package]
id = "org.example.triangle"
version = "1.0.0"
jess_api = "1"
include = ["src"]
default_target = "triangle"

[managed.viewer]
module = "src/viewer.jsl"

[targets.triangle]
kind = "application"
entry = "src/main.jsl"
capabilities = ["gpu.device", "gpu.window"]
```

The entry has nothing to do but say hello: `print('triangle: close the window to stop')`. The work is the managed module's, `src/viewer.jsl`:

```luau
--!strict
local application = require '@jess/application'
local render = require '@jess/render'
local shader = require '@jess/shader'

local program = shader.program {
  attributes = { position = shader.vec2, color = shader.vec3 },
  pass = { angle = shader.float, aspect = shader.float },
  vertex = function(input)
    local corner, turn = input.attributes.position, input.pass.angle
    local cosine, sine = shader.cos(turn), shader.sin(turn)
    local across = corner.x * cosine - corner.y * sine
    local up = corner.x * sine + corner.y * cosine

    return {
      position = shader.vec4(across / input.pass.aspect, up, 0, 1),
      color = input.attributes.color,
    }
  end,
  fragment = function(input) return shader.vec4(input.color, 1) end,
}

local device = jess.gpu.device()
local window = jess.gpu.window(device, { title = 'triangle', width = 640, height = 480 })
local format = assert(window.format)
local pipeline = device:pipeline(program, { colors = { format } })

local corners = render.vertices(program.layout, 3)
corners:set(1, 'position', { 0, 0.8 })
corners:set(1, 'color', { 1, 0.3, 0.1 })
corners:set(2, 'position', { -0.7, -0.5 })
corners:set(2, 'color', { 0.1, 1, 0.3 })
corners:set(3, 'position', { 0.7, -0.5 })
corners:set(3, 'color', { 0.2, 0.4, 1 })

local vertices = device:buffer(corners.stride * corners.count, 'vertex')
corners:upload(vertices)
device:drawSet { pipeline = pipeline, vertices = vertices, count = 3, tags = { 'scene' } }

local turning = render.block(program.layout, 'pass')
local turningBuffer = device:buffer(turning.size, 'uniform')
turning:set('aspect', window.width / window.height)

window:show(
  device:plan {
    viewport = { width = window.width, height = window.height },
    targets = { { name = 'screen', format = format, imported = true } },
    passes = {
      {
        name = 'scene',
        kind = 'draw',
        tags = { 'scene' },
        pass = pipeline:bind('pass', { [0] = turningBuffer }),
        colors = { { target = 'screen', clear = { 0.05, 0.05, 0.08, 1 } } },
      },
    },
  },
  'screen'
)

local angle = 0

return {
  on = {
    host = {
      update = function(deltaSeconds: number)
        angle += deltaSeconds
        turning:set('angle', angle)
        turning:upload(turningBuffer)
      end,

      shutdown = function() print(`drew {window.frames} frames`) end,
    },

    event = {
      ['jess.window'] = function()
        for _, event in window:events() do
          if event.kind == 'resized' then
            turning:set('aspect', event.width / event.height)
            print(`resized to {event.width}×{event.height}`)
          elseif event.kind == 'closed' then
            application.exit(0)
          end
        end
      end,
    },
  },
}
```

```text
$ jess package run triangle --frames 120
triangle: close the window to stop
drew 119 frames
```

Without `--frames` it turns until you close the window. The run needs no `--trusted`: `gpu.device` and `gpu.window` are what a script you run holds anyway, so running a target grants the ones it declares; it still holds nothing it didn't declare.

What each piece is doing:

- **The module body runs once, at load.** The window, the pipeline, the vertex buffer, the draw set, and the plan are made there and live as long as the module's locals do, which is as long as the application. Nothing is made per frame.
- **`update` changes one uniform block.** The write waits for the frame's commit, so the window draws a new frame every frame and never half of one. A frame that changes nothing commits nothing, and the window draws nothing new; drop the `update` and a still scene runs no Luau at all.
- **The window's news arrives as an event.** `jess.window` is posted only when something happened to a window, and the handler reads what with `window:events()`. A resize has already resized the plan by the time the handler hears of it; the handler only fixes the aspect. The person's close button closes the window (`closes` is `true` unless the window is opened with `false`), and `closed` is the application's cue to stop.
- **Its keys and pointer are input.** A target that declares `gpu.window` gets each window's keyboard and pointer as [input devices](@/docs/api/input.md#windows), so the bindings it installs see them and its `on.host.input` handlers get their actions, and the same keys are among `window:events()` for code that wants them raw.

`window.frames` counts what reached the screen, and the last frame's draw can still be on its way when `shutdown` reads it: here, 120 frames drew 119.

## On the network

An application that talks to the network for weeks needs two things the host already gives it: a frame that comes around whether or not anything happened, and a shutdown that runs however it stops. What it needs on top is never to wait in a callback. A request that takes forty seconds inside `update` is forty seconds with no frames, no heartbeat and no `shutdown` either.

[`@jess/net`](@/docs/api/net.md) is how not to. A managed module keeps one loop, runs its network work as tasks on it, and steps it from `update` without waiting. Here, one task keeps a WebSocket open and reconnects with a backoff when it drops, one sends a heartbeat on it, and one makes a slow JSON POST; the heartbeat keeps its time while the POST waits:

```luau
--!strict
local application = require '@jess/application'
local http = require '@jess/http'
local net = require '@jess/net'
local websocket = require '@jess/websocket'
local config = require './config'

local loop = net.loop()
local socket: websocket.Socket?
local beats = 0

local function heartbeat(connected: websocket.Socket)
  while connected.state == 'open' do
    if not net.sleep(config.intervalMs) then return end

    beats += 1
    connected:send(`heartbeat {beats}`)
  end
end

local gateway = loop:spawn(function()
  local backoff = 1000
  while true do
    local connected, message, kind = websocket.connect(config.gateway)
    if connected then
      socket, backoff = connected, 1000
      print('connected')
      loop:spawn(function() heartbeat(connected) end, 'heartbeat')

      while connected:receive() do
      end
    elseif kind ~= 'cancelled' then
      print(`gateway: {kind}: {message}`)
    end

    if not net.sleep(backoff) then return end

    backoff = math.min(backoff * 2, 60000)
  end
end, 'gateway')

local inference = loop:spawn(function()
  local response, message, kind = http.request {
    url = config.inference,
    method = 'POST',
    json = { prompt = 'hello' },
    headers = { Authorization = `Bearer {config.key}` },
    timeoutMs = 120000,
  }

  -- Shutdown cancels a request still in flight; that is an answer, not a failure.
  if kind == 'cancelled' then return end

  assert(response, message)
  print(`inference {response.status} after {beats} heartbeats`)
  if config.exitAfterInference then application.exit(0) end
end, 'inference')

return {
  on = {
    host = {
      update = function() loop:step(0) end,
      shutdown = function()
        gateway:cancel()
        inference:cancel()
        local open = socket
        if open then loop:spawn(function() open:close(1001, 'shutting down') end, 'close') end

        loop:run(3000)
        loop:close()
        print('shut down')
      end,
    },
  },
}
```

The target declares what the module does and nothing else, `network.dns.resolve` and `network.tcp.connect`, and runs with exactly those, never `--trusted`:

```sh
jess package run agent --capability network.dns.resolve --capability network.tcp.connect
```

What makes it hold up:

- **One loop, stepped with no wait.** `loop:step(0)` runs whatever is ready and returns. A message that arrives between frames waits at most a frame, about 16 ms, and a frame never waits for the network.
- **Every wait is a task's.** The request, the receive and the heartbeat's sleep each wait on the loop, so none holds up the others. jess's own suite runs this module against a server that takes 1.5 seconds to answer the POST, and checks that heartbeats 100 ms apart keep arriving with no gap past 400 ms.
- **Failures come back as values.** `connect`, `receive` and `request` return `nil`, a message and a kind, and the module decides what each means: back off and reconnect, give up, or carry on. A task that raises anyway is reported by the `step` that ran it, as a failure in `update`, and the loop runs on.
- **Shutdown is ordinary code.** Ctrl+C, SIGTERM and `application.exit` all run `shutdown`. It cancels what's in flight (the cancelled request returns `cancelled`, which the task takes as the answer it is), starts the close handshake as a task, gives the loop three seconds to finish, and closes it, which closes anything left. A `shutdown` that raised part way would leave the rest undone, which is why the inference task checks for `cancelled` before it asserts.
- **Nothing piles up.** A loop watches a socket only while a task waits on it, connections count against `maxConnections` and tasks against `maxTasks`, sends wait for the socket instead of queueing, and every message is bounded both ways, by `maxMessageBytes` and `maxSendBytes`. The suite reconnects 200 times and checks that the loop holds nothing afterwards and the heap hasn't grown.

An application that calls the same services all day keeps an [`http.client`](@/docs/api/http.md#client), so its requests share connections instead of resolving, connecting and shaking hands every time; a model that answers token by token is read with [`http.open`](@/docs/api/http.md#open), or with [`@jess/sse`](@/docs/api/sse.md) when it answers in Server-Sent Events, each piece handled as it arrives. They wait the way everything above does: the suite holds a heartbeat 50 ms apart beside an event stream and twenty pooled requests on one loop, and checks it never goes quiet for 300 ms.

## Authority

An application holds what its target declares and nothing more, as a command target does. Every managed module's context holds that grant; a package doesn't get a VM or a capability of its own for being in the graph. Workers a module spawns hold a subset, and have no interfaces and no event queue of their own to emit into. `require` works in managed modules exactly as in any package module: their own files, their declared dependencies' exports, never past their package's root.

## Embedding it

The command line is one driver. `jess::lua::package::launch` returns an `Application` for a host of your own:

```rust
use std::time::Duration;

use jess::lua::package::{self, Launched, TargetRequest};

let request = TargetRequest {
    package: "packages/game.jpk".to_owned(),
    paths: vec!["packages".to_owned()],
    grant: Some(Vec::new()),
    ..TargetRequest::default()
};
let Launched::Application(mut game) = package::launch(&jess::lua::Options::default(), &request)? else {
    return Err("not an application target".into());
};
game.start()?;
while game.exit_requested().is_none() {
    game.frame(Duration::from_millis(16))?;
    for line in game.diagnostics() {
        eprintln!("{line}");
    }
}
game.stop()?;
```

`package::launch_with` takes `LaunchOptions`; its `input` is the application's input source (`InputSource::Synthetic` for tests and replays, a platform backend for devices). With one, every frame polls it first and runs the `input` handlers on the action events it produced; `application.host().input()` is the service, for the host to configure.

`frame` is the domain's frame alone. `Application::step(delta, report)` is the whole of one as the command line runs it: the window system's news first (and, for each window something happened to, the named event [`jess.window`](@/docs/api/events.md#jess-window)), then the frame, then the frame's changes to what the GPU draws committed together and the windows told to draw them, with what a window or the GPU refused passed to `report`. A host with windows steps; one without can do either.

`Application::restart` loads everything again in a new generation: a new runtime, new module tables, new handlers, new interfaces, and queued events for the old generation dropped. Two applications are two domains; they share nothing, not even an interface id.

## Current limits

- The command line attaches no device backend of its own. Under `jess package run` and `jess workspace run`, an application's input is the keyboard and pointer of each of its [windows](@/docs/api/gpu.md#windows), for a target that declares `gpu.window`, and nothing for one that doesn't. A gamepad, or a keyboard with no window in front of it, reaches an application only through a host that launches it with `package::launch_with` and an input source in its `LaunchOptions`, which then gets every device, every frame, before update.
- Interfaces are composed once, at load. No hot override.
- Every managed context holds the target's whole grant; per-module capability subsets need manifest syntax nobody has designed.
- Routing between domains is a host decision with a Rust API. There is no script API for it, on purpose, until there is a host that runs several application domains from one command.
- Managed contexts are labels for attribution, not walls. Code anywhere in the domain can `context.bind` another module's context by its name, `<package>/<managed>`, and replace its handlers. It gains nothing today, since every context holds the same grant; per-module grants will need context ownership checked first.
