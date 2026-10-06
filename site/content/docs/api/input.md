---
title: 'jess.input'
description: 'Author an action graph as data, compile and validate it, and drive it headlessly with synthetic devices on a clock you control.'
weight: 115
extra:
  kind: api
---

{{ api_signature(value="jess.input: { key, gamepadButton, pointerButton, button, axis, gamepadAxis, pointerAxis, constant, invert, scale, deadzone, threshold, chord, any, gate, tap, hold, axisFromButtons, vector2, actionActive, actionValue, graph, fromJson, runtime }") }}

`jess.input` is the Luau face of `jess-input`, the input substrate under the future GUI ([Input Runtime](@/docs/contributing/input-runtime.md) has the model). A script writes a binding document as tables, the native side compiles and validates it, and a runtime evaluates it. Under a host ([Application Kernel](@/docs/contributing/application-kernel.md)) that runtime is the application's live input: real devices feed it, the host ticks it in the input phase, and the script configures it and reads it after the tick. Anywhere else it is headless: synthetic devices and a clock the script moves, which is what a test, a replay, or a package's own binding check needs. No device event ever calls into Luau.

{% usage_note(title="Three capabilities") %}
Under a script domain every runtime method checks a capability: `input.actions` for `action`, `intents`, `stats`, `now`, and `tick`; `input.observe` for `devices`; `input.configure` for everything that changes the runtime (`install`, `uninstall`, `restoreDefaults`, `setActive`, `pushLayer`, `popLayer`, `route`, `connect`, `disconnect`, `press`, `release`, `axis`, `focus`, `advance`). A context without the grant gets `capability 'input.actions' denied to domain#1 gen#1 context#2`. The authoring functions, `graph`, `fromJson`, and the graph methods are data and check nothing. Outside any domain (a plain `jess run`) nothing holds authority and every call succeeds.
{% end %}

Three things to know before the signatures:

- **Nothing here means anything.** Action names (`move`, `editor.save`) are yours; the module reserves none.
- **Durations are seconds** in every function and milliseconds in the document (`withinMs`, `durationMs`), which is what the native crate stores.
- **Values are shaped by the action's `valueType`:** `digital` is a boolean, `scalar` a number, `vector2` a `vector` with `z` 0.

## Nodes

{{ api_signature(value="jess.input.key(name: string) -> InputNode") }}

A key by its positional name: the W3C `KeyboardEvent.code` list (`KeyW`, `ControlLeft`, `Space`, `Numpad7`). `KeyW` is the same key on an AZERTY board; what's printed on it is the layout's business.

{{ api_signature(value="jess.input.gamepadButton(name: string) -> InputNode") }}

A gamepad button by position: `South`, `East`, `West`, `North`, `LeftBumper`, `RightBumper`, `LeftTrigger`, `RightTrigger`, `Select`, `Start`, `Guide`, `LeftStick`, `RightStick`, `DPadUp`, `DPadDown`, `DPadLeft`, `DPadRight`. `South` is `A` on an Xbox pad and `Cross` on a PlayStation one; a presentation family supplies the letter.

{{ api_signature(value="jess.input.pointerButton(name: string) -> InputNode") }}

`Primary`, `Secondary`, `Middle`, `Back`, or `Forward`.

{{ api_signature(value="jess.input.button(control: string) -> InputNode") }}
{{ api_signature(value="jess.input.axis(control: string) -> InputNode") }}

Any control by its full name, `class:Name`: `key:KeyA`, `gamepadButton:South`, `gamepadAxis:LeftStickX`, `pointerAxis:DeltaY`, `touch:0`. `button` refuses an axis and `axis` a button, each naming the other.

{{ api_signature(value="jess.input.gamepadAxis(name: string) -> InputNode") }}
{{ api_signature(value="jess.input.pointerAxis(name: string) -> InputNode") }}

Axes by bare name. Sticks (`LeftStickX`, `LeftStickY`, `RightStickX`, `RightStickY`) read `-1..1` with `Y` up; triggers (`LeftTrigger`, `RightTrigger`) read `0..1`. Pointer axes are `X`, `Y` (position, pixels), `DeltaX`, `DeltaY` (motion this tick), `WheelX`, `WheelY` (steps this tick).

{{ api_signature(value="jess.input.constant(value: number) -> InputNode") }}
{{ api_signature(value="jess.input.invert(input: InputNode) -> InputNode") }}
{{ api_signature(value="jess.input.scale(input: InputNode, factor: number) -> InputNode") }}
{{ api_signature(value="jess.input.deadzone(input: InputNode, inner: number, outer: number) -> InputNode") }}
{{ api_signature(value="jess.input.threshold(input: InputNode, level: number) -> InputNode") }}

Transforms. `invert` is not for a digital input and negation otherwise; `scale` and `deadzone` take a scalar or a vector, and a deadzone on a vector is radial, so a stick's dead spot stays round: magnitudes at or below `inner` are zero, at or above `outer` are one, and the band between is rescaled. `threshold` makes a button of a scalar, on at magnitude `level` or more (`0 <= inner < outer <= 1`, `level > 0`).

{{ api_signature(value="jess.input.chord(members: { InputNode }) -> InputNode") }}
{{ api_signature(value="jess.input.any(members: { InputNode }) -> InputNode") }}

`chord` is every member held: on when the last member goes down, off when any comes up, whatever the order. Members aren't consumed, so a plain binding to `S` still fires under `Ctrl+S`. `any` is any member held; it's how one action takes a keyboard key or a pad button.

{{ api_signature(value="jess.input.gate(input: InputNode, condition: InputNode) -> InputNode") }}

`input` while the digital `condition` is on, and the input type's zero while it's off. With `actionActive` it's how one action depends on another.

{{ api_signature(value="jess.input.tap(input: InputNode, within: number) -> InputNode") }}
{{ api_signature(value="jess.input.hold(input: InputNode, duration: number) -> InputNode") }}

A tap is on for exactly one tick when `input` is released within `within` seconds of its press. A hold turns on once `input` has been held for `duration` seconds and stays on until release. Both measure from the device's own event timestamps, and a release after the tap window is not a tap, so `tap(a, 0.2)` and `hold(a, 0.35)` on the same button tell themselves apart.

{{ api_signature(value="jess.input.axisFromButtons(negative: InputNode, positive: InputNode) -> InputNode") }}
{{ api_signature(value="jess.input.vector2(x: InputNode, y: InputNode) -> InputNode") }}

Two buttons as a `-1`, `0`, `1` axis (both held is `0`), and two scalars as a vector.

{{ api_signature(value="jess.input.actionActive(action: string) -> InputNode") }}
{{ api_signature(value="jess.input.actionValue(action: string) -> InputNode") }}

Another action, by name: whether it's active this tick, or its value with its type. Actions are evaluated after the actions they read; a cycle is a compile error that names it.

## jess.input.graph

{{ api_signature(value="jess.input.graph(document: InputDocument) -> InputGraph") }}

Compiles a document and validates all of it at once. A document is `actions` (each `{ name, valueType }`), `sets` (each `{ name, owner, bindings }`), and optional `layers` (each `{ name, owner, bindings, masks }`); a binding is `{ action, input }`. `owner` is the package declaring the set, for diagnostics. `version` is filled in.

- **Errors:** every problem in the document, one per line, each placed: `set "gameplay" (from @jess/example), action "quickSave", binding 1, at chord: chord's members[1] must be digital, and it is scalar`. Unknown actions, duplicate names, inputs of the wrong type, a binding that doesn't produce its action's type, zero thresholds, bad deadzone edges, and cycles are all refused. A document with any error compiles to nothing.

```luau
--!strict
local input = jess.input

local graph = input.graph {
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

print(#graph:actions()) -- 5
```

### InputGraph

{{ api_signature(value="graph:toJson() -> string") }}
{{ api_signature(value="graph:spec() -> InputDocument") }}

The document as JSON (the same document is always the same text, so it diffs and saves) and as a table `jess.input.graph` accepts back.

{{ api_signature(value="graph:actions() -> { { name: string, valueType: InputValueType } }") }}
{{ api_signature(value="graph:bindings(action: string) -> { InputBindingLocation }") }}

The actions in declaration order, and where an action's bindings are: `{ set = 'gameplay', index = 2 }` or `{ layer = 'inventory', index = 0 }`, sets first. The index is the 0-based position in that owner's `bindings`, which is how a remap addresses a binding.

{{ api_signature(value="graph:presentations(action: string, family: InputFamily) -> { InputBindingPresentation }") }}

The action's bindings a person using `family` (`keyboardMouse`, `xbox`, `playstation`, `nintendo`, `genericGamepad`, `touch`) can press, as data for a GUI to draw. Each has the `location`, and a `shape`: `{ kind = 'control', control = 'gamepadButton:South', family = 'xbox', label = 'A' }`, or `chord`/`anyOf` with `parts`, `tap`/`hold` with `input` and the threshold in seconds, `axis` with `negative` and `positive`, `vector2` with `x` and `y`, `while` with `input` and `condition`, `action` with `action`, or `constant`. Transforms that don't change what to press are left out.

```luau
--!strict
local shape = graph:presentations('contextMenu', 'playstation')[1].shape
print(shape.kind, shape.duration, shape.input.label) -- hold  0.35  Cross
print(#graph:presentations('contextMenu', 'keyboardMouse')) -- 0
```

{{ api_signature(value="graph:conflicts() -> { InputConflict }") }}

Two bindings in one set or layer that fire different actions from the same press, chord, tap, hold, or axis: `{ owner = { set = 's' }, kind = 'press', controls = { 'key:KeyS', 'key:ControlLeft' }, actions = { 'a', 'b' }, bindings = { ... } }`. `S` beside `Ctrl+S`, a tap beside a hold, and the same button in two sets are not reported: whether those collide depends on what an application activates together, and the document has the structure to judge it.

## jess.input.fromJson

{{ api_signature(value="jess.input.fromJson(text: string) -> InputGraph") }}

Reads a saved document and compiles it.

- **Errors:** a document of another format version, an unknown node type or field (``unknown variant `doubleTap` ``), a value of the wrong type, each with the line and column; then everything `graph` refuses. Nothing unrecognized is skipped.

## jess.input.runtime

{{ api_signature(value="jess.input.runtime() -> InputRuntime") }}

Under a host, the domain's live runtime: every call returns a handle over the same one, the host ticks it once per frame, and the input context's handler receives that tick's events and intents as its payload (`{ time, generation, events, intents }`, rows shaped as below). `tick` and `advance` are refused there (`InputRuntime:tick is the host's on a live input runtime: ...`); synthetic events (`connect`, `press`, ...) still work, under `input.configure`, and land in the next host tick.

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

{{ api_signature(value="runtime:install(graph: InputGraph | InputDocument) -> number") }}
{{ api_signature(value="runtime:uninstall()") }}
{{ api_signature(value="runtime:restoreDefaults()") }}

`install` replaces the graph and returns the new generation; sets that were active and layers that were pushed stay so where the new graph has them by name, every action starts from rest, and a button held across the swap is ignored until released. A document that doesn't compile raises and changes nothing. `restoreDefaults` puts the installed document back.

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

{{ api_signature(value="runtime:route(action: string, kind: 'pressed' | 'released' | 'changed', intent: string)") }}
{{ api_signature(value="runtime:intents() -> { InputIntent }") }}

Intents: an edge of an action becomes a named intent, with the action's provenance. `intents` is the last tick's, in order: `{ intent, action, kind, value, time, device }`. Routes survive reinstalls by name.

{{ api_signature(value="runtime:stats() -> InputStats") }}

What the last tick cost: `deviceEvents`, `ticks`, `nodesEvaluated`, `bindingsEvaluated`, `actionsChanged`, `actionEvents`, `activeSets`, `activeLayers`.

Related: [Input Runtime](@/docs/contributing/input-runtime.md), the model behind all of this.
