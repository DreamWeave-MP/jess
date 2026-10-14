---
title: 'Input Runtime'
description: 'What jess-input is, what it deliberately is not, and how devices, the action graph, sets and layers, ticks, intents, presentation, and backends work.'
weight: 75
extra:
  kind: guide
---

A GUI needs a key to mean "accept". A game needs the same key to mean "jump", a menu open on top of it needs it to mean "accept" again, and the settings screen needs to show which key that is on this keyboard and which button on that pad. Write the key-to-callback table that seems obvious on day one and every one of those needs turns into a special case in the GUI.

`jess-input` (`libs/input`) exists so input is decided before any of that exists. It knows what devices report. It knows how to turn that into named actions through a graph an author writes and a user can remap. It does not know what an action means, and it draws nothing.

## What It Is

A stack, each layer knowing only the one below it:

```text
physical device truth      Control, InputEvent, InputState        (backends, SyntheticSource)
    v
binding document           GraphSpec, NodeSpec                    (serialized, remapped)
    v
compiled action graph      ActionGraph                            (validated)
    v
sets and layers            Evaluator                              (allocation-free tick)
    v
semantic actions           ActionState, ActionEvent
    v
intents                    IntentRouter, IntentDispatch           (a consumer supplies scope)
```

`InputRuntime` ties the top four together for one application: a queue backends push to, the device state those events add up to, the installed graph, and one `tick` that drains, applies, evaluates, and returns the tick's action events. The default build has no platform dependencies; it's headless, and the tests drive it with a `SyntheticSource` whose clock moves only when told to.

## What It Is Not

- **Not a keybinding table.** A binding is a tree of nodes with types, not a key and a closure. That is the whole point, and it is why remapping, conflict detection, and binding display are functions over data instead of features.
- **Not an application.** No action name is reserved. `ui.accept`, `game.move`, and `editor.save` are conventions of whoever declares them. The crate has no idea what jumping is.
- **Not a GUI.** `presentations` says what a binding is made of and what each control is called in a glyph family. Turning `A` into an Xbox glyph is a theme's problem.
- **Not a backend.** No public type names evdev, gilrs, SDL, or a window toolkit. Backends translate into Jess's vocabulary behind a trait and a feature, and the stub compiles everywhere.
- **Not haptics.** `DeviceDescriptor` lists `OutputChannel`s and nothing drives them. The shape is reserved so haptics later attaches to the device model instead of growing a second one.
- **Not text input.** IME and text entry belong to the GUI and the platform.
- **Not an application.** `InputService` bundles one application's runtime, its intent routes by name, and a synthetic side into the unit a host ticks and a script configures; [jess-host](@/docs/contributing/application-kernel.md) ticks it in the input phase.

## Device Truth

A `Control` is one physical thing on a device, in Jess's own words: `key:KeyW`, `pointerButton:Primary`, `pointerAxis:DeltaX`, `gamepadButton:South`, `gamepadAxis:LeftStickX`, `touch:0`. Keys are named by position, after the W3C `KeyboardEvent.code` list, so `KeyW` is the same key on an AZERTY board and the keycap label is the layout's business, later. Gamepad buttons are positional too: `South` is `A` on an Xbox pad, `Cross` on Sony's, `B` on Nintendo's, and the graph binds `South`.

Axes are normalized by the backend, and the range is part of the vocabulary: sticks are `-1..=1` with `Y` up, triggers `0..=1`, pointer motion and wheels unbounded in pixels and steps. Relative axes (pointer deltas, wheels) add up over a tick and zero at its end, so an action bound to `pointerAxis:DeltaX` reads how far the pointer moved this tick.

Events and state are both there, because some consumers need edges and others need "is it held". An `InputEvent` is one change with a monotonic timestamp: connect, disconnect, button, axis, focus. `InputState` is every event so far applied in order. A disconnected device releases everything it held.

A `DeviceId` is a connection identity, allocated by the queue's registry from the backend's most stable key for the device (`DeviceIdentity`: a Bluetooth address, else the USB port, else vendor, product, and name). The same key reconnecting gets the same id; the platform decides how often that is possible, and the crate doesn't pretend otherwise. Connection identity is separate from `PresentationFamily`, the glyph family bindings are labeled with: two pads are two devices and one family.

## The Queue

Backends push through an `EventSender` from whatever thread they poll on. Nothing happens until the runtime drains the `InputQueue` at its own scheduler point. No callback runs on a backend's thread, and no Luau is ever entered from one. The future script context drains the same queue the same way, in its input phase, and reads action state after the tick.

## The Document

A `GraphSpec` declares actions, each with a `ValueType` (`digital`, `scalar`, `vector2`), and binds them in sets and layers. Each binding is a tree of `NodeSpec`s:

| Node | Inputs | Output | Semantics |
| --- | --- | --- | --- |
| `sourceButton` | | digital | A button, on any device with it or one device |
| `sourceAxis` | | scalar | An axis, in its control's range |
| `constant` | | scalar | A fixed value |
| `invert` | any | same | Not, or negated |
| `scale` | scalar or vector2 | same | Times `factor` |
| `deadzone` | scalar or vector2 | same | Zero at or below `inner`, one at or above `outer`, rescaled between; radial on a vector |
| `threshold` | scalar | digital | On at magnitude `level` or more |
| `chord` | 2+ digital | digital | Every member held |
| `any` | 1+ digital | digital | Any member held |
| `gate` | any, digital | input's | The input while the condition is on, else the type's zero |
| `tap` | digital | digital | On for one tick when released within `within_ms` |
| `hold` | digital | digital | On once held for `duration_ms`, until released |
| `axisFromButtons` | digital, digital | scalar | `-1`, `0`, or `1` |
| `vector2` | scalar, scalar | vector2 | |
| `actionActive` | | digital | Whether another action is active, this tick |
| `actionValue` | | the action's | Another action's value, this tick |

Every input is checked against what the node takes and nothing is coerced. A binding must produce its action's type. Thresholds are whole milliseconds on the monotonic clock. The document serializes as JSON, fields in declaration order and lists in author order, so the same document is the same text; an unknown node type or field is an error naming it, never a binding dropped:

```json
{
  "action": "quickSave",
  "input": {
    "type": "chord",
    "members": [
      { "type": "sourceButton", "control": "key:ControlLeft", "device": null },
      { "type": "sourceButton", "control": "key:KeyS", "device": null }
    ]
  }
}
```

`ActionGraph::compile` checks the whole document and reports every problem at once, each placed in Jess terms:

```text
acceptance.jsl:12:1, set "gameplay" (from @jess/acceptance), action "quickSave", binding 1, at chord: chord's members[1] must be digital, and it is scalar
action "a": actions read each other in a cycle: a -> b -> c -> a
```

The error is data too (`GraphError` has the owner, package, action, binding index, node path, and origin as fields), so a settings screen can point at the binding instead of printing the sentence.

## Semantics Worth Stating

**Chords** turn on when the last member goes down and off when any member comes up, whatever order the members were pressed in. Members are not consumed: a plain binding to `S` still fires under `Ctrl+S`. Chord precedence is a later decision, made deliberately, not a default.

**Tap and hold** measure from the device's own event timestamps where a source has them, so a hold threshold doesn't drift with the tick rate. A tap is a release within `within_ms` of the press, on for exactly one tick. A hold turns on when the threshold is crossed, on a tick, and stays on until release; a release after the tap window is not a tap. Tap 200 ms and hold 350 ms on one button tell themselves apart with nothing else.

**Gates** make dependency structural: `sprint` is `gate(ShiftLeft, actionActive(move))`. Actions are evaluated after the actions they read, so a gate sees this tick's value; a cycle of `actionActive`/`actionValue` is a compile error naming the cycle.

**Sets** are modes. Any number can be active. An action bound in several active sets, or several times in one, takes the value furthest from rest, the earliest binding winning a tie, and that binding is the state's `binding`: the answer to "why did this fire".

**Layers** are a stack. For each action a layer names, the topmost pushed layer that names it decides: its own bindings replace every set's and every lower layer's, or `masks` silences the action. Actions a layer doesn't name pass through. The policy is small enough to hold in one hand, which is why it's the one shipped.

**Going live.** A binding is live while its owner decides its action. When a binding goes live, it starts from rest, and a button that was already held when the sets or layers last changed stays invisible to it until released. Opening a menu with the key that also accepts in the menu does not accept; a key pressed after the menu opened does. This is the bug every input system ships once.

**A tick sees state as it is.** A press and its release that both land between two ticks are invisible to the graph. Tick at least as often as the shortest input you care about.

## Evaluation

An `Evaluator` is the per-graph state that changes: every node's value and when it last rose, which sets are active, which layers are pushed, every action's state. All of it is allocated when the evaluator is made. Compilation lays each binding's nodes out children first, so evaluating a binding is one pass over a contiguous range, and a tick allocates nothing.

`ActionState` carries `value`, `active`, `pressed_this_tick`, `released_this_tick`, `changed_this_tick`, `since`, `binding`, and `device`, so nobody diffs frames by hand. `ActionEvent`s are the tick's edges (`Pressed`, `Released`, and `Changed` for scalars and vectors), each stamped with the `Generation` that produced it. `EvaluationStats` says what the tick cost: nodes and bindings evaluated, actions changed, events, active sets and layers, and the evaluation time. Make the cost visible first.

## Generations and Remapping

Every install, remap, restore, and uninstall replaces the graph whole and bumps the runtime's `Generation`. Ids belong to one generation's graph; hold names across a replacement, and drop events stamped with an older generation, which the intent router does for you. A document that doesn't compile replaces nothing: the last good graph keeps running.

Remapping is editing the document: `bindings_for`, `add_binding`, `replace_binding`, `remove_binding` on `GraphSpec`, then `InputRuntime::remap` compiles the result and keeps the package's defaults for `restore_defaults`. A user's saved document goes through `apply_bindings`, which refuses one that declares different actions than the installed defaults. The future settings screen is a consumer of exactly these calls.

## Intents

An action is interpreted input; an intent is what it means to a target; a command is an application operation a menu or a script may invoke just as well. `IntentRouter` is the middle step and nothing more: routes from `(action, edge)` to an `IntentId` from an `IntentRegistry`, and a dispatch list with the action's provenance (action, binding, device, value, timestamp, generation) and an `IntentScope` of opaque tokens the consumer supplied. No focus, no bubbling, no command table. The GUI pass will pressure this; it was kept small on purpose.

## Presentation and Conflicts

`presentations(spec, action, family)` returns each binding a person using that family can press, as a `Presentation` tree: controls with the family's semantic label (`A`, `Cross`, `LB`, `L1`, `KeyW`), chords, taps and holds with their thresholds, axes, vectors, gates as `While`. Transforms that don't change what to press are left out; the presentation of a deadzoned stick is the stick. The GUI decides what any of it looks like.

`conflicts(spec)` finds the check that is always a mistake: two bindings in one set or layer firing different actions from the same press, chord, tap, hold, or axis. It deliberately says nothing about `Ctrl+S` beside `S`, a tap beside a hold, or the same button in two sets, because whether those are conflicts depends on which sets and layers an application activates together. The document exposes the structure a tool needs to judge that itself.

## Backends

| Target | Feature | Backend | Devices |
| --- | --- | --- | --- |
| Linux | `linux-evdev` | `/dev/input` through evdev | keyboards, pointers, gamepads |
| macOS, Windows | `gilrs` | gilrs | gamepads |
| anything else, or no feature | | `StubBackend` | none |

Linux reads evdev directly rather than through gilrs, which would pull libudev and pkg-config into every build. On macOS and Windows the window system owns keyboards and pointers, and whoever owns the window pushes those through the same `EventSender`.

What controllers actually do, and what the evdev backend does about it: shoulder and face buttons arrive as `EV_KEY`; a D-pad arrives as `BTN_DPAD_*` keys on some pads and as hat axes (`ABS_HAT0X`/`ABS_HAT0Y`) on others, and a few send both, so hats become D-pad button events and a pad with hats has its D-pad keys ignored; axes report raw integers in a range only the device knows, so each is normalized from its own `EVIOCGABS` minimum and maximum, with no deadzone applied because that is the graph's decision; the kernel's gamepad specification decides which `ABS_*` is which stick and trigger, and `Y` is inverted to point up. Devices are rescanned about once a second, a failed read is a disconnect, and a machine where no device can be opened is a machine with no devices, not an error. Known gap: the kernel names `BTN_NORTH`/`BTN_WEST` after the letters on an Xbox pad, which is positionally wrong for it, and the backend reports what the driver says; a mapping database is later work.

## Capabilities

The runtime enforces none, because a native caller holding an `InputRuntime` already has everything. `jess_input::capability` names what the script boundary checks, so a package that only consumes actions never needs to be trusted with raw keystrokes:

| Capability | Grants |
| --- | --- |
| `input.observe` | Raw device truth: `InputState`, descriptors, device events |
| `input.actions` | Action state and events, intent dispatches, binding presentation |
| `input.configure` | Installing, remapping, activating sets, pushing layers, routing, synthetic events |

`jess.input` checks them through `jess_runtime::require_capability` on every runtime method, whenever the script runs under a `ScriptDomain`; a denial is the broker's, `capability 'input.actions' denied to domain#1 gen#1 context#2`, attributed and diagnosed. Outside any domain (a plain `jess run`, a test) nothing holds authority and every call succeeds. The table of which method needs which is on [jess.input](@/docs/api/input.md).

## Tests

The acceptance fixture (`libs/input/tests/gameplay.rs`) is the handoff's: a `gameplay` set with WASD movement, `Ctrl+S`, a tap and a 350 ms hold on one button, a gated sprint, and an `inventory` layer that rebinds one action and masks another, driven entirely by synthetic events on a test clock. It checks the vector, the chord's edges, tap against hold, timing that ignores wall time, the gate, deactivating the set, the layer, the going-live rule, the disconnect release, that nothing allocates per tick, and that the document round-trips to a golden (`tests/golden/gameplay.json`, `JESS_BLESS=1`). `tests/intents.rs` routes `ui.accept` to `ui.activate` with provenance intact and drops a stale generation. `tests/graph.rs` pins every compile error's message. The same fixture runs from Luau in `libs/lua/tests/contracts/input/acceptance.jsl`; see [jess.input](@/docs/api/input.md).
