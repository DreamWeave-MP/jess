---
title: '@jess/input'
description: 'Author an action graph as data, compile and validate it, label and check it for a settings screen, route its edges to intents, and drive it headlessly on a clock you control.'
weight: 115
extra:
  kind: api
---

{{ api_signature(value="require('@jess/input'): { key, gamepadButton, pointerButton, button, axis, gamepadAxis, pointerAxis, constant, invert, scale, deadzone, threshold, chord, any, gate, tap, hold, axisFromButtons, vector2, actionActive, actionValue, graph, fromJson, document, label, bindings, presentations, conflicts, router, runtime, formatVersion }") }}

`@jess/input` is input for scripts, the built-in package's Luau over `jess.input`, the native half ([Input Runtime](@/docs/contributing/input-runtime.md) has the model). A script writes a binding document as tables, the native side compiles, validates, and evaluates it, and everything that decides what input means to a person (labels, the shape of a binding a GUI draws, conflict checks, intents) is Luau over the document and the action events. Under a host ([Application Kernel](@/docs/contributing/application-kernel.md)) that runtime is the application's live input: real devices feed it, the host ticks it in the input phase, and the script configures it and reads it after the tick. Anywhere else it is headless: synthetic devices and a clock the script moves, which is what a test, a replay, or a package's own binding check needs. No device event ever calls into Luau.

{% usage_note(title="Three capabilities") %}
Under a script domain every runtime method checks a capability: `input.actions` for `action`, `stats`, `now`, and `tick`; `input.observe` for `devices`; `input.configure` for everything that changes the runtime (`install`, `uninstall`, `restoreDefaults`, `setActive`, `pushLayer`, `popLayer`, `connect`, `disconnect`, `press`, `release`, `axis`, `focus`, `advance`). A context without the grant gets `capability 'input.actions' denied to domain#1 gen#1 context#2`. Everything else is data and checks nothing: the builders, compiling, labels, presentation, conflicts, and the router. Outside any domain the runtime's own grant decides: `jess run`, `jess test`, and a package's tests hold all three, a package target holds the ones it declares, and a runtime made without one refuses with `jess.input: needs the 'input.configure' capability, which this runtime does not grant`.
{% end %}

Three things to know before the signatures:

- **Nothing here means anything.** Action names (`move`, `editor.save`) are yours; the module reserves none.
- **Durations are seconds** in every function and milliseconds in the document (`withinMs`, `durationMs`), which is what the native crate stores.
- **Bindings are addressed by position from 1** in their set's or layer's `bindings`, the way the document's own array counts.
- **Values are shaped by the action's `valueType`:** `digital` is a boolean, `scalar` a number, `vector2` a `vector` with `z` 0.

## Nodes

{{ api_signature(value="input.key(name: string) -> Node") }}

A key by its positional name: the W3C `KeyboardEvent.code` list (`KeyW`, `ControlLeft`, `Space`, `Numpad7`). `KeyW` is the same key on an AZERTY board; what's printed on it is the layout's business.

{{ api_signature(value="input.gamepadButton(name: string) -> Node") }}

A gamepad button by position: `South`, `East`, `West`, `North`, `LeftBumper`, `RightBumper`, `LeftTrigger`, `RightTrigger`, `Select`, `Start`, `Guide`, `LeftStick`, `RightStick`, `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`. `South` is `A` on an Xbox pad and `Cross` on a PlayStation one; [`label`](#labels) supplies the letter.

{{ api_signature(value="input.pointerButton(name: string) -> Node") }}

`Primary`, `Secondary`, `Middle`, `Back`, or `Forward`.

{{ api_signature(value="input.button(control: string) -> Node") }}
{{ api_signature(value="input.axis(control: string) -> Node") }}

Any control by its full name, `class:Name`: `key:KeyA`, `gamepadButton:South`, `gamepadAxis:LeftStickX`, `pointerAxis:DeltaY`, `touch:0`. `button` refuses an axis and `axis` a button, each naming the other. Every builder that names a control checks it against the native vocabulary (`no key of that name`) when it is called.

{{ api_signature(value="input.gamepadAxis(name: string) -> Node") }}
{{ api_signature(value="input.pointerAxis(name: string) -> Node") }}

Axes by bare name. Sticks (`LeftStickX`, `LeftStickY`, `RightStickX`, `RightStickY`) read `-1..1` with `Y` up; triggers (`LeftTrigger`, `RightTrigger`) read `0..1`. Pointer axes are `X`, `Y` (position, pixels), `DeltaX`, `DeltaY` (motion this tick), `WheelX`, `WheelY` (steps this tick).

{{ api_signature(value="input.constant(value: number) -> Node") }}
{{ api_signature(value="input.invert(input: Node) -> Node") }}
{{ api_signature(value="input.scale(input: Node, factor: number) -> Node") }}
{{ api_signature(value="input.deadzone(input: Node, inner: number, outer: number) -> Node") }}
{{ api_signature(value="input.threshold(input: Node, level: number) -> Node") }}

Transforms. `invert` is not for a digital input and negation otherwise; `scale` and `deadzone` take a scalar or a vector, and a deadzone on a vector is radial, so a stick's dead spot stays round: magnitudes at or below `inner` are zero, at or above `outer` are one, and the band between is rescaled. `threshold` makes a button of a scalar, on at magnitude `level` or more (`0 <= inner < outer <= 1`, `level > 0`).

{{ api_signature(value="input.chord(members: { Node }) -> Node") }}
{{ api_signature(value="input.any(members: { Node }) -> Node") }}

`chord` is every member held: on when the last member goes down, off when any comes up, whatever the order. Members aren't consumed, so a plain binding to `S` still fires under `Ctrl+S`. `any` is any member held; it's how one action takes a keyboard key or a pad button.

{{ api_signature(value="input.gate(input: Node, condition: Node) -> Node") }}

`input` while the digital `condition` is on, and the input type's zero while it's off. With `actionActive` it's how one action depends on another.

{{ api_signature(value="input.tap(input: Node, within: number) -> Node") }}
{{ api_signature(value="input.hold(input: Node, duration: number) -> Node") }}

A tap is on for exactly one tick when `input` is released within `within` seconds of its press. A hold turns on once `input` has been held for `duration` seconds and stays on until release. Both measure from the device's own event timestamps, and a release after the tap window is not a tap, so `tap(a, 0.2)` and `hold(a, 0.35)` on the same button tell themselves apart.

{{ api_signature(value="input.axisFromButtons(negative: Node, positive: Node) -> Node") }}
{{ api_signature(value="input.vector2(horizontal: Node, vertical: Node) -> Node") }}

Two buttons as a `-1`, `0`, `1` axis (both held is `0`), and two scalars as a vector: `horizontal` is its `x`, `vertical` its `y`.

{{ api_signature(value="input.actionActive(action: string) -> Node") }}
{{ api_signature(value="input.actionValue(action: string) -> Node") }}

Another action, by name: whether it's active this tick, or its value with its type. Actions are evaluated after the actions they read; a cycle is a compile error that names it.

## Compiling

{{ api_signature(value="input.graph(document: Document) -> InputGraph") }}

Compiles a document and validates all of it at once. A document is `actions` (each `{ name, valueType }`), `sets` (each `{ name, owner, bindings }`), and optional `layers` (each `{ name, owner, bindings, masks }`); a binding is `{ action, input }`. `owner` is the package declaring the set, for diagnostics. `version`, `sets`, and `layers` are filled in when left out, in a copy: the table you pass isn't changed.

- **Errors:** every problem in the document, one per line, each placed: `set "gameplay" (from @jess/example), action "quickSave", binding 1, at chord: chord's members[1] must be digital, and it is scalar`. Unknown actions, duplicate names, inputs of the wrong type, a binding that doesn't produce its action's type, zero thresholds, bad deadzone edges, and cycles are all refused. A document with any error compiles to nothing.

```luau
--!strict
local input = require '@jess/input'

local document: input.Document = {
  actions = {
    { name = 'move', valueType = 'vector2' },
    { name = 'quickSave', valueType = 'digital' },
    { name = 'interact', valueType = 'digital' },
    { name = 'contextMenu', valueType = 'digital' },
    { name = 'sprint', valueType = 'digital' },
  },

  sets = {
    {
      name = 'gameplay',
      owner = '@my/package',
      bindings = {
        {
          action = 'move',
          input = input.vector2(
            input.axisFromButtons(input.key 'KeyA', input.key 'KeyD'),
            input.axisFromButtons(input.key 'KeyS', input.key 'KeyW')
          ),
        },
        { action = 'quickSave', input = input.chord { input.key 'ControlLeft', input.key 'KeyS' } },
        { action = 'interact', input = input.tap(input.gamepadButton 'South', 0.2) },
        { action = 'contextMenu', input = input.hold(input.gamepadButton 'South', 0.35) },
        { action = 'sprint', input = input.gate(input.key 'ShiftLeft', input.actionActive 'move') },
      },
    },
  },

  layers = {
    {
      name = 'inventory',
      owner = '@my/package',
      bindings = { { action = 'interact', input = input.key 'KeyE' } },
      masks = { 'sprint' },
    },
  },
}

local graph = input.graph(document)
print(#graph:actions()) -- 5
```

{{ api_signature(value="input.fromJson(text: string) -> InputGraph") }}

Reads a saved document and compiles it.

- **Errors:** a document of another format version, an unknown node type or field (``unknown variant `doubleTap` ``), a value of the wrong type, each with the line and column; then everything `graph` refuses. Nothing unrecognized is skipped.

{{ api_signature(value="input.document(graph: InputGraph) -> Document") }}

A compiled graph's document, with everything `graph` fills in filled in: the table to hand to `bindings`, `presentations`, and `conflicts`, or to edit and compile again.

### InputGraph

{{ api_signature(value="graph:toJson() -> string") }}
{{ api_signature(value="graph:actions() -> { { name: string, valueType: InputValueType } }") }}

The document as JSON (the same document is always the same text, so it diffs and saves), and the actions in declaration order. A compiled graph is what `runtime:install` takes.

## Labels

{{ api_signature(value="input.label(family: Family, control: string) -> string") }}

The glyph identifier a presentation family (`keyboardMouse`, `xbox`, `playstation`, `nintendo`, `genericGamepad`, `touch`) gives a control: `A` for `gamepadButton:South` on an Xbox pad, `Cross` on a PlayStation one, `B` on a Nintendo one (whose face buttons are swapped against Xbox's), and the positional name (`South`) on a generic pad. Keys, pointer controls, and axes keep their names, and a touch contact is `Contact0`. A theme maps the identifier to whatever it draws; a device reports its family in `runtime:devices()`.

```luau
--!strict
print(input.label('xbox', 'gamepadButton:South')) -- A
print(input.label('nintendo', 'gamepadButton:South')) -- B
print(input.label('keyboardMouse', 'key:KeyW')) -- KeyW
```

## Presentation and conflicts

{{ api_signature(value="input.bindings(document: Document, action: string) -> { Location }") }}

Where an action's bindings are: `{ set = 'gameplay', index = 3 }` or `{ layer = 'inventory', index = 1 }`, sets first. `index` counts from 1 in that owner's `bindings`.

{{ api_signature(value="input.presentations(document: Document, action: string, family: Family) -> { BindingPresentation }") }}

The action's bindings a person using `family` can press, as data for a GUI to draw. Each has the `location`, and a `shape`: `{ kind = 'control', control = 'gamepadButton:South', family = 'xbox', label = 'A' }`, or `chord`/`anyOf` with `parts`, `tap`/`hold` with `input` and the threshold in seconds (`within`, `duration`), `axis` with `negative` and `positive`, `vector2` with `horizontal` and `vertical`, `while` with `input` and `condition`, `action` with `action`, or `constant`. Transforms that don't change what to press are left out.

```luau
--!strict
local shape = input.presentations(document, 'contextMenu', 'playstation')[1].shape
if shape.kind == 'hold' and shape.input.kind == 'control' then
  print(shape.kind, shape.duration, shape.input.label) -- hold  0.35  Cross
end
print(#input.presentations(document, 'contextMenu', 'keyboardMouse')) -- 0
```

{{ api_signature(value="input.conflicts(document: Document) -> { Conflict }") }}

Two bindings in one set or layer that fire different actions from the same press, chord, tap, hold, or axis: `{ owner = { set = 's' }, kind = 'press', controls = { 'key:ControlLeft', 'key:KeyS' }, actions = { 'a', 'b' }, bindings = { ... } }`, the controls sorted. `S` beside `Ctrl+S`, a tap beside a hold, a binding to one particular device, and the same button in two sets are not reported: whether those collide depends on what an application activates together, and the document has the structure to judge it.

## Intents

{{ api_signature(value="input.router() -> Router") }}

An action is interpreted input (`ui.accept` became active); an intent is what that means to a target (activate whatever has focus). A router is a table of routes from action edges to intents and nothing more: no targets, no focus, no command table.

{{ api_signature(value="router:route(action: string, kind: 'pressed' | 'released' | 'changed', intent: string)") }}
{{ api_signature(value="router:dispatch(events: { InputActionEvent }) -> { Intent }") }}
{{ api_signature(value="router:routes() -> { Route }") }}
{{ api_signature(value="router:clear()") }}

`dispatch` takes a tick's events, from `runtime:tick()` or an input phase's payload, and returns the intents they mean, each event's routes in the order they were added: `{ intent, action, kind, value, time, device }`. Routes are by name, so they outlive every graph, and a route to an action no graph declares waits for one that does.

```luau
--!strict
local router = input.router()
router:route('quickSave', 'pressed', 'game.save')

local saver = input.runtime()
saver:install(graph)
saver:setActive 'gameplay'
local board = saver:connect 'keyboard'
saver:press(board, 'key:ControlLeft')
saver:press(board, 'key:KeyS')
for _, intent in router:dispatch(saver:tick()) do
  print(intent.intent, intent.action) -- game.save  quickSave
end
```

## The runtime

{{ api_signature(value="input.runtime() -> InputRuntime") }}

Under a host, the domain's live runtime: every call returns a handle over the same one, the host ticks it once per frame, and the input context's handler receives that tick's events as its payload (`{ time, generation, events }`, rows shaped as below; a router makes intents of them). `tick` and `advance` are refused there (`InputRuntime:tick is the host's on a live input runtime: ...`); synthetic events (`connect`, `press`, ...) still work, under `input.configure`, and land in the next host tick.

Anywhere else, a headless runtime of the script's own: synthetic devices, a clock that moves only when the script says so, and one graph at a time.

```luau
--!strict
local runtime = input.runtime()
runtime:install(graph)
runtime:setActive 'gameplay'
local keyboard, pad = runtime:connect 'keyboard', runtime:connect 'gamepad'
runtime:tick()

runtime:press(keyboard, 'key:ControlLeft')
runtime:advance(0.016)
runtime:tick()
print((runtime:action 'quickSave' :: InputActionState).active) -- false

runtime:press(keyboard, 'key:KeyS')
runtime:advance(0.016)
for _, event in runtime:tick() do
  print(event.action, event.kind) -- move  pressed / move  changed / quickSave  pressed
end

runtime:press(pad, 'gamepadButton:South')
runtime:advance(0.4)
runtime:tick()
print((runtime:action 'contextMenu' :: InputActionState).pressed) -- true
```

`S` is also `move`'s back key, so the chord's tick moves too: chord members aren't consumed.

### InputRuntime

{{ api_signature(value="runtime:install(graph: InputGraph) -> number") }}
{{ api_signature(value="runtime:uninstall()") }}
{{ api_signature(value="runtime:restoreDefaults()") }}

`install` takes a compiled graph (from `graph` or `fromJson`), replaces the installed one, and returns the new generation; sets that were active and layers that were pushed stay so where the new graph has them by name, every action starts from rest, and a button held across the swap is ignored until released. `restoreDefaults` puts the installed document back.

{{ api_signature(value="runtime:setActive(set: string, active: boolean?)") }}
{{ api_signature(value="runtime:pushLayer(layer: string)") }}
{{ api_signature(value="runtime:popLayer(layer: string) -> boolean") }}

Activate (the default) or deactivate a set; push a layer on top of the stack, or remove it (`true` if it was pushed). All take effect at the next tick. The topmost pushed layer that names an action decides it; masked actions are silent.

- **Errors:** `the installed action graph has no set "menu"`; no graph installed.

{{ api_signature(value="runtime:connect(kind: 'keyboard' | 'pointer' | 'gamepad' | 'touch') -> number") }}
{{ api_signature(value="runtime:disconnect(device: number)") }}
{{ api_signature(value="runtime:devices() -> { InputDevice }") }}

Synthetic devices, by id. Disconnecting releases everything the device held. `devices` lists every connected device the runtime has seen, real ones included, as of the last tick.

- **Errors:** `no device 3 is known to this runtime`.

{{ api_signature(value="runtime:press(device: number, control: string)") }}
{{ api_signature(value="runtime:release(device: number, control: string)") }}
{{ api_signature(value="runtime:axis(device: number, control: string, value: number)") }}
{{ api_signature(value="runtime:focus(focused: boolean)") }}
{{ api_signature(value="runtime:advance(seconds: number)") }}
{{ api_signature(value="runtime:now() -> number") }}

Events, stamped with the runtime's clock, which `advance` moves. A relative axis (`pointerAxis:DeltaX`) moves by `value`; every other axis moves to it.

{{ api_signature(value="runtime:tick() -> { InputActionEvent }") }}

Applies every event pushed so far, evaluates the graph once, and returns this tick's edges: `{ action, kind = 'pressed' | 'released' | 'changed', value, time, device }`. A press and release that both land between two ticks are invisible, so tick as often as the shortest input you care about. On a live runtime the host ticks; this raises.

{{ api_signature(value="runtime:action(name: string) -> InputActionState?") }}

`{ value, active, pressed, released, changed, since, device }` after the last tick, or `nil` for a name the graph doesn't declare. `pressed` and `released` are this tick's edges; `device` is the id behind the value.

{{ api_signature(value="runtime:stats() -> InputStats") }}

What the last tick cost: `deviceEvents`, `ticks`, `nodesEvaluated`, `bindingsEvaluated`, `actionsChanged`, `actionEvents`, `activeSets`, `activeLayers`.

## jess.input

{{ api_signature(value="jess.input: { graph, formatVersion, control, runtime }") }}

The native half `@jess/input` is written over: `graph(json)` compiles a document already encoded as JSON, `control(text)` returns a control's canonical `class:Name` or raises, `formatVersion` is the document format this jess reads and writes, and `runtime` is `input.runtime`. A script has no reason to call it directly; it is here so nothing about the boundary is hidden.

Related: [Input Runtime](@/docs/contributing/input-runtime.md), the model behind all of this.
