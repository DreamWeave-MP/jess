---
title: 'Embedding jess-lua'
description: 'Run jess scripts from your own program.'
weight: 50
extra:
  kind: guide
---

`jess-lua` isn't a Lua module you `require` from a stock interpreter. It's a Rust crate that builds a Luau VM with jess installed. `jess run` is about forty lines around it; your program can be too.

`jess-lua` is MIT OR Apache-2.0 like every other jess crate, and the `jess` facade re-exports it behind the `lua` feature, off by default because it builds Luau under l3i's toolchain policy. Depending on `jess-lua` directly is the same code.

```toml
[dependencies]
jess-lua = "0.1"
```

The VM is [l3i](https://DreamWeave-MP.github.io/l3i/), which builds only with clang, lld, and cross-language thin LTO, and Cargo doesn't inherit a dependency's build config. Copy the `[env]` and `rustflags` lines from jess's `.cargo/config.toml` into your own; l3i's `build.rs` names what's missing otherwise. See [Building and Testing](@/docs/contributing/building-and-testing.md).

## The smallest host

```rust
use jess_lua::l3i::value::Table;
use jess_lua::{ConfigSource, Options, runtime};

fn main() -> jess_lua::l3i::Result<()> {
    let runtime = runtime(Options { config: ConfigSource::Disabled })?;

    let summary: Table = runtime.eval(
        r#"
        local plugin = jess.plugin.open("MyMod.esp")
        local lights = 0
        for _ in plugin:select { type = "LIGH" } do
            lights += 1
        end
        return { version = jess.version, lights = lights }
        "#,
    )?;
    let stack = runtime.stack();
    let version: String = summary.get(&stack, "version")?;
    let lights: u32 = summary.get(&stack, "lights")?;
    println!("jess {version} saw {lights} lights");
    Ok(())
}
```

Values come back the l3i way: `eval::<T>()` for a scalar, a tuple, or a pinned `Table`, then `Table::get` on the runtime's stack. l3i's [stack and values](https://DreamWeave-MP.github.io/l3i/docs/stack-and-values/) page has the rest. To run a chunk under its own name, so errors and relative `require`s point somewhere useful, load it through `jess_lua::scripts::load(&runtime, "path/to/chunk.jsl", source)` and call the function it returns.

## What `runtime` installs

`runtime(options)` is what `jess run` does. It makes a VM from jess's runtime plan (`jess_lua::plan()`), with:

- `jess`, the whole API, also `require("@jess")`.
- The libraries jess is built on, as globals: `vfstool`, `openmwConfig`, and `dreamPath`, each also reachable as `require("@dream/vfs")` and so on. See [Downstream Globals](@/docs/api/downstream-globals.md).
- An OpenMW-style `require`: those module names (with or without a leading `@`) return the modules, `@dream/...` paths go to the libraries' own modules, and anything else loads from the OpenMW VFS the way the engine does, so `require("scripts.myMod.util")` finds `scripts/mymod/util.lua` or `.../util/init.lua`.
- A `print` that writes through Rust's stdout, so it doesn't interleave with your `println!`s or turn `\n` into `\r\n` on Windows.

Then it sandboxes the globals (`luaL_sandbox`): the globals table is read-only and marked safe, which is what lets Luau call `math.floor` and `buffer.readi32` on its fast path. A script that wants a global writes to `_G` and fails; a host that needs one sets it before any script runs, the way `scripts::run_file` sets `arg`, through `l3i::readonly::set_read_only_field`.

One plan serves every runtime on a thread; `Runtime::from_plan(&jess_lua::plan()?)` gives you a VM with jess's types and modules registered but none of the globals above. That's for hosts composing their own environment; `runtime` is the one to start from.

## `Options`: which OpenMW config

`Options { config }` takes a `ConfigSource`:

- `ConfigSource::Default` (what `Options::default()` gives you): OpenMW's own lookup, honouring `OPENMW_CONFIG` and `OPENMW_CONFIG_DIR`, else the user's `openmw.cfg`. Loaded lazily, the first time a script needs it.
- `ConfigSource::Path(path)`: a specific `openmw.cfg` or the folder holding it.
- `ConfigSource::Disabled`: no config. Only literal file paths resolve, and `require` has no VFS to look in.

That's the config bare names like `jess.plugin.open("Morrowind.esm")` resolve through. Scripts can switch it later with `jess.openmw.configPath`. See [OpenMW and the VFS](@/docs/scripting/openmw-and-vfs.md).

## One VM, several libraries

jess-lua is an l3i extension (`jess_lua::plan::JessExtension`), composed into one `RuntimePlan` with `vfstool_lib`'s, `openmw-config`'s, and `dream-path`'s. The plan assigns every userdata tag, atom, and direct-access slot, so the libraries can all live in one VM and pass handles to each other: a VFS from `context:vfs()` is a real `vfstool` object, and `require("@dream/vfs").VFS.new()` is the same type jess reads the load order through.

Your host has to be on the same l3i. Use the re-export, `jess_lua::l3i`, and never name l3i yourself, or depend on `l3i = "1"` so Cargo unifies them. `jess_lua::runtime` returns l3i's `Runtime`, so nothing from another binder type-checks against it.

jess-lua turns on l3i's `jit` feature, so `--!native` scripts compile to machine code, and the vfstool `lua-write` and `lua-host` features with their capabilities granted, so scripts reach the host filesystem through `vfstool.host` as well as `@dream/fs`.

## Load-order scripts and modify scripts

`jess_lua::scripts` has what the CLI builds its script commands from, and the load-order runner the CLI no longer uses but a host can:

```rust
use jess_lua::scripts;

// Every jess-script= entry in the runtime's openmw.cfg, in order.
let list = scripts::load_order_scripts(&runtime)?;   // needs a config
let state = scripts::run_scripts(&runtime, &list)?;  // the shared state table, after the last script

// `jess run script.jsl a b`: `arg` and `...` set up the same way.
let returned = scripts::run_file(&runtime, std::path::Path::new("script.jsl"), &["a".into(), "b".into()])?;
```

Each load-order script is called with `(state, openmw)`: a table every script in the run shares, and the loaded OpenMW context. `scripts::BUNDLED` holds the bundled scripts (`@jess/tes3merge` and the rest) by name, `run_file` runs one by name when no file has it, and `script_name` names an entry. `load_order_scripts` fails on an entry nothing has; `list_load_order_scripts` reads nothing and returns every entry with its `ScriptStatus` (`Bundled`, `Found`, `Missing`).

`run_file` sets the global `arg` table (the path at `0`, the arguments from `1`) and passes the arguments as the script's `...`, then returns whatever the script returns, as pinned values. Use it instead of `runtime.exec(...)` when your users expect a script to behave the way it does under `jess run`; they will, and they'll file bugs when it doesn't.

`scripts::modify_plugin(&runtime, name, source, raw_plugin, path)` and `scripts::modify_nif(&runtime, name, source, stream, path)` run a script on one file, called with `(handle, path)`, and return the new bytes only if the script changed something. That's `plugin modify --script` and `nif modify --script`. See [Modify Scripts](@/docs/scripting/modify-scripts.md).

Every chunk these load, and every module a script `require`s, is compiled once per runtime and kept by its name, then loaded on the running thread and compiled to native code under the plan's policy (l3i's `LoadScope`), OpenMW's script template cache without the template: a load-order run of the same script twice compiles it once.

## Types for your users

`jess_lua::types::types()` is the folder `jess luau-types` writes: `jess.d.jsl` (what `jess_lua::definitions()` returns: the libraries' types as their extensions declare them, jess's own, and the record and NIF types) and a stub module for every name `runtime`'s `require` resolves. Write it out, and a `.jessrc` with its aliases beside your users' scripts, and luau-lsp and `--!strict` scripts know every field and every module:

```rust
let types = jess_lua::types::types()?;
jess_lua::types::write(std::path::Path::new("scripts/types"), &types)?;
std::fs::write("scripts/.jessrc", types.jessrc("types"))?;
```

The types describe the jess that generated them. A folder from an older build will cheerfully type-check a script against fields that no longer exist, so regenerate it when you upgrade.

If you'd rather hand your users a whole script folder, `jess_lua::init` is `jess init`:

```rust
use jess_lua::init::{Editor, InitOptions, init};

let report = init(std::path::Path::new("scripts"), &InitOptions { editor: Editor::Neovim, force: false })?;
for written in &report.written {
    println!("wrote {}", written.path.display());
}
```

It writes `types/`, a strict `.jessrc` with the aliases, a `main.jsl` that runs, and the editor's luau-lsp settings (`Editor::VsCode`, `Neovim`, `Zed`, `Helix`, or `None`). `types/` is always rewritten. Anything else already there lands in `report.skipped` unless `force`, which backs it up and replaces it. `init::files(editor, &types)` returns the files besides `types/` as `(path, contents)` pairs, if you want to write them yourself.

## Sandboxing

The globals are read-only and that's all. Scripts read and write files like any program, through `@dream/fs`, `vfstool.host`, and every `save` method. If you run scripts you didn't write, that's on you, same as running a binary you didn't write.
