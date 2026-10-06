---
title: 'Rust Crates'
description: 'Use jess''s libraries from your own Rust code.'
template: docs/section.html
page_template: docs/page.html
sort_by: weight
weight: 80

extra:
  kind: guide
---

Every part of jess is a crate on crates.io, all sharing one version. The CLI is a thin frontend over them, so anything `jess` does, your program can do too. APIs are unstable until 1.0: pin a minor version.

- [Crate Overview](@/docs/rust/crates.md): which crate to depend on, and the `jess` facade's features.
- [Plugins in Rust](@/docs/rust/plugins.md): the byte-preserving `RawPlugin`, typed records, and minimal write-back.
- [NIFs in Rust](@/docs/rust/nifs.md): editing Morrowind NIFs, reading everything else, and checking them in bulk.
- [Saves in Rust](@/docs/rust/saves.md): reading, checking, and stripping saves.
- [Embedding jess-lua](@/docs/rust/embedding-lua.md): the whole Luau API in your own program.

The examples on these pages compile against the workspace. When they don't, that's a bug: [file it](https://github.com/DreamWeave-MP/jess/issues).
