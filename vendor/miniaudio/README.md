# miniaudio

[miniaudio](https://github.com/mackron/miniaudio) 0.11.25, by David Reid: `miniaudio.h` and its
`LICENSE` as tagged upstream, unmodified. It is public domain, or MIT No Attribution, at the
user's choice.

jess uses its device layer only, behind the `audio-device` feature: `src/audio/backend/device.c`
compiles it with its decoding, encoding, node graph, engine, resource manager, and generators
turned off, and the engine, resampling, and mixing are jess's own. To move to a new release,
replace both files with the tagged ones and change the version here.
