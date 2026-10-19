---
title: 'Applications'
description: 'Application targets: managed modules loaded into one ScriptDomain, host callbacks, named events, and domain-local interfaces that later packages override on purpose.'
weight: 98
extra:
  kind: guide
---

A command target is a script with a package around it: it runs, it returns, it's done. An **application target** keeps running. jess builds it a host of its own, loads the managed modules of every package in its graph into one [ScriptDomain](@/docs/contributing/script-domains.md), and drives frames until the application says it's finished.

Nothing about this is Morrowind. A game, an editor, and a launcher are all "some packages, some callbacks, some frames"; what the callbacks mean is the packages' business.

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

`on.event` holds handlers for **named events**, keyed by event name. Names are case-sensitive strings; dots and anything else go in quoted keys, `['game.playerDied'] = function(payload) ... end`.

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

1. **Resolve.** The target's package resolves with its dependencies (lock pins and package paths as for any target), every layer it declares is planned, and every entrypoint's capabilities are checked against what the target declares. A capability the target declares and you didn't grant stops it here too. No package code has run yet.
2. **Build the host.** One `Host`, one domain, holding exactly the target's capabilities.
3. **Mount and activate.** Inside the domain's load, the plan is mounted in the domain's one primary runtime and the layers' entrypoints run, as for any activation.
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

events.emit('playerDied', { player = 'Caius' })
```

`emit` queues the event and returns. No handler runs inside `emit`, so emitting from a handler never re-enters one. The domain's next event drain, at the end of the current frame, delivers it to every handler bound to that name, in context order (load order). Events are delivered in the order they were emitted. One emitted while the queue drains waits for the next drain, which means the next frame. A handler that raises is reported, attributed to its context and event, and the next handler runs anyway. An event nobody handles is counted in the frame's report and dropped; nothing waits for a listener that will never come.

The payload has to be **transferable**: nil, booleans, numbers, strings, vectors, buffers, and tables of those, nested at most 32 deep. No functions, no userdata, no threads. The same rules hold whether the event stays in the domain or crosses to another one, so an event that works locally doesn't break the day a host routes it somewhere. A payload that can't cross raises, naming the path to the problem:

```text
payload.onDeath: a function can't cross a worker or event boundary
```

A table that contains itself is refused the same way, as too deep, with a path that repeats until you notice.

Script events and host callbacks never meet. `events.emit('update')` delivers to `on.event.update` handlers if there are any, and never runs an `on.host.update`. `events.emit('interfaceOverride')` is just an event with a silly name. A script can't enter a host phase at all; the host owns the scheduler.

### Between domains

Each domain has its own queue, and nothing is broadcast. A host that wants one domain's events in another routes them, explicitly, by name:

```rust
// Every `playerDied` the game emits is also posted to the server, as its payload.
game.domain().route("playerDied", server.domain().mailbox()?)?;
```

A native host can also post a named event into any domain from any thread with `DomainMailbox::post_named`. Either way it is stamped with the recipient's generation, so a restart drops what was queued for the old code. Interfaces never cross; only the event's payload does.

## Running and stopping

```sh
jess package run game --package-path packages                       # until it stops itself
jess package run game --package-path packages --frames 600          # or after 600 frames
jess workspace run org.example.game:game -- some arguments
```

Frames run at 60 a second. The fixed step is 1/60 s, catching up at most 5 steps a frame; `update` gets the real time since the last frame. An application stops when:

- its code calls [`application.exit(status)`](@/docs/api/application.md): it stops after the current frame, and `status` is the process's exit status;
- `--frames` runs out: status 0;
- you press Ctrl+C (or send SIGTERM) on Unix: status 130. The second Ctrl+C ends the process immediately, for an application whose shutdown handler is the problem. On Windows, Ctrl+C ends the process without the shutdown handlers; use `application.exit`.

Every way out runs the `shutdown` handlers. Diagnostics go to stderr as they happen, one line each, with the context by name, the phase or event, and where the handler was defined:

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
inventory drops Caius's loot
armor dents for Caius
scoreboard: death 1, Caius on frame 2
director update 3 (number)
director game over: Caius died
director update 4 (number)
director shutdown
```

Read it top to bottom and it's the whole contract: the override ran once, during load; the entry saw the composed interface; `load` before `startup`; three subscribers to `playerDied` in load order; the scoreboard's `gameOver` a frame later; shutdown last. Pack the four packages into JPKs and run `game.jpk` with `--package-path` pointing at them, and the output is the same, byte for byte.

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

`Application::restart` loads everything again in a new generation: a new runtime, new module tables, new handlers, new interfaces, and queued events for the old generation dropped. Two applications are two domains; they share nothing, not even an interface id.

## Not yet

- The command-line host has no input source, so `input` handlers don't run under `jess package run`. A host that attaches one gets them.
- Interfaces are composed once, at load. No hot override.
- Every managed context holds the target's whole grant; per-module capability subsets need manifest syntax nobody has designed.
- Routing between domains is a host decision with a Rust API. There is no script API for it, on purpose, until there is a host that runs several application domains from one command.
