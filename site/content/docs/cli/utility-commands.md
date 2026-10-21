---
title: 'init, luau-types, completions, man'
description: 'The commands that set up your editor and shell.'
weight: 50
extra:
  kind: reference
---

The commands that run no Luau and open no package: they write things for your editor and your shell. They take the [global options](@/docs/cli/global-options.md), but only `init`, `luau-types`, and `man --output` print a document that `--format` shapes.

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
| `main.jsl` | A working example: a strict script that makes an [`@jess/log`](@/docs/api/log.md) logger and reports the jess version. `jess run main.jsl` runs it. |
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
  files: 24
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
- 'run the example: jess run my-scripts/main.jsl'
- 'helix: put luau-lsp on your PATH and open the folder with hx; hx --health luau checks it'
- rerun jess init after updating jess to refresh types/
```

## luau-types

{{ api_signature(value="jess luau-types <FOLDER>") }}

Writes the Luau types for the whole jess API into `FOLDER`, created if missing. [`init`](#init) does it for you, with the `.jessrc` and editor settings; this is the same folder, by hand.

```text
types/
  jess.d.jsl           the definitions: the jess global, the libraries' globals, every type
  jess/init.jsl        require '@jess'
  jess/log.jsl         require '@jess/log'
  jess/context.jsl     require '@jess/context', which is jess.context
  jess/package.jsl     ...one stub per @jess/ name
  dream/fs.jsl         require '@dream/fs'
  dream/bytes.jsl, dream/process.jsl, dream/tcp.jsl, ...
```

`jess.d.jsl` declares the globals and every type: the hand-written API and the generated library declarations. Every other file is a stub, one per module `require` resolves natively, returning a value of that module's type. A `.jessrc` aliases `jess` and `dream` to their folders, and from then on `require('@jess/log')` is typed the way Luau types any `require`: by finding a file. There is no list of overloads to outgrow, so a new module costs one file.

```sh
jess luau-types types
```

```yaml
folder: types
definitions: types/jess.d.jsl
files: 24
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

The built-in package's exports (`@jess/style`, `@jess/json`, and the rest) aren't stubs here: they are Luau with their own types, and `jess package check` resolves them from the package itself.

## completions

{{ api_signature(value="jess completions <SHELL>") }}

Prints a completion script for `<SHELL>`: `bash`, `elvish`, `fish`, `powershell`, or `zsh`. It completes commands, flags, and flag values (`--format`, `--editor`, and so on).

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

With it, writes a page for every command and group, named after the command path with dashes: `jess.1`, `jess-package.1`, `jess-package-inspect.1`, `jess-workspace-sync.1`, and so on. It then prints the files it wrote, sorted:

```sh
jess man --output ~/.local/share/man/man1
```

```yaml
written:
- /home/you/.local/share/man/man1/jess-completions.1
- /home/you/.local/share/man/man1/jess-init.1
- /home/you/.local/share/man/man1/jess-luau-types.1
- /home/you/.local/share/man/man1/jess-man.1
- /home/you/.local/share/man/man1/jess-pack.1
- /home/you/.local/share/man/man1/jess-package-check.1
# ...
- /home/you/.local/share/man/man1/jess-workspace.1
- /home/you/.local/share/man/man1/jess.1
```

Twenty-one pages: one per command, one per group, and `jess.1`. Existing pages of the same name are overwritten, without a backup: they're generated, and `jess man` makes them again.
