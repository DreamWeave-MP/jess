--!strict
-- The input acceptance fixture (the input handoff's sections 39 and 40), driven from Luau: a
-- gameplay set with WASD movement, a Ctrl+S chord, a tap and a hold on one button, a gated sprint,
-- an inventory layer over it, and ui.accept routed to an intent. Every timestamp comes from the
-- runtime's own clock, which moves only when the script says so.
local oracle, env = ...
local input = jess.input

local function key(name: string): InputNode return input.key(name) end

local south = input.gamepadButton 'South'
local graph = input.graph {
  actions = {
    { name = 'move', valueType = 'vector2' },
    { name = 'quickSave', valueType = 'digital' },
    { name = 'interact', valueType = 'digital' },
    { name = 'contextMenu', valueType = 'digital' },
    { name = 'sprint', valueType = 'digital' },
    { name = 'ui.accept', valueType = 'digital' },
  },

  sets = {
    {
      name = 'gameplay',
      owner = '@jess/acceptance',
      bindings = {
        {
          action = 'move',
          input = input.vector2(
            input.axisFromButtons(key 'KeyA', key 'KeyD'),
            input.axisFromButtons(key 'KeyS', key 'KeyW')
          ),
        },
        { action = 'quickSave', input = input.chord { key 'ControlLeft', key 'KeyS' } },
        { action = 'interact', input = input.tap(south, 0.2) },
        { action = 'contextMenu', input = input.hold(south, 0.35) },
        { action = 'sprint', input = input.gate(key 'ShiftLeft', input.actionActive 'move') },
      },
    },

    {
      name = 'ui',
      owner = '@jess/acceptance',
      bindings = { { action = 'ui.accept', input = input.any { south, key 'Enter' } } },
    },
  },

  layers = {
    {
      name = 'inventory',
      owner = '@jess/acceptance',
      bindings = { { action = 'interact', input = key 'KeyE' } },
      masks = { 'sprint' },
    },
  },
}

local runtime = input.runtime()
runtime:install(graph)
runtime:setActive 'gameplay'
runtime:route('ui.accept', 'pressed', 'ui.activate')
local keyboard, pad = runtime:connect 'keyboard', runtime:connect 'gamepad'
runtime:tick()

local function step(seconds: number): { InputActionEvent }
  runtime:advance(seconds)
  return runtime:tick()
end

local function edges(events: { InputActionEvent }, action: string): { string }
  local kinds = {}
  for _, event in events do
    if event.action == action then table.insert(kinds, event.kind) end
  end

  return kinds
end

-- WASD is a vector; opposite keys cancel.
runtime:press(keyboard, 'key:KeyW')
step(0.016)
local moveUp = (runtime:action 'move' :: InputActionState).value
runtime:press(keyboard, 'key:KeyD')
runtime:press(keyboard, 'key:KeyA')
step(0.016)
local moveUpCancelled = (runtime:action 'move' :: InputActionState).value
runtime:release(keyboard, 'key:KeyW')
runtime:release(keyboard, 'key:KeyD')
runtime:release(keyboard, 'key:KeyA')
local moveStop = edges(step(0.016), 'move')

-- The chord fires when the last member lands, once, and breaks when any member lifts.
runtime:press(keyboard, 'key:ControlLeft')
local halfChord = edges(step(0.016), 'quickSave')
runtime:press(keyboard, 'key:KeyS')
local chord = edges(step(0.016), 'quickSave')
local chordHeld = edges(step(0.016), 'quickSave')
local chordState = runtime:action 'quickSave' :: InputActionState
runtime:release(keyboard, 'key:ControlLeft')
local chordBroken = edges(step(0.016), 'quickSave')
runtime:release(keyboard, 'key:KeyS')
step(0.016)

-- A short press taps; a long one holds, and never taps.
runtime:press(pad, 'gamepadButton:South')
step(0.1)
runtime:release(pad, 'gamepadButton:South')
local tap = edges(runtime:tick(), 'interact')
local tapOver = edges(step(0.016), 'interact')
runtime:press(pad, 'gamepadButton:South')
step(0.3)
local notYetHeld = (runtime:action 'contextMenu' :: InputActionState).active
local held = edges(step(0.1), 'contextMenu')
local heldStill = edges(step(0.5), 'contextMenu')
runtime:release(pad, 'gamepadButton:South')
local heldReleased = runtime:tick()

-- Sprint only while moving, Shift held throughout.
runtime:press(keyboard, 'key:ShiftLeft')
step(0.016)
local sprintStill = (runtime:action 'sprint' :: InputActionState).active
runtime:press(keyboard, 'key:KeyW')
step(0.016)
local sprintMoving = (runtime:action 'sprint' :: InputActionState).active
runtime:release(keyboard, 'key:KeyW')
step(0.016)
local sprintStopped = (runtime:action 'sprint' :: InputActionState).active
runtime:release(keyboard, 'key:ShiftLeft')
step(0.016)

-- The layer binds interact to E and masks sprint; popping it restores the set.
runtime:pushLayer 'inventory'
step(0.016)
runtime:press(keyboard, 'key:KeyE')
local layered = edges(step(0.016), 'interact')
runtime:press(keyboard, 'key:KeyW')
runtime:press(keyboard, 'key:ShiftLeft')
step(0.016)
local maskedSprint = (runtime:action 'sprint' :: InputActionState).active
local layeredMove = (runtime:action 'move' :: InputActionState).active
runtime:release(keyboard, 'key:KeyE')
runtime:release(keyboard, 'key:KeyW')
runtime:release(keyboard, 'key:ShiftLeft')
runtime:popLayer 'inventory'
step(0.016)
runtime:press(keyboard, 'key:KeyE')
step(0.016)
local unlayered = (runtime:action 'interact' :: InputActionState).active
runtime:release(keyboard, 'key:KeyE')
step(0.016)

-- Deactivating the set silences it; the ui set routes an intent with the pad as its source.
runtime:setActive('gameplay', false)
runtime:setActive 'ui'
runtime:press(keyboard, 'key:KeyW')
runtime:press(pad, 'gamepadButton:South')
local silenced = step(0.016)
local intents = runtime:intents()
runtime:release(pad, 'gamepadButton:South')
step(0.016)
local noIntentOnRelease = #runtime:intents()

-- The document round-trips through JSON and through the spec table.
local json = graph:toJson()
local roundTrip = input.fromJson(json):toJson() == json
local respec = input.graph(graph:spec()):toJson() == json

local xbox = graph:presentations('contextMenu', 'xbox')
local playstation = graph:presentations('contextMenu', 'playstation')
local conflicting = input.graph {
  actions = {
    { name = 'a', valueType = 'digital' },
    { name = 'b', valueType = 'digital' },
  },

  sets = {
    {
      name = 's',
      owner = 'probe',
      bindings = {
        { action = 'a', input = input.chord { key 'ControlLeft', key 'KeyS' } },
        { action = 'b', input = input.chord { key 'KeyS', key 'ControlLeft' } },
      },
    },
  },
}

return {
  moveUp = moveUp,
  moveUpCancelled = moveUpCancelled,
  moveStop = moveStop,
  halfChord = halfChord,
  chord = chord,
  chordHeld = chordHeld,
  chordDevice = chordState.device == keyboard,
  chordBroken = chordBroken,
  tap = tap,
  tapOver = tapOver,
  notYetHeld = notYetHeld,
  held = held,
  heldStill = heldStill,
  heldReleased = edges(heldReleased, 'contextMenu'),
  heldNotTapped = edges(heldReleased, 'interact'),
  sprint = { still = sprintStill, moving = sprintMoving, stopped = sprintStopped },
  layered = layered,
  maskedSprint = maskedSprint,
  layeredMove = layeredMove,
  unlayered = unlayered,
  silenced = edges(silenced, 'move'),
  intents = intents,
  intentDevice = intents[1].device == pad,
  noIntentOnRelease = noIntentOnRelease,
  roundTrip = roundTrip,
  respec = respec,
  actions = graph:actions(),
  bindings = graph:bindings 'interact',
  xbox = xbox,
  playstationLabel = playstation[1].shape.input.label,
  keyboardSees = #graph:presentations('contextMenu', 'keyboardMouse'),
  conflicts = conflicting:conflicts(),
  noConflicts = #graph:conflicts(),
  devices = #runtime:devices(),
  now = runtime:now(),
  stats = runtime:stats().ticks,
  errors = {
    badType = oracle.raises(
      'argument',
      'the action is vector2, and this binding produces digital',
      input.graph,
      {
        actions = { { name = 'move', valueType = 'vector2' } },
        sets = {
          { name = 's', owner = 'probe', bindings = { { action = 'move', input = key 'KeyA' } } },
        },
      }
    ),
    cycle = oracle.raises(
      'argument',
      'actions read each other in a cycle: a -> b -> a',
      input.graph,
      {
        actions = {
          { name = 'a', valueType = 'digital' },
          { name = 'b', valueType = 'digital' },
        },

        sets = {
          {
            name = 's',
            owner = 'probe',
            bindings = {
              { action = 'a', input = input.actionActive 'b' },
              { action = 'b', input = input.actionActive 'a' },
            },
          },
        },
      }
    ),
    unknownKey = oracle.raises('argument', 'no key of that name', input.key, 'Enter2'),
    unknownNode = oracle.raises(
      'parse',
      'unknown variant `doubleTap`',
      input.fromJson,
      (string.gsub(json, '"chord"', '"doubleTap"'))
    ),
    unknownSet = oracle.raises(
      'missing',
      'has no set "menu"',
      function() runtime:setActive 'menu' end
    ),
    badDuration = oracle.raises('argument', 'takes a duration in seconds', input.hold, south, -1),
  },
}
