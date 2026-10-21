---
title: 'Editor Setup'
description: 'Types, completion, and strict checking for jess scripts in VS Code, Neovim, Zed, and Helix: what jess init writes, and how to do it by hand.'
weight: 50
extra:
  kind: guide
---

A jess script without types is a Lua script: it runs until the misspelled field, and then it tells you. With types, the editor underlines `logger.inof` before you run anything. This page gets you there.

The pieces are the same in every editor:

- **[luau-lsp](https://github.com/JohnnyMorganz/luau-lsp)**, the Luau language server. jess doesn't ship it.
- **`types/`**, the types for the whole jess API: `jess.d.jsl`, the definitions, one stub module per `@jess/...` and `@dream/...` name, and the built-in package's own modules, so `require('@jess/log')` and `require('@jess/json')` are typed by finding a file. `jess luau-types` writes it for the jess you have installed.
- **A `.jessrc`** aliasing `jess` and `dream` to their folders in `types/`.
- **Settings** telling luau-lsp to load `types/jess.d.jsl` as `@jess`, to use the plain `standard` platform (not Roblox), and not to look for a Rojo sourcemap.
- **Luau's new type solver**, which some of the types need. `jess init` turns it on. [See below](#the-new-type-solver).

## jess init

```sh
jess init my-scripts --editor vscode    # or neovim, zed, helix, none
```

```yaml
dir: my-scripts
editor: vscode
types:
  folder: my-scripts/types
  definitions: my-scripts/types/jess.d.jsl
  files: 44
  removed: []
  aliases:
    dream: types/dream
    jess: types/jess
written:
- path: my-scripts/.jessrc
- path: my-scripts/main.jsl
- path: my-scripts/.vscode/settings.json
skipped: []
next_steps:
- 'run the example: jess run my-scripts/main.jsl'
- 'vscode: install the Luau Language Server extension (JohnnyMorganz.luau-lsp) and open the folder'
- rerun jess init after updating jess to refresh types/
```

The folder is created if it's missing, and defaults to the current one. What goes in it:

| File | Editor | What it is |
| --- | --- | --- |
| `types/` | all | The types: `jess.d.jsl` and the module stubs. |
| `.jessrc` | all | `"languageMode": "strict"`, so every script in the folder is type-checked, `--!strict` or not, and the `jess` and `dream` aliases into `types/`. |
| `main.jsl` | all | A working script: a `@jess/log` logger reporting the version it runs under. `jess run my-scripts/main.jsl` runs it. |
| `.vscode/settings.json` | `vscode` (the default) | luau-lsp settings for VS Code. |
| `.nvim.lua` | `neovim` | A `vim.lsp.config` call for Neovim 0.11+. |
| `.zed/settings.json` | `zed` | luau-lsp settings for Zed's Luau extension. |
| `.helix/languages.toml` | `helix` | A `luau-lsp` language server for Helix. |

Run it again and it rewrites `types/` and skips everything else, since `main.jsl`, `.jessrc`, and your settings are yours now and jess won't clobber them. `--force` replaces them too, after backing each one up to `.bak`. `--editor none` writes only the first three.

Open the folder itself in your editor, not a parent of it. Every editor below finds its settings, and luau-lsp finds `types/`, relative to the folder you opened.

### Why a folder

jess's types used to be one file with one overloaded `require`, a function type listing every module name. Luau gives up on an overloaded function after eight overloads, and jess has more modules than that. A folder has no such ceiling: each module is a file, `.jessrc` aliases point `@jess/` and `@dream/` at them, and `require('@jess/package')` is typed the way every Luau `require` is, by resolving it.

That resolution is Luau's: `@jess/` and `@dream/` names through the aliases, and `./` paths relative to the file. Those are also the only names jess's own `require` resolves, apart from the exports of an activated package, so what type-checks is what runs. A package's exports type-check inside the package, under its own `.jessrc`, which [`jess init`](@/docs/getting-started/first-package.md) writes there too.

### After upgrading jess

`types/` describes exactly one version of jess; the first line of `types/jess.d.jsl` says which. After an upgrade, refresh it:

```sh
jess init my-scripts                 # rewrites types/, leaves the rest
jess luau-types my-scripts/types     # same thing, by hand
```

Skip this and the editor keeps approving calls the new jess doesn't have, and flagging the ones it added. Neither is a jess bug. Both are a stale folder.

A folder set up before the types were a folder has a `jess.d.jsl` at its top and a `.jessrc` without aliases. Delete the old file, rerun `jess init`, and add the `aliases` it prints to your `.jessrc` (or rerun with `--force`, which backs up and replaces `.jessrc` and your settings).

## VS Code

Install the **Luau Language Server** extension (`JohnnyMorganz.luau-lsp`), then open the folder. `jess init` writes `.vscode/settings.json`:

```json
{
  "luau-lsp.platform.type": "standard",
  "luau-lsp.sourcemap.enabled": false,
  "luau-lsp.fflags.enableNewSolver": true,
  "luau-lsp.types.definitionFiles": {
    "@jess": "types/jess.d.jsl"
  },
  "files.associations": {
    "*.jsl": "luau"
  }
}
```

By hand: put those keys in the workspace settings of whatever folder holds your scripts. The definitions path is relative to the workspace folder.

## Neovim

Neovim 0.11 or later, with `luau-lsp` on your `PATH`. `jess init --editor neovim` writes `.nvim.lua`:

```lua
local root = vim.fs.dirname(vim.fs.normalize(debug.getinfo(1, 'S').source:sub(2)))

vim.lsp.config('luau_lsp', {
  cmd = {
    'luau-lsp',
    'lsp',
    '--definitions:@jess=' .. vim.fs.joinpath(root, 'types', 'jess.d.jsl'),
    '--flag:LuauSolverV2=true',
  },
  filetypes = { 'luau' },
  root_markers = { '.jessrc', '.git' },
  settings = {
    ['luau-lsp'] = {
      platform = { type = 'standard' },
      sourcemap = { enabled = false },
    },
  },
})
vim.lsp.enable 'luau_lsp'
```

`root` is the folder the file sits in, so the definitions path is absolute and doesn't care where you started Neovim from.

**Neovim ignores `.nvim.lua` unless `exrc` is on.** Project files run code, so this is off by default. Add this to your `init.lua`:

```lua
vim.o.exrc = true
```

Neovim looks for `.nvim.lua` in the folder you start it in, and the first time it finds one it asks whether to trust it (`:trust` does the same). Start `nvim` from inside the script folder. If you start it from a parent folder, it never sees the file and luau-lsp never starts.

**Without exrc.** Put the same `vim.lsp.config` call in your `init.lua`, with `root` replaced by the script folder's absolute path. If you have [nvim-lspconfig](https://github.com/neovim/nvim-lspconfig) installed (with lazy.nvim or anything else), it already defines a `luau_lsp` config, and `vim.lsp.config` merges yours over it, so the call is the same:

```lua
-- lazy.nvim
{
  'neovim/nvim-lspconfig',
  config = function()
    vim.lsp.config('luau_lsp', {
      cmd = {
        'luau-lsp',
        'lsp',
        '--definitions:@jess=' .. vim.fn.expand '~/my-scripts/types/jess.d.jsl',
        '--flag:LuauSolverV2=true',
      },
      settings = {
        ['luau-lsp'] = { platform = { type = 'standard' }, sourcemap = { enabled = false } },
      },
    })
    vim.lsp.enable 'luau_lsp'
  end,
}
```

## Zed

Install Zed's **Luau** extension and open the folder. `jess init --editor zed` writes `.zed/settings.json`:

```json
{
  "lsp": {
    "luau-lsp": {
      "settings": {
        "roblox": { "enabled": false },
        "fflags": { "enable_new_solver": true },
        "definitions": ["types/jess.d.jsl"],
        "luau-lsp": {
          "platform": { "type": "standard" },
          "sourcemap": { "enabled": false }
        }
      }
    }
  }
}
```

## Helix

Put `luau-lsp` on your `PATH`, open the folder with `hx`, and `hx --health luau` tells you whether Helix found it. `jess init --editor helix` writes `.helix/languages.toml`:

```toml
[language-server.luau-lsp]
command = "luau-lsp"
args = ["lsp", "--definitions:@jess=types/jess.d.jsl", "--flag:LuauSolverV2=true"]

[language-server.luau-lsp.config.luau-lsp]
platform = { type = "standard" }
sourcemap = { enabled = false }

[[language]]
name = "luau"
language-servers = ["luau-lsp"]
```

Helix starts language servers in the folder it found `.helix` in, which is why the relative path works.

## The new type solver

Luau has two type checkers. The old one can't express parts of jess's types: fields with separate `read` and `write` types, and read-only tables such as a frozen export. Under the old solver those show up as errors on code that is correct and runs fine.

`luau-analyze` uses the new solver by default, and so does `jess package check`. luau-lsp doesn't, so every settings file `jess init` writes turns it on, and the by-hand snippets above include it. If you set an editor up some other way, this is the line to add:

| Editor | Setting |
| --- | --- |
| VS Code | `"luau-lsp.fflags.enableNewSolver": true` |
| Zed | `"fflags": { "enable_new_solver": true }` under `lsp.luau-lsp.settings` |
| Neovim, Helix, anything else | `--flag:LuauSolverV2=true` on the `luau-lsp lsp` command line |

## Checking without an editor

Types help, but the only authority is running the code. `jess run` stops at the first error with the script's line and a traceback. For a package, [`jess package check`](@/docs/cli/package.md#jess-package-check) type-checks every `.jsl` under it in strict mode, with each `require` resolved as the runtime resolves it, and runs nothing. When the editor and jess disagree, jess is running the code; the editor is guessing from a file that may be stale.

If something still isn't right, [Troubleshooting](@/docs/reference/troubleshooting.md#editors-and-types) has the usual suspects.
