---
title: 'Audio Engine'
description: 'How jess::audio mixes on a sample clock without ever waiting on a script: the control side and the render side, frame-stamped commands, the graph, voices, streams, devices, and what is left for later.'
weight: 78
extra:
  kind: guide
---

A footstep, a score that follows a fight, a room full of machines, a synthesizer, a multitrack bounce: they all need the same thing underneath, a mixer that does exactly what it was told on exactly the frame it was told, and never stops to ask. `jess::audio` (`src/audio`) is that mixer. It knows frames, channels, voices, nodes, connections, and when. It doesn't know what music or a footstep is: that's Luau, in [`@jess/audio`](@/docs/api/audio.md) and the packages above it.

## Two sides

A context is two halves joined by two bounded queues:

```text
application side    Control          play, stop, set, ramp, revise the graph...
                                       each a command stamped with an output frame
     v  commands                       (bounded; a full queue is an error, never a drop)
render side         Engine::render   carries each command out on its frame, mixes
     ^  feedback                       (no allocation, no lock, no Luau)
application side    Control::collect ended voices and buses, memory to free
```

`Engine::render(out)` is the whole render side. A device's thread calls it when the hardware wants frames; an offline context calls it when a script asks. It's the same function either way, which is why an offline render is the test of a live one.

The control side owns every decision that can fail: whether a voice slot is free, whether a route loops, whether a parameter is in range. It compiles the graph, too: the engine runs plans it is handed and never works out an order. The engine only carries out commands that were already checked, so it has nothing to report but counters.

## The render path's rules

Rendering never allocates, frees, locks, blocks, logs, touches a file, or calls into Luau. Everything the engine needs is allocated when it's made or arrives allocated: the voice table, the node table and the nodes' input buffers, the list of commands waiting for their frame, scratch buffers, and each plan with its delay lines and scratch. Anything the engine is done with that holds memory (a voice's PCM, a node that has rung out, a plan it replaced, a spent group) goes back down the feedback queue to be dropped on the control side. The feedback queue is sized so it can't fill: each voice and node ends once and its slot is reused only after the control side has heard, and each command produces at most a fixed amount of feedback.

A test installs a counting allocator around `Engine::render` and asserts zero allocations and frees across renders that start, stop, ramp, loop, cull, spatialize, and stream voices.

Samples that aren't finite are written as silence and counted. Filters and reverbs flush denormals in their feedback state. A panic on a device's thread is caught at the boundary: the device plays silence from then on and reports `failed`.

## Time

The clock is output frames rendered, a `u64` only the engine moves. A command carries `at`, an output frame, or none for "the next frame rendered". The engine keeps commands for later in a list sorted by frame and then by the order they were sent, and splits whatever block it is mixing at each command's frame, so a start, stop, or change lands on its exact sample, not at the next block boundary.

What the engine renders doesn't depend on how the render is cut into blocks: by a device's buffer size, by the lengths an offline render asks for, or by where commands split a block. Whatever happens less often than every sample happens on a grid fixed to the clock, never at the start of whatever block is running. Control-rate parameters hear where they have got to on every multiple of 32 frames; space is heard, and the voices to mix are chosen, on every multiple of 128. Everything else is sample by sample. A test renders a scene of every built-in processor, ramps of both rates, modulation, feedback, voices gliding in pitch and moving in space, more voices than the budget mixes, and a revision, all stamped between grid frames. It renders it whole, then in blocks of random lengths, and requires the same samples.

That list is allocated once, so it has a size, and a command for later can't go in it when it's full. The control side keeps the frames of the commands for later it has sent and refuses one more than the list holds (`ScheduleFull`). The engine can then take every queued command whenever it renders, and a command for now never waits behind ones that are minutes away.

Musical time, application time, and a device's presentation latency are not the engine's. `@jess/audio`'s tempo map turns beats into frames from a formula (never by adding up beats); a device reports its latency for whoever needs to line pictures up with sound.

A group sends several commands as one queue entry, so the ones that say "now" are carried out on the same frame. It can't half-arrive: there is room in the queue when it opens, every member is checked against that room, and nothing else is sent until it closes. A member refused (no room left in the group, or none in the list of commands for later) isn't added, and the rest still go. The members stamped for later take their places in that list only when the group is sent, so a group dropped before it is sent holds none. The queue can't refuse a closing group; only an engine that is gone can, and then it refuses every command after it the same way, so what the control side recorded for the group has no render left to disagree with.

## The graph

[Audio Graph](@/docs/contributing/audio-graph.md) is the graph's own page. In short: buses and processors are nodes in one table, which holds each node's processor and automation for as long as the node lives. The control side keeps the graph's definition, and makes every change on a clone, checks it, and compiles it into a plan before anything is sent. The plan is an order (Kahn's algorithm, lowest slot first, so the same graph always runs the same way), the connections each step pushes and pulls, the compensation and feedback delay lines, and scratch for outputs that aren't a node's own buffer. The engine swaps a plan in whole on its revision's frame and sends the old one back, so no processor's state, and no ramp, notices a change elsewhere in the graph. A change that would make a loop without a feedback delay on it is refused on the control side; the engine never sees a cycle.

Each block, the engine clears the input buffers of the nodes that have inputs, adds each voice into its outputs, delays what voices added into a node by as long as the latest path into that node takes, and runs the plan's steps in order. A step runs a node's insert chain in its buffer, pushes the connections tapped before its processing, processes, meters a bus, and pushes the rest. A removed node with a tail (a reverb's decay, a delay's echoes) keeps running, unfed, for as long as its tail, then goes.

Each node reports latency: the processor latency along the longest path into it. Where paths of different latency meet, they are lined up. Every connection into a node waits for the latest path into it, and what voices add into the node directly (which arrives with no latency at all) waits for the whole of it. The lines are part of the plan, and change with the revision that needs them, on its frame. The engine moves each line's history into its replacement, so a line that keeps its length plays on without a seam. A delay that changes length can't be seamless: one that grows plays silence where it has no history, and one that shrinks skips.

## Voices

Voices are not graph nodes. A voice is an entry in a fixed table that renders into one or more node inputs: a bus, usually, or a processor only it goes through. Starting one is a command, not a graph change, and compiles nothing, which is what lets a context hold thousands of them and start hundreds a frame.

A voice reads its source at a 32.32 fixed-point position, so a render is the same on every machine however long it runs. At its source's own rate it copies samples exactly. Otherwise each output sample is a windowed sinc over the source frames around it (`src/audio/resample.rs`): eight zero crossings a side under a Kaiser window, read from a table and interpolated, with its zero crossings on source frames so a slower source still passes through its own samples. Read faster than the output plays it (a higher source rate, a rate above one, Doppler towards the listener), the kernel widens by the step, which moves its cutoff down to the output's Nyquist: what the output can't hold is filtered out rather than folded back as an alias. It is flat to three quarters of the output's Nyquist, 6 dB down at it, and about 80 dB down by one and a half times it. The widening stops at four times, past which a faster voice aliases again rather than reading ever more frames. Loops wrap without a gap, and can be released on a frame for an outro.

A voice pauses and plays on, and seeks, on a frame. A paused voice holds its place, unmixed and uncounted against the budget. A seek on a source of PCM moves the position to a whole frame, so the next sample is the source's own: the resampler reads the source around the position and keeps no history, so there is nothing to reset. A seek past a loop's end while looping lands where playing through the loop would have. Each render publishes every voice's source frame, so a script asks where a voice is without a round trip. Where a voice plays changes on a frame too, by a command that swaps its outputs and sends the old ones back to be dropped.

A voice's gain and pan ramp per sample. A node parameter ramps at its own rate. An audio-rate one (a bus's gain) moves every sample, and the processor reads its values for the block. A control-rate one moves on the clock's 32-frame grid: while one moves, the engine breaks its blocks on multiples of 32 and tells the processor where the ramp has got to on each. A step lands on its own frame.

When a context mixes fewer voices than it holds, it ranks them by priority and then by loudness on every multiple of 128 frames, and on any frame a command starts, stops, or pauses a voice, and mixes the top ones. A voice that runs out frees its place at the next multiple of 128. The rest keep time silently, hold their place, or are stolen, as each voice asked. A small bonus for voices already mixed keeps the line from flickering.

Space is a stage of the voice, not a separate system: the listener and each voice's place give a gain (distance model, cone, and the application's own occlusion gain), a pan, a Doppler factor on the playback step, and a one-pole low-pass. These are worked out on every multiple of 128 frames and glided across the 128 frames that follow.

## Streams

A stream is a ring of interleaved frames in atomics, one writer and one reader, with no lock and no unsafe code. A voice reads it through a small window, so a stream resamples and interpolates like any source. A dry ring plays silence and counts it. A stream handle crosses to a worker as a `JessHandle`, found again through a registry of weak references, so a worker can decode or generate into a stream the main script plays.

A feed (`src/audio/feed.rs`) keeps a stream full from a decoder on a thread of its own, so music plays on through anything the script does: a long frame, a level loading, a breakpoint. The thread waits for room, decodes a block into the ring, and goes round, back to a loop's start when the decoder runs out if it loops. It knows no formats, only a `Decode` trait the binding implements over l3i's decoders. It ends when the decoder does, when it fails (finishing the stream and saying why), when it is stopped, or when nothing but the feed holds the stream any more, and it checks for the last two every 50 ms, so stopping one never waits longer than that and a block's decoding.

A feed's stream can seek, and a seek is either prepared or not. The control side sends two commands as one group: one stamped for the seek's frame, and one that prepares it, stamped a lead earlier (half the stream's length), or at once if that has passed. If the list of commands for later has room for only one of them, neither is sent. On the preparation the engine asks the stream for the seek (an epoch, counted by the engine, and a source frame), and until the seek's frame the voice plays on from what the feed has already decoded, which a full stream has more than the lead of. The feed sees the request within a couple of milliseconds, moves its decoder, and marks where in the ring what it writes from then on starts: the same epoch and a ring position, written so a reader never pairs one seek's epoch with another's position. Until the seek's frame the voice reads only up to that mark. On it, the voice lets go of everything before the mark and plays on from there: a prepared seek is gapless and lands on its frame. If the mark isn't there yet, the voice keeps time. It plays silence (counted as `missed` and as `late`), lets go of what the feed wrote before the seek so the feed has the whole stream to write into, and counts the frames as it would have played them; when the mark comes it joins that far past it. A feed's stream keeps time on any underrun the same way: what the feed was late with is let go once it comes, not played late. A script's stream plays every frame it is given, late ones after the gap. The feed carries out one seek at a time. A seek asked for while another is still waiting replaces it, and the one replaced plays silence until the next lands: never another seek's audio. Nothing is rebuilt: the ring, the voice, and the feed's thread carry on. At the decoder's end a feed finishes the stream but keeps its thread until its voice has played everything, so a seek back is still served. A stream a script writes has no decoder to move, and refuses to seek.

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
- **MIDI, multitrack arrangement, automation lanes, and freezing tracks.** These are Luau over the same commands and graph: a transport is a tempo map and a queue of stamped commands, a note is a parameter set and a gate opened on a frame, and a freeze is an offline render into a `Pcm`. [Audio Graph](@/docs/contributing/audio-graph.md) lists what else comes next, and the graph's limits.
