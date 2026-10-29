---
title: 'Mix Audio Offline'
description: 'Create PCM, render a few frames without an audio device, and inspect the result.'
weight: 80
extra:
  kind: guide
---

An offline context mixes on a sample clock without opening a sound device. This makes a small audio operation reproducible in a headless test. Save this as `mix.jsl`:

```luau
--!strict
local audio = require '@jess/audio'
local samples = buffer.create(4 * 4)

for frame = 0, 3 do
  buffer.writef32(samples, frame * 4, 0.25)
end

local sound = audio.pcm(samples, { format = 'f32', channels = 1, rate = 48000 })
local context = audio.offline { channels = 1, rate = 48000 }
context:play(sound)
local mixed = context:render(4)
print(buffer.len(mixed), context:now())
```

`jess check mix.jsl` checks it; `jess run mix.jsl` prints `16` and `4`, separated by a tab. Four mono frames of 32-bit floats occupy 16 bytes, and the next frame to render is frame 4. This script needs no filesystem, network, or audio-output capability and produces no audible playback.

The returned buffer is interleaved floating-point PCM, not a WAV file. Use `audio.writeWav` to encode it and a separately granted filesystem write to save it. For audible playback, an output context and its device capability are a different operation; the machine's sound system and device availability matter. The [audio reference](@/docs/api/audio.md) covers formats, contexts, scheduling, errors, and device behavior. The checked fixture is `tests/fixtures/docs/offline-audio.jsl`.
