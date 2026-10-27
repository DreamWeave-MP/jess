---
title: '@jess/audio'
description: 'Turn bytes into PCM, play voices on a sample clock where every command lands on its exact frame, render the mix offline, read and write WAV, and load or stream MP3, Vorbis, and Opus.'
weight: 52
extra:
  kind: api
---

{{ api_signature(value="require('@jess/audio'): { pcm, stream, offline, output, processors, frames, seconds, tempo, readWav, loadWav, writeWav, load, decoder, feed }") }}

`@jess/audio` is audio for scripts: the built-in package's Luau over `jess.audio`, the native half. The native side owns samples, voices, the sample clock, and the mix. What a file's bytes are and which frame a second lands on are Luau's, so WAV reading and writing and the time helpers live here. MP3, Ogg Vorbis, and Ogg Opus need codecs, which are [`@dream/audio`](@/docs/api/l3i.md)'s; this module loads them whole or streams them.

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

A stream is audio written while it plays: a bounded ring of interleaved frames that one writer fills while one voice plays it. Luau generating a sound, a worker decoding a file, anything that has audio a little at a time writes it, learns how much fit, and keeps ahead of the engine. The engine never waits for it and never calls it: when the ring runs dry the voice plays silence, counts the frames as `missed`, and carries on with whatever comes next. A stream a [feed](#compressed-files) writes keeps time instead: what the feed was late with is let go when it comes, not played late, and `late` counts the silence a [seek](#voices) played before the feed got there.

{{ api_signature(value="audio.stream(options: { channels: number?, rate: number?, seconds: number? }) -> AudioStream") }}
{{ api_signature(value="audio.stream(handle: JessHandle) -> AudioStream") }}

A stream of `channels` (2) at `rate` (48000) frames per second holding `seconds` (1, at most 60) of audio. Given a handle from `stream:handle()`, the same stream, wherever the handle was sent: a worker opens the stream the main script plays, and writes it.

{{ api_signature(value="stream:write(samples: buffer, frames: number?) -> number") }}
{{ api_signature(value="stream:room() -> number") }}
{{ api_signature(value="stream:wait(frames: number, timeout: number?) -> boolean") }}
{{ api_signature(value="stream:finish()") }}
{{ api_signature(value="stream:stats() -> { written: number, read: number, room: number, missed: number, late: number, finished: boolean }") }}
{{ api_signature(value="stream:handle() -> JessHandle") }}

`write` takes interleaved 32-bit floats and writes as many whole frames as there is room for, returning how many; given `frames`, it takes only that many from the start of the buffer, so a block that came back part full needs no copy. `wait` waits until there is room for `frames`, or `timeout` seconds pass; it sleeps exactly as long as playing at the stream's rate takes to make the room, so it is for a worker feeding a device, and in an offline context (where room comes only from rendering) it waits out its timeout. `finish` says the writer is done: the voice plays what is left and ends. A stream has one writer at a time and one voice: playing it again is an error.

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

{{ api_signature(value="audio.offline(options: { rate: number?, channels: number?, voices: number?, nodes: number?, connections: number?, commands: number?, speedOfSound: number?, handedness: ('right' | 'left')?, physical: number? }?) -> AudioContext") }}

A context the script renders. Defaults: 48000 frames per second, 2 channels, 256 voices, 256 nodes (buses and processors, the master included), 1024 connections between them, and room for 4096 commands waiting for their frame. `speedOfSound` (343) and `handedness` (`right`) describe the world's space; see [Space](#space).

`voices` counts voices playing or stamped for later; `commands` bounds the queue to the engine, and again the commands stamped for later that the engine holds until their frames. A command for now never waits behind those, so a schedule full of later work can't keep a `stop` from landing. Every count is fixed when the context is made, so the engine never allocates while it mixes. Running out of one is an error at the call that ran out (`all 256 voices are playing or scheduled`, `the audio command queue is full`, `all 4096 commands the engine can hold for later are waiting for their frames`), never a sound quietly not played.

{{ api_signature(value="audio.output(options: AudioOutputOptions?) -> AudioContext") }}

A context an output device renders: the platform's default output, or with `device = 'null'` the null device, which plays to nowhere in real time (for tests, and machines with no sound). It takes a context's options, runs at the device's own rate unless `rate` asks for another, and asks for `period` frames at a time as a hint. Everything else is the same as an offline context, except `render`, which is the device's: the script plays into it and the device's thread mixes as time passes, whether or not the script is running.

An output needs the `audio.output` capability, and a jess built with the `audio-device` feature, which is on unless a build turned it off; without it, `output` says so. An offline context needs neither.

An output plays until it is closed or the runtime that opened it ends, whether or not the script still holds it: music started from a function that returned keeps playing after its handles are collected. A domain's restart ends its runtime, and with it every output that generation opened.

{{ api_signature(value="context:close()") }}

Stops an output's device and drops every voice it played. Commands after it are errors; `now` and the rest of what the context knows stay readable, frozen where the device left them. Closing twice is nothing, and an offline context has nothing to close: it goes when nothing holds it.

{{ api_signature(value="context:device() -> AudioDevice?") }}

The device rendering an output, or nil for an offline context: its `backend` (the platform API, such as `PulseAudio`, `WASAPI`, `Core Audio`, or `Null`), `rate`, `channels`, the frames it asks for at a time (`period`) and how many it buffers (`periods`), `latency` (about how many frames what you hear lags `now`), and what has happened to it: `stopped`, `rerouted` to a new default device, `interrupted` by the system, whether the engine `failed` on its thread, and whether the output was `closed`.

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
| `bus` | the master | The node it plays into: a bus, or a processor only it goes through |
| `sends` | none | Up to fifteen `{ bus, level }`: nodes it also plays into, at `level` (1) times its own gain |
| `fade` | `0` | Frames to fade in over, from silence to `gain` |
| `place` | none | Where it is in space, which then sets its level, pan, and pitch; see [Space](#space) |
| `priority` | `0` | How much it matters when there are more voices than the context mixes; see [Budgets](#budgets) |
| `culled` | `advance` | What it does while it isn't mixed: `advance`, `pause`, or `stop` |

A source at a different rate from the context is resampled; at the same rate and `rate = 1` it plays sample for sample. A source read faster than the context plays it, by its rate or its `rate`, is filtered first, so nothing too high for the output folds back as a lower tone. Mono mixes into any output, stereo into mono (averaged), and otherwise the channel counts must match.

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

{{ api_signature(value="voice:route(node: AudioNode, at: number?) -> boolean") }}
{{ api_signature(value="voice:send(node: AudioNode, level: number, at: number?) -> boolean") }}
{{ api_signature(value="voice:unsend(node: AudioNode, at: number?) -> boolean") }}

Where a voice plays can change while it plays, on a frame. `route` plays it into `node` instead of where it plays first, keeping its sends. `send` plays it into `node` as well, at `level`, or changes the level it plays there at; `unsend` stops it playing there. A voice plays into up to sixteen nodes at once. None of this is a graph change: moving a voice compiles nothing and waits behind no change to the graph. To move a voice on exactly the frame the graph changes around it, connect and disconnect it in that change's [patch](#patches).

{{ api_signature(value="context:voice(source: AudioPcm | AudioStream, options: AudioPlayOptions?) -> AudioVoice") }}
{{ api_signature(value="voice:pause(at: number?) -> boolean") }}
{{ api_signature(value="voice:play(at: number?) -> boolean") }}
{{ api_signature(value="voice:seek(frame: number, at: number?) -> boolean") }}
{{ api_signature(value="voice:position() -> number?") }}
{{ api_signature(value="voice:seekLead() -> number?") }}

`context:voice` makes a voice as `play` does, paused: it holds its slot and plays nothing until `voice:play`. `pause` holds a voice where it is on frame `at`, and `play` plays it on from there; a paused voice is neither mixed nor counted against the context's [budget](#budgets), and its `state` is `paused`. `seek` moves it to source frame `frame` on frame `at`. A voice of PCM goes there at once, on a whole frame, so the first sample after a seek is the source's own (the resampler keeps no history to reset). A seek past the end of a loop the voice is in lands where playing through the loop would have taken it. `position` is the source frame the voice is at, as of the last render, or where a voice not started yet will start; it is nil once the voice has ended.

A stream seeks only when a [feed](#compressed-files) decodes into it. A seek stamped at least `voice:seekLead()` frames ahead (half the stream) is prepared: the feed is asked that long before the seek's frame, the voice plays on from what is already decoded until then, and on the frame it moves to the new one without a gap. A seek stamped closer, or for now, asks the feed at once. If the feed still hasn't decoded from the new frame when the seek's frame comes, the voice keeps time: it plays silence (counted as `missed` and as `late` in the stream's stats), its position moves on as if it were playing, and when the frames come it joins where it would have got to. A feed's stream keeps time through any underrun that way; a script's plays late frames after the gap. One seek waits on a feed at a time: a seek asked for while another is still waiting replaces it, and the one replaced plays silence until the next lands. A stream a script writes has nothing to move, and `seek` says so with an error. A stream's position counts the frames it has played from where it started or last sought to.

## Nodes

Everything a voice plays through is a node of the context's graph: a bus, or a processor. A bus is a mixer node: it sums what reaches it, runs its insert chain over the sum, applies its gain and mute, and adds the result into its route (the node it's routed into) and each node it sends to. The master bus is the output. Any route or send that would feed a node back into itself is refused when it's asked for, not discovered while mixing. Every change to the graph is checked and compiled on the script's side and reaches the engine whole, on its frame; one that's wrong changes nothing. [Audio Graph](@/docs/contributing/audio-graph.md) has the model.

Changes to the graph (making, routing, sending, inserting, removing) take effect in the order they're made. A change can be stamped for a later frame, but not for one earlier than a change still waiting: that's an error naming the frame it would overtake (`a graph change waits for frame 96000`). Playing, stopping, and setting voices and parameters are not graph changes, and keep their own stamps.

{{ api_signature(value="context:master() -> AudioNode") }}
{{ api_signature(value="context:bus(options: { parent: AudioNode?, gain: number?, muted: boolean?, channels: number? }?) -> AudioNode") }}

A new bus routed into `parent` (the master), at `gain` (1), with `channels` if fewer than the context's. Voices can play into it at once. A handle from another context is an error.

{{ api_signature(value="node:set(param: string, value: number, at: number?) -> boolean") }}
{{ api_signature(value="node:route(parent: AudioNode, at: number?)") }}

A bus has two parameters: `gain`, linear, and `mute`, on for anything but 0. `route` moves the node's route to another node on frame `at`, keeping its sends.

{{ api_signature(value="node:ramp(param: string, value: number, frames: number, at: number?, curve: ('linear' | 'exponential')?) -> boolean") }}

Ramps a parameter as `voice:ramp` does a voice's. An audio-rate parameter (a bus's gain) moves every sample; a control-rate one moves every 32 frames: while one ramps, the processor hears where it has got to on every frame that is a multiple of 32, however the render is cut. A mute is a step; to fade a bus out and back, ramp its gain.

{{ api_signature(value="node:cancel(param: string, at: number?) -> boolean") }}

From frame `at`, drops every change of the parameter sent before the cancel that is still waiting for its frame, and stops a ramp of it where it has got to. Changes sent after the cancel stand, so cancel-then-ramp replaces a schedule.

{{ api_signature(value="node:params() -> { { name: string, min: number, max: number, default: number, unit: string, rate: 'control' | 'audio' } }") }}

A node's parameters, in order: range, default, unit (`Hz`, `dB`, `s`, or empty for a plain number), and rate.

{{ api_signature(value="node:send(target: AudioNode, options: { level: number?, tap: ('post' | 'pre')?, at: number? }?)") }}
{{ api_signature(value="node:unsend(target: AudioNode, at: number?) -> boolean") }}

A send adds the node into `target` as well as its route, at `level` (1). It taps after the node's processing, a bus's gain and mute (`post`), or before it (`pre`), so a muted bus still feeds the room it sends to. Sending to a target again changes the send there. A context has room for `connections` of them, routes included.

Many voices and nodes sending to one bus are one input to it: a reverb on a shared bus runs once over everything sent there, not once per route.

{{ api_signature(value="node:remove(at: number?) -> boolean") }}

Removes the node. Nothing may feed it first: a voice playing or sending into it, or a node routed or sending into it, makes `remove` an error that counts them (`the node still has 2 voices and 0 connections feeding it`). Its own route and sends go with it, and so does a sidechain it is the key of. A removed node whose processors have a tail keeps running until the tail has rung out, so a reverb's decay isn't cut off, then goes with its insert chain. The master is never removed.

{{ api_signature(value="node:live() -> boolean") }}
{{ api_signature(value="node:meter() -> { peak: number, rms: number }") }}
{{ api_signature(value="node:latency() -> number") }}
{{ api_signature(value="node:kind() -> string") }}

`meter` is what a bus put out after its gain since the last read: the largest sample's magnitude and the RMS, linear, across its channels. Reading starts the next measurement; another node has no meter. `latency` is how many frames the node's output lags the voices that reach it, along the path whose processors delay them most. Where paths of different latency meet, the engine lines them up: every path into a node, a sidechain key, and what voices play into it directly wait for the latest, so a voice heard through a limited group and through a dry one stays one voice.

## Processors

{{ api_signature(value="node:insert(kind: string, settings: { [string]: number | string }?) -> AudioNode") }}

Puts a processor of `kind` in the node's insert chain, last or at `index` (from 1), made with `settings`: its parameters by name, and whatever the kind is made with. The chain runs in place over what the bus summed, before its gain. An inserted processor is a node like any other: it is set, ramped, listened to, and removed through its handle. `audio.processors()` lists the kinds, including any the program running the script added:

| Kind | Made with | Parameters (range, default) | Does |
| --- | --- | --- | --- |
| `mixer` | | `gain` (-64 to 64, 1, audio rate), `mute` (0 or 1) | A bus's own processing |
| `gain` | | `gain` (-64 to 64, 1, audio rate) | Multiplies what passes: a trim, or an amplifier something modulates |
| `pan` | | `pan` (-1 to 1, 0) | Balances a stereo signal; on any other it does nothing |
| `filter` | `shape`: `lowpass` (the default), `highpass`, `bandpass`, `notch`, `peak`, `lowshelf`, `highshelf`, `allpass` | `frequency` (Hz, 1000), `q` (0.025 to 40, 0.707), `gain` (dB, -48 to 48, 0; the peak and shelves) | One biquad from the audio EQ cookbook. A parametric EQ is a chain of `peak` filters |
| `delay` | `longest`: seconds, up to 10 (2) | `time` (seconds, 0.25), `feedback` (-0.99 to 0.99, 0.3), `mix` (0 dry to 1 wet, 0.5) | Echoes. A time past `longest` plays at `longest` |
| `compressor` | | `threshold` (dB, -80 to 0, -20), `ratio` (1 to 50, 4), `attack` (seconds, 0.005), `release` (seconds, 0.1), `knee` (dB, 0 to 24, 6), `makeup` (dB, -24 to 24, 0) | Turns down what goes over the threshold, following its own input or the key it `listen`s to |
| `limiter` | `lookahead`: seconds, up to 0.05 (0.005) | `ceiling` (dB, -24 to 0, -0.3), `release` (seconds, 0.05) | Keeps every sample at or under the ceiling, turning down before a peak arrives; its lookahead is its latency |
| `reverb` | | `room` (0 to 1, 0.5), `damping` (0 to 1, 0.5), `wet` (0 to 1, 0.3), `dry` (0 to 1, 0), `width` (0 to 1, 1) | A room, after Jezar's public-domain Freeverb: eight damped combs and four allpasses a side. With `dry` at 0 it suits a bus voices send to |
| `drive` | | `drive` (dB, 0 to 48, 12), `level` (dB, -48 to 12, -6), `mix` (0 to 1, 1) | Saturation: a hyperbolic tangent that rounds off where the signal would clip |
| `crossfade` | | `mix` (0 to 1, 0.5, audio rate) | Two inputs, `a` and `b`: all of `a` at 0, all of `b` at 1. A dry path into one and a wet one into the other is a wet and dry control |
| `oscillator` | `wave`: `sine` (the default), `triangle`, `saw`, `square` | `frequency` (Hz, 0 to 96000, 440, audio rate), `gain` (-64 to 64, 1, audio rate) | A source: a periodic wave, its saw and square band-limited. Modulating its own frequency or gain is frequency or amplitude modulation |
| `envelope` | | `attack`, `decay`, `release` (seconds, 0.01, 0.1, 0.3), `sustain` (0 to 1, 0.7), `gate` (0 or 1) | A source: from 0 to 1 and back in straight lines as its `gate` opens and closes, for modulating an amplifier or a cutoff |

Parameters are control rate unless the table says otherwise. A source has no inputs: it writes the same signal on every channel of its node, so one made with `channels = 1` costs least. A processor's tail is how long it keeps sounding once its input stops: a delay until its echoes have died to -60 dB, a reverb until its longest comb has, a limiter its lookahead. A removed node runs that long before it goes.

{{ api_signature(value="context:node(kind: string, settings: { [string]: number | string }?) -> AudioNode") }}

A processor as a node of its own, connected to nothing, with `channels` among its settings if fewer than the context's. A voice can play straight into it.

{{ api_signature(value="node:listen(key: AudioNode?, at: number?) -> boolean") }}

`listen` gives a processor with a sidechain (the compressor) a key: it hears `key` as it leaves that node, after its gain, instead of its own input, and still processes its own. A compressor on the music listening to the dialogue bus ducks the music whenever anyone speaks:

```luau
--!strict
local audio = require '@jess/audio'

local context = audio.offline()
local music, dialogue = context:bus(), context:bus()
local ducker = music:insert('compressor', { threshold = -30, ratio = 8, release = 0.25 })
ducker:listen(dialogue)
```

The key runs before the node listening to it, so listening to a node that the listener feeds (its bus's route, say) would be a loop, and is refused. `listen(nil)` goes back to the processor's own input.

A name the processor doesn't have, or a value outside its range, is an error naming what it does take.

## Patches

{{ api_signature(value="context:patch(body: (graph: AudioPatch) -> (), at: number?)") }}
{{ api_signature(value="graph:connect(from: AudioNode | AudioVoice, to: AudioNode, options: { from: string?, to: string?, level: number?, tap: ('post' | 'pre')?, feedback: number? }?)") }}
{{ api_signature(value="graph:disconnect(from: AudioNode | AudioVoice, to: AudioNode, options: { from: string?, to: string? }?)") }}
{{ api_signature(value="graph:route(node: AudioNode, parent: AudioNode)") }}
{{ api_signature(value="graph:insert(host: AudioNode, node: AudioNode, index: number?)") }}
{{ api_signature(value="graph:remove(node: AudioNode)") }}

A patch is any number of changes to the graph made as one. `body` makes them through `graph`; once it returns, they all take effect together on frame `at`, or, if any of them is wrong (a port that isn't there, a loop, no room), none of them does and the error says why. If `body` raises, nothing changes. Everything the bus and node methods do is a patch of one change.

`connect` joins output `from` of one node (its first, `out`, unless named) to input `to` of another (its first audio input unless named), at `level` (1), tapped after the source's processing or, with `pre`, before it. Any output can feed any number of inputs, and an audio input sums everything connected to it. The input can also be a sidechain (`key`) or a parameter, by name: that's modulation, below. A voice connects to a node at a level, as `voice:send` does, but on the patch's frame. Two nodes with different channel counts connect only one way: mono into anything, anything into mono as the average of its channels, or the same count. Any other pair is refused when the patch is made.

```luau
--!strict
local audio = require '@jess/audio'

local context = audio.offline()
local effects = context:bus()
local shot = audio.pcm(buffer.create(4 * 4800), { format = 'f32', channels = 1, rate = 48000 })

-- One sound, three voices, each through processing of its own into one bus: no bus per voice.
for index, cutoff in { 500, 2000, 8000 } do
  local filter = context:node('filter', { frequency = cutoff })
  local drive = context:node('drive', { drive = index * 6 })
  local voice = context:voice(shot)
  context:patch(function(graph)
    graph:connect(voice, filter)
    graph:connect(filter, drive)
    graph:connect(drive, effects)
  end)
  voice:play()
end
```

### Feedback

A loop is allowed only through a connection with `feedback`: a delay in frames, at least 128, on the connection that closes it. Without one, a loop is refused. The delay is counted in output frames, whatever size the blocks are rendered in, so a loop sounds the same through a device and offline. It is part of the sound, so latency compensation doesn't line anything up against it: the time round a loop is its feedback delay plus the latency of the processors on the way.

```luau
--!strict
local audio = require '@jess/audio'

local context = audio.offline()
local echo = context:bus()
local quieter = context:node('gain', { gain = 0.5 })
context:patch(function(graph)
  graph:connect(echo, quieter)
  graph:connect(quieter, echo, { feedback = 12000 })
end)
```

### Modulation

Connecting a node's output to a parameter modulates it: the parameter is its automation plus `level` times the source, mixed to mono, kept inside its range. A value that isn't a number leaves the automation alone. An audio-rate parameter hears its modulation every sample; a control-rate one on every frame that is a multiple of 32. Nothing runs in the script while it happens: an oscillator can sweep a filter, an envelope can open an amplifier, and one oscillator can drive another's frequency.

```luau
--!strict
local audio = require '@jess/audio'

local context = audio.offline()
local voice = context:node('oscillator', { wave = 'saw', frequency = 110, channels = 1 })
local cutoff = context:node('filter', { frequency = 400 })
local amplifier = context:node('gain', { gain = 0 })
local sweep = context:node('envelope', { attack = 0.01, decay = 0.2, sustain = 0.2, channels = 1 })
local swell = context:node('envelope', { attack = 0.005, sustain = 0.8, channels = 1 })
context:patch(function(graph)
  graph:connect(voice, cutoff)
  graph:connect(cutoff, amplifier)
  graph:connect(amplifier, context:master())
  graph:connect(sweep, cutoff, { to = 'frequency', level = 4000 })
  graph:connect(swell, amplifier, { to = 'gain', level = 0.5 })
end)

-- A note: both envelopes open on frame 4800 and close half a second later.
for _, envelope in { sweep, swell } do
  envelope:set('gate', 1, 4800)
  envelope:set('gate', 0, 4800 + 24000)
end
```

## Budgets

A context can hold far more voices than it mixes. With `physical` set below `voices`, on every multiple of 128 frames, and on any frame a voice is started, stopped, or paused, the engine ranks the live voices by `priority`, then by how loud they would be (their gain, times what distance and cones give them in space), and mixes the top `physical`; the rest are culled until the next ranking. A voice that runs out frees its place at the next multiple of 128. A voice that was mixed holds its place against one only a little louder, so voices near the line don't flicker in and out. Ties go to the voice that started in the lower slot, so the same score culls the same voices every time.

What a culled voice does is its own: `advance` (the default) keeps its place in its source moving, silently, so a looping hum out of earshot is where it would have been when it comes back. A stream does the same: it lets go of what it would have played, so its writer keeps going and a stem stays in step with the others; `pause` holds it where it was; `stop` ends it, which is voice stealing. Music and dialogue that must never drop out get a high `priority`. Rules like "no more than four footsteps at once" are the application's: count them in Luau and stop the oldest.

## Space

A voice played with a `place` is in space: where it is relative to the listener sets its level, its pan, and its pitch, worked out on every multiple of 128 frames and glided across the 128 that follow, so a moving voice slides rather than steps, the same however the render is cut. Positions, velocities, and directions are in the world's own units and axes; the only thing the context needs to know about the axes is which way they turn (`handedness`), and the only thing about the units is `speedOfSound` in them. A voice not given a `place` is not in space, and costs nothing for it.

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

## Compressed files

{{ api_signature(value="audio.load(data: buffer | string) -> AudioPcm") }}

A WAV, MP3, Ogg Vorbis, or Ogg Opus file, whole, as PCM at the file's own rate: what a footstep or a door wants. The encoder's delay and padding are trimmed, so the PCM is as long as what was encoded, and a loop cut in an encoder loops without a gap. Opus always decodes at 48000 frames per second, whatever rate it was made from.

{{ api_signature(value="audio.decoder(data: buffer | string, options: { format: ('mp3' | 'vorbis' | 'opus')? }?) -> Decoder") }}

A decoder over a copy of the bytes, the format detected unless named: `decoder:format()`, `rate()`, `channels()`, `frames()` (nil if the file doesn't say), `position()`, `read(target, offset?, frames?)` into a buffer as interleaved 32-bit floats, and `seek(frame)`. It is [`@dream/audio`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-audio)'s, which says how each format's length and seeks are worked out: MP3 and Vorbis seek exactly, Opus to within a millionth of full scale.

{{ api_signature(value="audio.feed(data: buffer | string, options: AudioFeedOptions?) -> AudioFeed") }}

Music, ambience, anything too long to hold decoded: a feed decodes an MP3, Ogg Vorbis, or Ogg Opus file into a [stream](#streams) on a thread of its own. The stream has the file's channels and rate and holds `seconds` (1) of it ahead; the thread waits for room, decodes `block` frames (4096) into it, and goes round, so the stream keeps playing whatever the script is doing: a long frame, a level loading, nothing at all. It starts at frame `start` (0), the `format` detected unless named. At the file's end it finishes the stream, and the voice ends once it has played the rest. With `loop`, it seeks back to `loopStart` (0) instead, and since the seek is exact, the loop has no seam.

{{ api_signature(value="feed:stream() -> AudioStream") }}
{{ api_signature(value="feed:status() -> AudioFeedStatus") }}
{{ api_signature(value="feed:stop()") }}

`stream` is the stream to play. `status` says whether the thread is `done`, how many `loops` it has gone round, and the `error` the decoder failed with if it did, in which case the stream was finished there. `stop` stops decoding, and what the feed already wrote plays out. A feed also ends once nothing but it holds its stream, no voice and no script, and the end of the runtime that started it stops it and waits for its thread.

```luau
--!strict
local audio = require '@jess/audio'
local fs = require '@dream/fs'

local music, problem = fs.readFile 'music.ogg'
assert(music, problem)

local context = audio.output()
local feed = audio.feed(music, { loop = true })
context:play(feed:stream(), { gain = 0.8 })

-- Two busy seconds of something else entirely; the music doesn't notice.
local busy = os.clock() + 2
while os.clock() < busy do
end
print(feed:status().loops, feed:stream():stats().missed)
```

For a file a second long it prints `2 0`: twice round the loop, and not a frame missed, though the script never looked at the music after starting it.

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
