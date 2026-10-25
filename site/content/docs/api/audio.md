---
title: '@jess/audio'
description: 'Turn bytes into PCM, play voices on a sample clock where every command lands on its exact frame, render the mix offline, and read and write WAV.'
weight: 52
extra:
  kind: api
---

{{ api_signature(value="require('@jess/audio'): { pcm, offline, frames, seconds, readWav, loadWav, writeWav }") }}

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

## Contexts

{{ api_signature(value="audio.offline(options: { rate: number?, channels: number?, voices: number?, commands: number? }?) -> AudioContext") }}

A context the script renders. Defaults: 48000 frames per second, 2 channels, 256 voices, and room for 4096 commands waiting for their frame.

`voices` counts voices playing or stamped for later; `commands` bounds the queue to the engine. Both are fixed when the context is made, so the engine never allocates while it mixes. Running out of either is an error at the call that ran out (`all 256 voices are playing or scheduled`, `the audio command queue is full`), never a sound quietly not played.

{{ api_signature(value="context:play(pcm: AudioPcm, options: AudioPlayOptions?) -> AudioVoice") }}

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

A source at a different rate from the context is resampled; at the same rate and `rate = 1` it plays sample for sample. Mono mixes into any output, stereo into mono (averaged), and otherwise the channel counts must match.

{{ api_signature(value="context:render(frames: number) -> buffer") }}

Mixes the next `frames` frames and returns them as interleaved 32-bit floats, the shape `writeWav` takes. Samples aren't clipped; any sample that isn't a finite number is written as silence and counted.

{{ api_signature(value="context:now() -> number") }}

The next frame the engine renders. Anything stamped for it or later lands on its frame; anything stamped earlier runs on the next frame and counts as `late`.

{{ api_signature(value="context:rate() -> number") }}
{{ api_signature(value="context:channels() -> number") }}
{{ api_signature(value="context:stats() -> AudioStats") }}

`stats` is the engine as of its last render: `frames` rendered, voices `active` in the mix and `allocated` (playing or waiting to start), commands carried out `late`, `nonfinite` samples silenced, and commands `pending` for a later frame.

## Voices

{{ api_signature(value="voice:stop(at: number?) -> boolean") }}
{{ api_signature(value="voice:set(param: 'gain' | 'pan' | 'rate', value: number, at: number?) -> boolean") }}
{{ api_signature(value="voice:state() -> 'scheduled' | 'playing' | 'ended'") }}

A stop or change stamped for a frame is carried out on that frame. One sent before the voice has started applies from its first frame: a voice stopped before its start never plays, and one whose gain is set before it starts starts at that gain. Both return whether the voice was still live; on an ended voice they do nothing.

## Time

{{ api_signature(value="audio.frames(seconds: number, rate: number) -> number") }}
{{ api_signature(value="audio.seconds(frames: number, rate: number) -> number") }}

Seconds to the nearest frame (halves round up), and frames back to seconds.

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
