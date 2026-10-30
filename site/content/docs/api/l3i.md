---
title: 'The @dream modules'
description: 'The native primitives under jess: files, bytes, processes, the network, and text, from L3i, each behind a named capability.'
weight: 210
extra:
  kind: api
---

{{ api_signature(value="require('@dream/audio' | '@dream/bytes' | '@dream/data' | '@dream/dns' | '@dream/fs' | '@dream/intern' | '@dream/intl' | '@dream/luau' | '@dream/process' | '@dream/tcp' | '@dream/tls' | '@dream/udp')") }}

Jess runs Luau on [L3i](https://DreamWeave-MP.github.io/l3i/), and L3i brings the modules that touch the operating system: the `@dream/*` names. They are the whole native floor under a script. Everything jess adds on top is Luau, and everything below them is L3i's.

L3i documents them, on its [built-in extensions](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/) page, and this page won't repeat it. What jess adds is the capability gate: a module's functions that reach outside the VM check a named capability at the native seam, and a runtime that wasn't granted it refuses, naming the function and the capability. Nothing is denied quietly, and nothing is granted by default except your terminal, and the input devices and the GPU in front of you.

```text
$ jess run read.jsl
error: dream.fs.readFileString: needs the 'filesystem.read' capability, which this runtime does not grant

$ jess run env.jsl
error: dream.process.env: needs the 'process.environment' capability, which this runtime does not grant
```

[`jess.granted`](@/docs/api/jess.md#jess-granted) asks before reaching; [Capabilities](@/docs/reference/capabilities.md) lists every name, what grants it, and what asks for it.

| Module | What it is | Capability |
| --- | --- | --- |
| [`@dream/audio`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-audio) | Decoders for MP3, Ogg Vorbis and Ogg Opus into float frames; what [`@jess/audio`](@/docs/api/audio.md) loads and streams compressed files with | none |
| [`@dream/bytes`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-bytes) | Buffer search and slicing, digests, secure random bytes, HMAC, RSA and ECDSA signatures (RS256, ES256) for signed tokens, Deflate, Zstandard and LZMA, text encodings, varints, and [regular expressions](@/docs/api/regex.md) as `bytes.regex` | none |
| [`@dream/data`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-data) | Whole-buffer arithmetic: sums, extrema, sorting, gathering, scattering, and partitioning over typed buffers | none |
| [`@dream/dns`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-dns) | Name resolution | `network.dns.resolve` |
| [`@dream/fs`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-fs) | [Files and folders](@/docs/api/fs.md): read, write, list, paths | `filesystem.read` for every read, `filesystem.write` for every change to the disk |
| [`@dream/intern`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-intern) | String interning, one table of unique strings | none |
| [`@dream/intl`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-intl) | Locales, plural rules, number and date formatting; what [`@jess/l10n`](@/docs/api/l10n.md) is built on | none |
| [`@dream/luau`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-luau) | Luau's own parser, as data; what [`@jess/analysis`](@/docs/api/analysis.md) and [`@jess/style`](@/docs/api/style.md) are built on | none |
| [`@dream/process`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-process) | The environment, the standard streams, and child processes | `process.environment` for `env`, `process.stdio` for `write`, `process.spawn` for `run` |
| [`@dream/tcp`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-tcp) | Streams, listeners, and a poller | `network.tcp.connect` to connect out; `network.tcp.listen` to listen on loopback, plus `network.tcp.public` to bind a wildcard or non-loopback address |
| [`@dream/tls`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-tls) | TLS sessions over a connected TCP stream | none of its own: making the stream took `network.tcp.connect` or `network.tcp.listen` |
| [`@dream/udp`](https://DreamWeave-MP.github.io/l3i/docs/builtin-extensions/#dream-udp) | An authenticated datagram transport, client and server | `network.transport` |

`jess luau-types` writes a stub for each under `types/dream/`, and the `.jessrc` that `jess init` writes aliases `@dream/` to them, so `local fs = require '@dream/fs'` is fully typed under `--!strict` without an annotation. The stubs describe the L3i the jess you have was built against; regenerate them when you upgrade.

Two things are jess's, not L3i's, and have pages here: [`@dream/fs`](@/docs/api/fs.md) is where [`@jess/backup`](@/docs/api/backup.md) comes in, and the [`require`](@/docs/api/jess.md#require) that resolves these names is jess's.

Related: [Capabilities](@/docs/reference/capabilities.md), [`jess run --capability`](@/docs/cli/run.md#capabilities).
