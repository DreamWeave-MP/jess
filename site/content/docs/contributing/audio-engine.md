---
title: 'Audio Engine'
description: 'How jess::audio mixes on a sample clock without ever waiting on a script: the control side and the render side, frame-stamped commands, the graph, voices, streams, devices, and what is left for later.'
weight: 78
extra:
  kind: guide
---

A footstep, a score that follows a fight, a room full of machines, a synthesizer, a multitrack bounce: they all need the same thing underneath, a mixer that does exactly what it was told on exactly the frame it was told, and never stops to ask. `jess::audio` (`src/audio`) is that mixer. It knows frames, channels, voices, buses, processors, and when. It doesn't know what music or a footstep is: that's Luau, in [`@jess/audio`](@/docs/api/audio.md) and the packages above it.

## Two sides

A context is two halves joined by two bounded queues:

```text
application side    Control          play, stop, set, ramp, route, mount...
                                       each a command stamped with an output frame
     v  commands                       (bounded; a full queue is an error, never a drop)
render side         Engine::render   carries each command out on its frame, mixes
     ^  feedback                       (no allocation, no lock, no Luau)
application side    Control::collect ended voices and buses, memory to free
```

`Engine::render(out)` is the whole render side. A device's thread calls it when the hardware wants frames; an offline context calls it when a script asks. It's the same function either way, which is why an offline render is the test of a live one.

The control side owns every decision that can fail: whether a voice slot is free, whether a route loops, whether a parameter is in range. The engine only carries out commands that were already checked, so it has nothing to report but counters.

## The render path's rules

Rendering never allocates, frees, locks, blocks, logs, touches a file, or calls into Luau. Everything the engine needs is allocated when it's made: the voice table, the bus table and its mix buffers, the processor table, the list of commands waiting for their frame, scratch buffers. Anything the engine is done with that holds memory (a voice's PCM, a removed processor, a routing order it replaced, a spent group) goes back down the feedback queue to be dropped on the control side. The feedback queue is sized so it can't fill: each voice, bus, and processor ends once and its slot is reused only after the control side has heard, and each command produces at most a fixed amount of feedback.

A test installs a counting allocator around `Engine::render` and asserts zero allocations and frees across renders that start, stop, ramp, loop, cull, spatialize, and stream voices.

Samples that aren't finite are written as silence and counted. Filters and reverbs flush denormals in their feedback state. A panic on a device's thread is caught at the boundary: the device plays silence from then on and reports `failed`.

## Time

The clock is output frames rendered, a `u64` only the engine moves. A command carries `at`, an output frame, or none for "the next frame rendered". The engine keeps commands for later in a list sorted by frame and then by the order they were sent, and splits whatever block it is mixing at each command's frame, so a start, stop, or change lands on its exact sample, not at the next block boundary.

Musical time, application time, and a device's presentation latency are not the engine's. `@jess/audio`'s tempo map turns beats into frames from a formula (never by adding up beats); a device reports its latency for whoever needs to line pictures up with sound.

A group sends several commands as one queue entry, so the ones that say "now" are carried out on the same frame. It can't half-arrive: there is room in the queue when it opens, and nothing else is sent until it closes.

## The graph

Buses form a tree under the master bus, with sends adding more paths and sidechain keys adding dependencies. The control side keeps the graph and works out, on every change, an order in which each bus comes after everything that feeds it (Kahn's algorithm, lowest slot first, so the same graph always mixes the same way). A change and its new order reach the engine in one command. A change that would make a loop is refused on the control side; the engine never sees a cycle.

Each block, voices add into their buses, then the buses are walked in order: processors, pre-fader sends, gain and mute, meter, post-fader sends, parent. A removed bus keeps running its processors for as long as their tails say (a reverb's decay, a delay's echoes), then goes with them.

Each bus reports latency: the processor latency along the longest path into it. Where paths of different latency meet, they are lined up. The same longest-path walk gives each bus the time its latest path arrives, and every other way in waits for that: each edge from a bus to its parent or a send by the difference, and what voices add into the bus directly (which arrives with no latency at all) by the whole of it. The control side works the delays out whenever the routing or a processor's latency changes, allocates their lines, and sends them stamped with the change's own frame. The engine moves each line's history into its replacement, so a line that keeps its length plays on without a seam, and sends the old set back to be dropped. A delay that changes length can't be seamless: one that grows plays silence where it has no history, and one that shrinks skips. A processor hearing a sidechain key hears it as it left its bus, uncompensated.

## Voices

Voices are not graph nodes. A voice is an entry in a fixed table that renders into a bus. Starting one is a command, not a graph change, which is what lets a context hold thousands of them.

A voice reads its source at a 32.32 fixed-point position, so a render is the same on every machine however long it runs. At its source's own rate it copies samples exactly; otherwise it interpolates (Catmull-Rom). Loops wrap without a gap, and can be released on a frame for an outro.

Gain and pan ramp per sample. Processor parameters ramp at a control rate of 32 frames: the engine caps its blocks at 32 while one moves and tells the processor where the ramp has got to at each block's start.

When a context mixes fewer voices than it holds, it ranks them each block by priority and then by loudness, and mixes the top ones. The rest keep time silently, hold their place, or are stolen, as each voice asked. A small bonus for voices already mixed keeps the line from flickering.

Space is a stage of the voice, not a separate system: the listener and each voice's place give a gain (distance model, cone, and the application's own occlusion gain), a pan, a Doppler factor on the playback step, and a one-pole low-pass. These are worked out once a block and glided across it.

## Streams

A stream is a ring of interleaved frames in atomics, one writer and one reader, with no lock and no unsafe code. A voice reads it through a small window, so a stream resamples and interpolates like any source. A dry ring plays silence and counts it. A stream handle crosses to a worker as a `JessHandle`, found again through a registry of weak references, so a worker can decode or generate into a stream the main script plays.

## Devices

The `audio-device` feature builds miniaudio's device layer (`vendor/miniaudio`, 0.11.25, unmodified) behind a C shim of a dozen functions (`src/audio/backend/device.c`). Rust never sees a miniaudio structure. The device runs at its own rate unless asked otherwise, so jess resamples rather than miniaudio. Stops, reroutes to a new default device, and interruptions are counted.

Why miniaudio, and only its device layer:

- **Deployment.** miniaudio loads ALSA, PulseAudio, PipeWire, JACK, CoreAudio, WASAPI, AAudio, and OpenSL at run time, so a jess binary links no audio library and runs on a machine with none (it falls back to nothing, and the null device is always there). cpal links libasound on Linux at build time, which a cross-built handheld image or a minimal container may not have. SDL3 would bring a window system jess has no use for here. cubeb is C++ with its own build.
- **Behaviour.** miniaudio follows the default device when it changes and reports it, and its null device plays to nowhere in real time, so every platform's tests can run a real device thread.
- **Licence.** Public domain or MIT No Attribution, either compatible with jess's own.
- **Scope.** Only the device. miniaudio's engine, node graph, resampler, and decoders stay compiled out. The mixer, the clock, and the DSP are jess's, so offline and live renders are one engine.

The feature is on by default, because it costs nothing at run time: miniaudio links no audio library when jess is built, so a jess on a machine with no sound system still starts and finds the null device. What it costs is jess's first build script and a C compile, which `--no-default-features` leaves out.

A live context belongs to the Luau runtime that opened it, not to the script's handles: the runtime's jess state holds every output it opened until the script closes it, so a garbage collection never stops a sound. A domain restart drops its runtime and so its outputs, stopping their devices; closing one drops its engine on the spot, and the commands sent after find the engine gone.

## What is deliberately left

These are designed for, but not built:

- **Capture.** The `audio.capture` capability is reserved, and miniaudio's duplex devices are the path.
- **Pitch-preserving time stretch.** It would be a processor, never a playback rate.
- **HRTF and binaural rendering.** It would be another voice stage after pan, chosen per voice.
- **Geometric acoustics.** It is the application's to compute, and hands the engine the gains, low-passes, and send levels it already takes.
- **MIDI, instruments, multitrack arrangement, automation lanes, and freezing tracks.** These are Luau over the same commands: a transport is a tempo map and a queue of stamped commands, a freeze is an offline render into a `Pcm`.
- **A public processor interface.** The built-in processors implement a Rust trait, and there is no plugin ABI.
