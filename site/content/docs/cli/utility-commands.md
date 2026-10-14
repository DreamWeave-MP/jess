---
title: 'init, luau-types, completions, man'
description: 'The commands that set up your editor and shell.'
weight: 50
extra:
  kind: reference
---

Commands that don't touch game data. They take the [global options](@/docs/cli/global-options.md), but only `init`, `luau-types`, and `man --output` use `--format`.

## init

{{ api_signature(value="jess init [--editor <EDITOR>] [--force] [DIR]") }}

| Argument / option | Default | Does |
| --- | --- | --- |
| `[DIR]` | `.` | The folder to set up, created if missing. |
| `--editor <EDITOR>` | `vscode` | Which editor's luau-lsp settings to write: `vscode`, `neovim`, `zed`, `helix`, or `none`. |
| `--force` | off | Replace files already there, after backing them up. |

Sets up a folder for Luau scripts:

| File | What |
| --- | --- |
| `types/` | The types of the whole jess API, from the jess you have installed: the folder [`luau-types`](#luau-types) writes. Rewritten on every run. |
| `.jessrc` | `"languageMode": "strict"`, so every script is type-checked, and the aliases that point `@jess/...` and `@dream/...` at `types/`. |
| `main.jsl` | A working example: lists each plugin in a load order with its light count (`jess run main.jsl path/to/openmw.cfg`, the file or the folder holding it), or one plugin's (`jess run main.jsl MyMod.esp`). |
| `.vscode/settings.json` | VS Code with the Luau Language Server extension: `luau-lsp.types.definitionFiles` set to `{ "@jess": "types/jess.d.jsl" }`, the `standard` platform, no Rojo sourcemap, and `luau-lsp.fflags.enableNewSolver`. |
| `.nvim.lua` | Neovim 0.11+: `vim.lsp.config("luau_lsp", ...)` running `luau-lsp lsp --definitions:@jess=<folder>/types/jess.d.jsl --flag:LuauSolverV2=true`. Neovim reads it only with `vim.o.exrc = true`, and asks once whether to trust it. |
| `.zed/settings.json` | Zed with the Luau extension: `definitions`, the standard platform, and `fflags.enable_new_solver` under `lsp.luau-lsp.settings`. |
| `.helix/languages.toml` | Helix: a `luau-lsp` language server with `--definitions:@jess=types/jess.d.jsl --flag:LuauSolverV2=true` for the `luau` language. |

Every settings file turns on Luau's new type solver, which some of jess's types need ([Editor Setup](@/docs/getting-started/editor-setup.md#the-new-type-solver)). Only the chosen editor's file is written. Running `init` again in the same folder rewrites `types/` (do it after upgrading jess), removing stubs this jess no longer writes, and leaves every other file that's there alone, listing it under `skipped`; `--force` replaces them too, backing each up first. A `.jessrc` from before the types were a folder has no aliases: add them, or rerun with `--force`.

```sh
jess init my-scripts --editor helix
```

```yaml
dir: my-scripts
editor: helix
types:
  folder: my-scripts/types
  definitions: my-scripts/types/jess.d.jsl
  files: 33
  removed: []
  aliases:
    dream: types/dream
    jess: types/jess
written:
- path: my-scripts/.jessrc
- path: my-scripts/main.jsl
- path: my-scripts/.helix/languages.toml
skipped: []
next_steps:
- 'run the example over a load order: jess run my-scripts/main.jsl path/to/openmw.cfg'
- 'or on one plugin: jess run my-scripts/main.jsl MyMod.esp'
# ...
```


## luau-types

{{ api_signature(value="jess luau-types <FOLDER>") }}

Writes the Luau types for the whole jess API into `FOLDER`, created if missing. [`init`](#init) does it for you, with the `.jessrc` and editor settings; this is the same folder, by hand.

```text
types/
  jess.d.jsl           the definitions: the jess global, the libraries' globals, every type
  jess/init.jsl        require '@jess'
  jess/log.jsl         require '@jess/log'
  jess/plugin.jsl      require '@jess/plugin', which is jess.plugin
  jess/package.jsl     ...one stub per @jess/ name
  dream/vfs.jsl        require '@dream/vfs'
  dream/archive/init.jsl, dream/archive/ba2.jsl, ...
```

`jess.d.jsl` declares the globals and every type: the hand-written API, and the generated record and NIF types. Every other file is a stub, one per module `require` resolves natively, returning a value of that module's type. A `.jessrc` aliases `jess` and `dream` to their folders, and from then on `require('@jess/log')` is typed the way Luau types any `require`: by finding a file. There is no list of overloads to outgrow, so a new module costs one file.

```sh
jess luau-types types
```

```yaml
folder: types
definitions: types/jess.d.jsl
files: 33
removed: []
aliases:
  dream: types/dream
  jess: types/jess
```

Point luau-lsp at the definitions file (`luau-lsp.types.definitionFiles`, `{ "@jess": "types/jess.d.jsl" }`, in VS Code; `--definitions:@jess=types/jess.d.jsl` elsewhere), turn on the new solver, and put the `aliases` in the `.jessrc` beside your scripts:

```json
{
  "languageMode": "strict",
  "aliases": {
    "dream": "types/dream",
    "jess": "types/jess"
  }
}
```

Rerunning it replaces jess's files and removes stubs a previous jess wrote that this one doesn't; nothing else in the folder is touched. The folder describes the jess you have installed, and it isn't shipped in release archives: regenerate it whenever you upgrade jess, or your editor will complain about functions that exist and miss ones that changed. Setup and usage: [Your First Script](@/docs/scripting/intro.md).

## completions

{{ api_signature(value="jess completions <SHELL>") }}

Prints a completion script for `<SHELL>`: `bash`, `elvish`, `fish`, `powershell`, or `zsh`. It completes commands, flags, and flag values (`--format`, `--show`, `--profile`, and so on).

```sh
jess completions fish > ~/.config/fish/completions/jess.fish
jess completions bash > ~/.local/share/bash-completion/completions/jess
jess completions zsh > ~/.zfunc/_jess
```

Where each shell wants it is in [Install](@/docs/getting-started/install.md#shell-completions). Regenerate after upgrading.

## man

{{ api_signature(value="jess man [--output <OUTPUT>]") }}

| Option | Does |
| --- | --- |
| `--output <OUTPUT>` | Write `jess.1` and one page per command into this folder (created if missing), instead of printing `jess(1)`. |

Without `--output`, prints the top-level `jess(1)` page as roff to stdout:

```sh
jess man > jess.1
```

With it, writes a page for every command and group, named after the command path with dashes: `jess.1`, `jess-plugin.1`, `jess-plugin-dump.1`, `jess-save-strip.1`, `jess-nif-check.1`, and so on. It then prints the files it wrote, sorted:

```sh
jess man --output ~/.local/share/man/man1
```

```yaml
written:
- /home/you/.local/share/man/man1/jess-completions.1
- /home/you/.local/share/man/man1/jess-init.1
- /home/you/.local/share/man/man1/jess-luau-types.1
- /home/you/.local/share/man/man1/jess-man.1
# ...
- /home/you/.local/share/man/man1/jess.1
```

Existing pages of the same name are overwritten, without a backup: they're generated, and `jess man` makes them again. A debug build also writes `jess-plugin-test-codec.1`; a release build doesn't have that command.
