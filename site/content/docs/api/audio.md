---
title: '@jess/audio'
description: 'Turn bytes into PCM, play voices on a sample clock where every command lands on its exact frame, render the mix offline, and read and write WAV.'
weight: 52
extra:
  kind: api
---

{{ api_signature(value="require('@jess/audio'): { pcm, stream, offline, output, processors, frames, seconds, tempo, readWav, loadWav, writeWav }") }}

`@jess/audio` is audio for scripts: the built-in package's Luau over `jess.audio`, the native half. The native side owns samples, voices, the sample clock, and the mix. What a file's bytes are and which frame a second lands on are Luau's, so WAV reading and writing and the time helpers live here, and a format that needs a native codec arrives as PCM bytes the same way.

The engine never calls into Luau. A script sends it commands, each with an optional `at`, an output frame, and the engine carries each one out on exactly that frame, wherever it falls in the block it is mixing. An offline context is rendered by the script, on demand, into a buffer: the same engine a device runs, which makes an offline render the test of a live one.

Things to know before the signatures:

- **Time is frames.** The clock counts output frames rendered, and moves only when the engine renders. Converting seconds is your choice, rounding included; [`frames`](#time) rounds to nearest.
- **One `Pcm`, many voices.** A `Pcm` is decoded audio, immutable, shared by every voice that plays it; playing it a hundred times copies nothing.
- **A stamp is a promise.** Two voices stamped for the same frame start on the same sample. A command without one runs on the next frame rendered, which is wherever the engine happens to be: stamp anything that has to line up.
- **Ended is final.** Once a voice runs out or is stopped, its handle names nothing: `stop` and `set` on it do nothing and return `false`.

## PCM

{{ api_signature(value="audio.pcm(data: buffer | string, options: { format: AudioSampleFormat, channels: number, rate: number }) -> AudioPcm") }}

PCM from interleaved bytes: `channels` samples per frame (1 to 8), each stored as `format`, at `rate` frames per second. Formats are little-endian: `u8` (unsigned, centred on 128), `s16`, `s24` (three bytes), `s32`, `f32`, `f64`. Integers scale so full scale is `-1` to `1`. Bytes that aren't a whole number of frames are an error.

{{ api_signature(value="pcm:rate() -> number") }}
{{ api_signature(value="pcm:channels() -> number") }}
{{ api_signature(value="pcm:frames() -> number") }}

## Streams

A stream is audio written while it plays: a bounded ring of interleaved frames that one writer fills while one voice plays it. Luau generating a sound, a worker decoding a file, anything that has audio a little at a time writes it, learns how much fit, and keeps ahead of the engine. The engine never waits for it and never calls it: when the ring runs dry the voice plays silence, counts the frames as `missed`, and carries on with whatever comes next.

{{ api_signature(value="audio.stream(options: { channels: number?, rate: number?, seconds: number? }) -> AudioStream") }}
{{ api_signature(value="audio.stream(handle: JessHandle) -> AudioStream") }}

A stream of `channels` (2) at `rate` (48000) frames per second holding `seconds` (1, at most 60) of audio. Given a handle from `stream:handle()`, the same stream, wherever the handle was sent: a worker opens the stream the main script plays, and writes it.

{{ api_signature(value="stream:write(samples: buffer) -> number") }}
{{ api_signature(value="stream:room() -> number") }}
{{ api_signature(value="stream:wait(frames: number, timeout: number?) -> boolean") }}
{{ api_signature(value="stream:finish()") }}
{{ api_signature(value="stream:stats() -> { written: number, read: number, room: number, missed: number, finished: boolean }") }}
{{ api_signature(value="stream:handle() -> JessHandle") }}

`write` takes interleaved 32-bit floats and writes as many whole frames as there is room for, returning how many. `wait` waits until there is room for `frames`, or `timeout` seconds pass; it sleeps exactly as long as playing at the stream's rate takes to make the room, so it is for a worker feeding a device, and in an offline context (where room comes only from rendering) it waits out its timeout. `finish` says the writer is done: the voice plays what is left and ends. A stream has one writer at a time and one voice: playing it again is an error.

A stream plays once, from wherever its writer is: no `offset`, no `loop`, and a `rate` of at most 4. It is resampled to the context's rate like any source.

```luau
--!strict
local audio = require '@jess/audio'

local rate = 48000
local context = audio.offline { channels = 1, rate = rate }
local stream = audio.stream { channels = 1, rate = rate, seconds = 0.1 }
context:play(stream)

-- A tone generated a block at a time, ahead of what the engine has played.
local frame = 0
for _ = 1, 100 do
  local room = stream:room()
  local chunk = buffer.create(room * 4)
  for index = 0, room - 1 do
    buffer.writef32(chunk, index * 4, 0.2 * math.sin(2 * math.pi * 330 * (frame + index) / rate))
  end

  frame += stream:write(chunk)
  context:render(1024)
end
```

`stream:stats().missed` is `0` afterwards: the generator never fell behind.

## Contexts

{{ api_signature(value="audio.offline(options: { rate: number?, channels: number?, voices: number?, buses: number?, processors: number?, commands: number?, speedOfSound: number?, handedness: ('right' | 'left')?, physical: number? }?) -> AudioContext") }}

A context the script renders. Defaults: 48000 frames per second, 2 channels, 256 voices, 64 buses (the master included), 256 processors, and room for 4096 commands waiting for their frame. `speedOfSound` (343) and `handedness` (`right`) describe the world's space; see [Space](#space).

`voices` counts voices playing or stamped for later; `commands` bounds the queue to the engine. Every count is fixed when the context is made, so the engine never allocates while it mixes. Running out of one is an error at the call that ran out (`all 256 voices are playing or scheduled`, `the audio command queue is full`), never a sound quietly not played.

{{ api_signature(value="audio.output(options: AudioOutputOptions?) -> AudioContext") }}

A context an output device renders: the platform's default output, or with `device = 'null'` the null device, which plays to nowhere in real time (for tests, and machines with no sound). It takes a context's options, runs at the device's own rate unless `rate` asks for another, and asks for `period` frames at a time as a hint. Everything else is the same as an offline context, except `render`, which is the device's: the script plays into it and the device's thread mixes as time passes, whether or not the script is running.

An output needs the `audio.output` capability, and a jess built with the `audio-device` feature; without the feature it says so. An offline context needs neither. Dropping the context stops the device, and so does the end of the generation of the script that opened it.

{{ api_signature(value="context:device() -> AudioDevice?") }}

The device rendering an output, or nil for an offline context: its `backend` (the platform API, such as `PulseAudio`, `WASAPI`, `Core Audio`, or `Null`), `rate`, `channels`, the frames it asks for at a time (`period`) and how many it buffers (`periods`), `latency` (about how many frames what you hear lags `now`), and what has happened to it: `stopped`, `rerouted` to a new default device, `interrupted` by the system, and whether the engine `failed` on its thread.

See [Audio Engine](@/docs/contributing/audio-engine.md) for how the device and the engine meet.

{{ api_signature(value="context:play(source: AudioPcm | AudioStream, options: AudioPlayOptions?) -> AudioVoice") }}

Starts a voice. Options:

| Option | Default | Meaning |
| --- | --- | --- |
| `at` | the next frame | The output frame to start on |
| `gain` | `1` | Linear gain |
| `pan` | `0` | `-1` (left) to `1` (right): equal power for a mono source, balance for a stereo one |
| `rate` | `1` | Playback rate, above 0 and at most 64; pitch moves with it |
| `offset` | `0` | The source frame to start from |
| `loop` | `false` | Loop from `loopEnd` back to `loopStart`, without a gap |
| `loopStart`, `loopEnd` | `0`, the source's end | The loop region, in source frames, the end exclusive |
| `bus` | the master | The bus it plays into |
| `sends` | none | Up to two `{ bus, level }`: buses it also plays into, at `level` (1) times its own gain |
| `fade` | `0` | Frames to fade in over, from silence to `gain` |
| `place` | none | Where it is in space, which then sets its level, pan, and pitch; see [Space](#space) |
| `priority` | `0` | How much it matters when there are more voices than the context mixes; see [Budgets](#budgets) |
| `culled` | `advance` | What it does while it isn't mixed: `advance`, `pause`, or `stop` |

A source at a different rate from the context is resampled; at the same rate and `rate = 1` it plays sample for sample. Mono mixes into any output, stereo into mono (averaged), and otherwise the channel counts must match.

{{ api_signature(value="context:render(frames: number) -> buffer") }}

Mixes the next `frames` frames and returns them as interleaved 32-bit floats, the shape `writeWav` takes. Samples aren't clipped; any sample that isn't a finite number is written as silence and counted.

{{ api_signature(value="context:now() -> number") }}

The next frame the engine renders. Anything stamped for it or later lands on its frame; anything stamped earlier runs on the next frame and counts as `late`.

{{ api_signature(value="context:rate() -> number") }}
{{ api_signature(value="context:channels() -> number") }}
{{ api_signature(value="context:stats() -> AudioStats") }}

`stats` is the engine as of its last render: `frames` rendered, voices `active` (live, mixed or not) and `allocated` (playing or waiting to start), commands carried out `late`, `nonfinite` samples silenced, commands `pending` for a later frame, voices `culled` in the last block, and voices `stolen` all told.

{{ api_signature(value="context:group(body: () -> ())") }}
{{ api_signature(value="context:room() -> number") }}

`group` runs `body` and sends every command it makes to the engine as one: the ones not stamped for later are carried out on the same frame, with no render in between. Stamp anything that has to line up and a group isn't needed; a group is for the commands that say "now". It can't arrive half-sent: there is room for it in the queue when it opens, and nothing else goes until it closes. If `body` raises, what it already sent still goes, then the error is raised again. Groups don't nest. `room` is how many more commands the queue to the engine takes.

## Voices

{{ api_signature(value="voice:stop(at: number?, fade: number?) -> boolean") }}
{{ api_signature(value="voice:set(param: 'gain' | 'pan' | 'rate', value: number, at: number?) -> boolean") }}
{{ api_signature(value="voice:state() -> 'scheduled' | 'playing' | 'ended'") }}

A stop or change stamped for a frame is carried out on that frame. One sent before the voice has started applies from its first frame: a voice stopped before its start never plays, and one whose gain is set before it starts starts at that gain. Both return whether the voice was still live; on an ended voice they do nothing. A stop is a cut on its frame unless it has a `fade`: then the voice fades to silence over that many frames and ends at the end of the fade.

{{ api_signature(value="voice:ramp(param: 'gain' | 'pan' | 'rate', value: number, frames: number, at: number?, curve: ('linear' | 'exponential')?) -> boolean") }}

Moves a parameter from wherever it is on frame `at` to `value`, reaching it on the last of `frames` frames, sample by sample, without the script doing anything more. `linear` (the default) is a straight line; `exponential` keeps one ratio every frame, which is even steps in decibels for a gain and even steps in pitch for a rate, and needs both ends above 0 (a ramp from silence starts at -100 dB). A ramp works out each frame from where it started, so a long one ends exactly on its target. Rate ramps glide the pitch.

{{ api_signature(value="voice:loop(start: number, stop: number, at: number?) -> boolean") }}
{{ api_signature(value="voice:release(at: number?) -> boolean") }}

`loop` sets the loop region from frame `at`: playback that reaches source frame `stop` carries on at `start`. `release` lets go of the loop, so playback runs on past it into whatever follows. One source can hold an intro, a loop, and an outro: start it looping, and release it on the frame the outro should follow.

{{ api_signature(value="voice:send(bus: AudioBus, level: number, at: number?) -> boolean") }}

Changes the level of a send the voice started with. A voice's sends are fixed when it starts; their levels aren't.

## Buses

A bus is where voices and other buses sum. Each block, a bus runs its processors over what it summed, applies its gain and mute, and adds the result into its parent and into each bus it sends to. Every bus has a parent, so they make a tree under the master bus, which is the output; sends add more paths, and any route that would feed a bus back into itself is refused when it's asked for, not discovered while mixing.

{{ api_signature(value="context:master() -> AudioBus") }}
{{ api_signature(value="context:bus(options: { parent: AudioBus?, gain: number?, muted: boolean? }?) -> AudioBus") }}

A new bus under `parent` (the master), at `gain` (1). Voices can play into it at once. A handle from another context is an error.

{{ api_signature(value="bus:set(param: 'gain' | 'mute', value: number, at: number?) -> boolean") }}
{{ api_signature(value="bus:route(parent: AudioBus, at: number?)") }}

`gain` is linear; `mute` is on for anything but 0. `route` moves the bus under another parent on frame `at`.

{{ api_signature(value="bus:ramp(param: 'gain', value: number, frames: number, at: number?, curve: ('linear' | 'exponential')?) -> boolean") }}

Ramps the bus's gain sample by sample, as `voice:ramp` does a voice's. A mute is a step; to fade a bus out and back, ramp its gain.

{{ api_signature(value="bus:send(target: AudioBus, options: { level: number?, tap: ('post' | 'pre')?, at: number? }?)") }}
{{ api_signature(value="bus:unsend(target: AudioBus, at: number?) -> boolean") }}

A send adds the bus into `target` as well as its parent, at `level` (1). It taps after the bus's gain and mute (`post`), or before them (`pre`), so a muted bus still feeds the room it sends to. A bus has up to four sends; sending to a target again changes the one there.

Many voices and buses sending to one bus are one input to it: a reverb on a shared bus runs once over everything sent there, not once per route.

{{ api_signature(value="bus:remove(at: number?) -> boolean") }}

Removes the bus. It has to be quiet first: a voice playing or sending into it, or a bus routed into it, makes `remove` an error that counts them (`the bus still has 2 voices and 0 buses routed into it`). A removed bus keeps running its processors until their tails have rung out, so a reverb's decay isn't cut off, then goes with them. The master is never removed.

{{ api_signature(value="bus:live() -> boolean") }}
{{ api_signature(value="bus:meter() -> { peak: number, rms: number }") }}
{{ api_signature(value="bus:latency() -> number") }}

`meter` is what the bus put out after its gain since the last read: the largest sample's magnitude and the RMS, linear, across its channels. Reading starts the next measurement. `latency` is how many frames the bus's output lags the voices that reach it, along the path whose processors delay them most. Paths of different latency summing into one bus are where compensation would line them up; the engine reports the number and doesn't yet compensate.

## Processors

{{ api_signature(value="bus:insert(kind: string, settings: { [string]: number | string }?) -> AudioProcessor") }}

Puts a processor of `kind` in the bus's chain, last or at `index` (from 1), made with `settings`: its parameters by name, and whatever the kind is made with. A bus runs up to 16. `audio.processors()` lists the kinds:

| Kind | Made with | Parameters (range, default) | Does |
| --- | --- | --- | --- |
| `gain` | | `gain` (-64 to 64, 1) | Multiplies the bus: a trim before the rest of the chain |
| `pan` | | `pan` (-1 to 1, 0) | Balances a stereo bus; on any other it does nothing |
| `filter` | `shape`: `lowpass` (the default), `highpass`, `bandpass`, `notch`, `peak`, `lowshelf`, `highshelf`, `allpass` | `frequency` (Hz, 1000), `q` (0.025 to 40, 0.707), `gain` (dB, -48 to 48, 0; the peak and shelves) | One biquad from the audio EQ cookbook. A parametric EQ is a chain of `peak` filters |
| `delay` | `longest`: seconds, up to 10 (2) | `time` (seconds, 0.25), `feedback` (-0.99 to 0.99, 0.3), `mix` (0 dry to 1 wet, 0.5) | Echoes. A time past `longest` plays at `longest` |
| `compressor` | | `threshold` (dB, -80 to 0, -20), `ratio` (1 to 50, 4), `attack` (seconds, 0.005), `release` (seconds, 0.1), `knee` (dB, 0 to 24, 6), `makeup` (dB, -24 to 24, 0) | Turns down what goes over the threshold, following its own bus or the one it `listen`s to |
| `limiter` | `lookahead`: seconds, up to 0.05 (0.005) | `ceiling` (dB, -24 to 0, -0.3), `release` (seconds, 0.05) | Keeps every sample at or under the ceiling, turning down before a peak arrives; its lookahead is its latency |
| `reverb` | | `room` (0 to 1, 0.5), `damping` (0 to 1, 0.5), `wet` (0 to 1, 0.3), `dry` (0 to 1, 0), `width` (0 to 1, 1) | A room, after Jezar's public-domain Freeverb: eight damped combs and four allpasses a side. With `dry` at 0 it suits a bus voices send to |
| `drive` | | `drive` (dB, 0 to 48, 12), `level` (dB, -48 to 12, -6), `mix` (0 to 1, 1) | Saturation: a hyperbolic tangent that rounds off where the signal would clip |

A processor's tail is how long it keeps sounding once its input stops: a delay until its echoes have died to -60 dB, a reverb until its longest comb has, a limiter its lookahead. A removed bus runs that long before it goes.

{{ api_signature(value="processor:set(param: string, value: number, at: number?) -> boolean") }}
{{ api_signature(value="processor:ramp(param: string, value: number, frames: number, at: number?, curve: ('linear' | 'exponential')?) -> boolean") }}
{{ api_signature(value="processor:listen(bus: AudioBus?, at: number?) -> boolean") }}
{{ api_signature(value="processor:remove(at: number?) -> boolean") }}
{{ api_signature(value="processor:kind() -> string") }}
{{ api_signature(value="processor:live() -> boolean") }}

`listen` gives a processor a sidechain: it hears `bus` as it leaves that bus, after its gain, instead of its own input, and still processes its own. A compressor on the music listening to the dialogue bus ducks the music whenever anyone speaks:

```luau
--!strict
local audio = require '@jess/audio'

local context = audio.offline()
local music, dialogue = context:bus(), context:bus()
local ducker = music:insert('compressor', { threshold = -30, ratio = 8, release = 0.25 })
ducker:listen(dialogue)
```

The bus a processor listens to is mixed before the processor's own, so listening to a bus that the processor's bus feeds (its parent, say) would be a loop, and is refused. A bus being listened to can't be removed until nothing listens. `listen(nil)` goes back to the processor's own input.

A name the processor doesn't have, or a value outside its range, is an error naming what it does take. A processor's parameters ramp in steps of 32 frames rather than every frame: while one ramps, the engine mixes in blocks of 32 and tells the processor where the ramp has got to at the start of each.

## Budgets

A context can hold far more voices than it mixes. With `physical` set below `voices`, each block the engine ranks the live voices by `priority`, then by how loud they would be (their gain, times what distance and cones give them in space), and mixes the top `physical`; the rest are culled for that block. A voice that was mixed holds its place against one only a little louder, so voices near the line don't flicker in and out. Ties go to the voice that started in the lower slot, so the same score culls the same voices every time.

What a culled voice does is its own: `advance` (the default) keeps its place in its source moving, silently, so a looping hum out of earshot is where it would have been when it comes back; `pause` holds it where it was; `stop` ends it, which is voice stealing. Music and dialogue that must never drop out get a high `priority`. Rules like "no more than four footsteps at once" are the application's: count them in Luau and stop the oldest.

## Space

A voice played with a `place` is in space: where it is relative to the listener sets its level, its pan, and its pitch, worked out once a block and glided across it, so a moving voice slides rather than steps. Positions, velocities, and directions are in the world's own units and axes; the only thing the context needs to know about the axes is which way they turn (`handedness`), and the only thing about the units is `speedOfSound` in them. A voice not given a `place` is not in space, and costs nothing for it.

What lies between a voice and the listener isn't the engine's to know. Occlusion, obstruction, a door half open: the application works those out however it likes (a ray against its own geometry, a room graph) and hands the result over as the place's `gain` and `lowpass`.

{{ api_signature(value="context:listener(listener: { position: vector?, forward: vector?, up: vector?, velocity: vector? }, at: number?)") }}

Moves the listener on frame `at`; fields left out stay as they were. It starts at the origin, facing `-z` with `y` up, standing still.

{{ api_signature(value="voice:place(place: AudioPlace, at: number?) -> boolean") }}

Moves a voice in space on frame `at`, changing the fields given. False for a voice that ended or was never in space; a voice can't be put into space after it starts.

| Field | Default | Meaning |
| --- | --- | --- |
| `position`, `velocity` | origin, still | Where it is, and how fast it moves (units a second) |
| `direction` | none | The way it faces, if it's directional |
| `cone` | everywhere | `{ inner, outer, outside }`: whole angles in degrees around `direction`. Full level inside `inner`, `outside` (0 to 1) beyond `outer`, in between in between |
| `distance` | `inverse` | How distance turns it down: `inverse`, `linear`, `exponential`, or `none`, clamped between `reference` and `maximum` the way OpenAL's models are |
| `reference` | `1` | The distance it's at full level within |
| `maximum` | none | The distance beyond which it gets no quieter |
| `rolloff` | `1` | How quickly it gets quieter |
| `doppler` | `1` | How much its and the listener's motion shift its pitch: 0 not at all, 1 as sound does |
| `gain` | `1` | A further gain: what the application found between it and the listener |
| `lowpass` | none | A low-pass cutoff in hertz, for the same |

The pan comes from where the voice is around the listener, left to right; stereo has no front and back, so a voice behind is panned as one in front. Doppler follows OpenAL 1.1: both velocities along the line between them, held under the speed of sound.

```luau
--!strict
local audio = require '@jess/audio'

local rate = 48000
local context = audio.offline { rate = rate }
local hum = audio.pcm(buffer.create(rate * 4), { format = 'f32', channels = 1, rate = rate })

-- A machine two metres to the right of where the listener stands, behind a thin wall.
local machine = context:play(hum, {
  loop = true,
  place = { position = vector.create(2, 0, 0), gain = 0.5, lowpass = 1200 },
})

-- The listener walks past it, and the wall ends at x = 3.
for step = 0, 59 do
  context:listener({ position = vector.create(step / 10, 0, 0) }, context:now())
  if step == 30 then machine:place({ gain = 1, lowpass = 20000 }, context:now()) end

  context:render(rate // 30)
end
```

## Time

{{ api_signature(value="audio.frames(seconds: number, rate: number) -> number") }}
{{ api_signature(value="audio.seconds(frames: number, rate: number) -> number") }}

Seconds to the nearest frame (halves round up), and frames back to seconds.

{{ api_signature(value="audio.tempo(options: { rate: number, bpm: number, beatsPerBar: number?, start: number?, changes: { { beat: number, bpm: number } }? }) -> Tempo") }}

A tempo map: beats (from 0) to frames and back, at `rate` frames per second, `bpm` beats a minute from frame `start` (0), `beatsPerBar` (4) to a bar, and later tempos from the beats `changes` names. The engine knows nothing about music; this is how a score turns "the next bar" into the frame to stamp.

{{ api_signature(value="tempo:frame(beat: number) -> number") }}
{{ api_signature(value="tempo:beat(frame: number) -> number") }}
{{ api_signature(value="tempo:bar(bar: number) -> number") }}
{{ api_signature(value="tempo:next(frame: number, every: number) -> number") }}
{{ api_signature(value="tempo:nextBar(frame: number) -> number") }}

`frame` is where a beat lands, to the nearest frame (halves up), worked out from the map each time rather than added up beat by beat, so beat ten million is where it should be. `beat` goes back, fractionally. `next` is the first frame at or after `frame` that falls on a whole number of `every` beats from the start: the next beat with 1, the next half bar with 2. `nextBar` is the next bar line.

A change of section on the next bar line, scheduled the moment the game asks for it:

```luau
--!strict
local audio = require '@jess/audio'

local rate = 48000
local context = audio.offline { rate = rate }
local tempo = audio.tempo { rate = rate, bpm = 120 }

local function tone(pitch: number): audio.Pcm
  local length = tempo:bar(2)
  local samples = buffer.create(length * 4)
  for frame = 0, length - 1 do
    buffer.writef32(samples, frame * 4, 0.2 * math.sin(2 * math.pi * pitch * frame / rate))
  end

  return audio.pcm(samples, { format = 'f32', channels = 1, rate = rate })
end

local calm = context:play(tone(220), { at = 0, loop = true })
local battle = tone(330)
context:render(70000)

-- The game asks for combat partway through a bar; the music changes on the next bar line.
local line = tempo:nextBar(context:now())
context:group(function()
  calm:stop(line, audio.frames(0.01, rate))
  context:play(battle, { at = line, loop = true })
end)
```

`line` is `96000`, the start of the third bar: however late in the bar the request comes, and however often the game gets round to it, the change lands there.

## WAV

{{ api_signature(value="audio.readWav(data: buffer | string) -> Wav") }}

Reads a WAV file's chunks: `{ format, channels, rate, frames, data, loopStart, loopEnd }`. `data` is the `data` chunk's bytes as the file has them. The format comes from the `fmt ` chunk, integer PCM or float, plain or extensible; the sample size is the container's, so 20-bit audio in 24-bit containers reads as `s24`. `loopStart` and `loopEnd` are a `smpl` chunk's first loop, the end exclusive the way `play` takes it.

{{ api_signature(value="audio.loadWav(data: buffer | string) -> (AudioPcm, Wav)") }}

`readWav`, then `pcm` over its samples. A `data` chunk that stops partway through a frame plays its whole frames.

{{ api_signature(value="audio.writeWav(samples: buffer, options: { channels: number, rate: number, format: ('s16' | 's24' | 'f32')? }) -> buffer") }}

Interleaved 32-bit floats as a WAV file, `f32` unless `format` asks for integers. Integers round to nearest and clip at full scale, and the scale is the one `pcm` reads with, so a 16-bit file read, played at its own rate, rendered, and written back as `s16` is the same bytes.

## Example

Three notes on exact frames, mixed offline and saved:

```luau
--!strict
local audio = require '@jess/audio'
local fs = require '@dream/fs'

-- A tenth of a second of a 440 Hz sine, mono, as interleaved floats.
local rate = 48000
local length = audio.frames(0.1, rate)
local samples = buffer.create(length * 4)
for frame = 0, length - 1 do
  buffer.writef32(samples, frame * 4, 0.5 * math.sin(2 * math.pi * 440 * frame / rate))
end

local tone = audio.pcm(samples, { format = 'f32', channels = 1, rate = rate })
local context = audio.offline { rate = rate, channels = 2 }

-- A fifth up, then an octave up, panned apart, a quarter second apart.
local beat = audio.frames(0.25, rate)
context:play(tone, { at = 0, pan = -0.5 })
context:play(tone, { at = beat, rate = 1.5 })
local last = context:play(tone, { at = 2 * beat, rate = 2, pan = 0.5 })
last:set('gain', 0.5, 2 * beat + audio.frames(0.025, rate))

local mix = context:render(3 * beat)
local wav = audio.writeWav(mix, { channels = 2, rate = rate, format = 's16' })
assert(fs.writeFile('target/notes.wav', wav, { createParents = true }))
```

`last:state()` is `'ended'` afterwards, and `context:now()` is `36000`.
