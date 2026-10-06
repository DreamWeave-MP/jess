//! Setting up a folder for jess scripts, as `jess init` does.
//!
//! It holds jess's Luau types (the folder of [`crate::lua::types`]), a strict `.luaurc` aliasing its
//! modules, a working `main.luau`, and luau-lsp settings for an editor.
//!
//! The settings turn on Luau's new type solver, which some of jess's types (read and write
//! property types) need.
//!
//! [`init`] writes the types into `types/` and [`files`] beside it. Re-running it refreshes
//! `types/`, which belongs to jess, and leaves every other file alone unless
//! [`InitOptions::force`] is set, since those are the user's to change.

use std::io;
use std::path::{Path, PathBuf};

pub use crate::lua::files::Written;
use crate::lua::files::{Backup, write_file_with};

/// The types folder, refreshed on every run.
pub const TYPES_FOLDER: &str = "types";

/// Which editor to write luau-lsp settings for.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Editor {
    /// Visual Studio Code with the luau-lsp extension: `.vscode/settings.json`.
    #[default]
    VsCode,
    /// Neovim 0.11 or later with luau-lsp: `.nvim.lua`, which Neovim reads with `exrc` set.
    Neovim,
    /// Zed with the Luau extension: `.zed/settings.json`.
    Zed,
    /// Helix: `.helix/languages.toml`.
    Helix,
    /// No editor settings.
    None,
}

impl Editor {
    /// Every editor, [`Editor::None`] last.
    pub const ALL: [Self; 5] = [Self::VsCode, Self::Neovim, Self::Zed, Self::Helix, Self::None];

    /// The editor's name as `jess init --editor` takes it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::VsCode => "vscode",
            Self::Neovim => "neovim",
            Self::Zed => "zed",
            Self::Helix => "helix",
            Self::None => "none",
        }
    }

    /// The settings file this editor reads, relative to the folder; `None` for [`Editor::None`].
    #[must_use]
    pub const fn settings_file(self) -> Option<&'static str> {
        match self {
            Self::VsCode => Some(".vscode/settings.json"),
            Self::Neovim => Some(".nvim.lua"),
            Self::Zed => Some(".zed/settings.json"),
            Self::Helix => Some(".helix/languages.toml"),
            Self::None => None,
        }
    }

    /// What to do in the editor to pick the settings up, if anything.
    #[must_use]
    pub const fn hint(self) -> Option<&'static str> {
        match self {
            Self::VsCode => Some("install the Luau Language Server extension (JohnnyMorganz.luau-lsp) and open the folder"),
            Self::Neovim => Some(
                "Neovim reads .nvim.lua only with 'exrc' on: add vim.o.exrc = true to your init.lua (Neovim asks once \
                 whether to trust the file); it needs Neovim 0.11+ and luau-lsp on your PATH",
            ),
            Self::Zed => Some("install Zed's Luau extension and open the folder"),
            Self::Helix => Some("put luau-lsp on your PATH and open the folder with hx; hx --health luau checks it"),
            Self::None => None,
        }
    }
}

/// How [`init`] writes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InitOptions {
    /// Which editor to write settings for.
    pub editor: Editor,
    /// Replace files that are already there (after backing them up) instead of skipping them.
    pub force: bool,
}

/// What [`init`] did.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InitReport {
    /// The folder.
    pub dir: PathBuf,
    /// The types folder.
    pub types: PathBuf,
    /// What writing the types folder did.
    pub types_written: crate::lua::types::Written,
    /// The files written beside it, with the backups of any replaced.
    pub written: Vec<Written>,
    /// Files left alone because they were already there.
    pub skipped: Vec<PathBuf>,
}

/// The files for a script folder besides the types, as (path relative to the folder, contents):
/// the `.luaurc` aliasing `types`' modules, the example, and the editor's settings.
#[must_use]
pub fn files(editor: Editor, types: &crate::lua::types::Types) -> Vec<(&'static str, String)> {
    let mut files = vec![(".luaurc", types.luaurc(TYPES_FOLDER)), ("main.luau", MAIN.to_owned())];
    let settings = match editor {
        Editor::VsCode => Some(VSCODE),
        Editor::Neovim => Some(NEOVIM),
        Editor::Zed => Some(ZED),
        Editor::Helix => Some(HELIX),
        Editor::None => None,
    };
    if let (Some(path), Some(settings)) = (editor.settings_file(), settings) {
        files.push((path, settings.to_owned()));
    }
    files
}

/// Writes the types into `dir/types` and [`files`] into `dir`, creating it and any folders the
/// files need.
///
/// `types/` is always rewritten; other files already there are skipped, unless `options.force`,
/// which replaces them after a backup.
///
/// # Errors
///
/// Returns an error if the types can't be made, a folder can't be created, or a file can't be
/// written.
pub fn init(dir: &Path, options: &InitOptions) -> io::Result<InitReport> {
    let types = crate::lua::types::types().map_err(|error| io::Error::other(error.to_string()))?;
    let folder = dir.join(TYPES_FOLDER);
    let mut report = InitReport {
        dir: dir.to_path_buf(),
        types_written: crate::lua::types::write(&folder, &types)?,
        types: folder,
        ..InitReport::default()
    };
    for (name, contents) in files(options.editor, &types) {
        let path = dir.join(name);
        if path.exists() && !options.force {
            report.skipped.push(path);
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let written = write_file_with(&path, contents.as_bytes(), Backup::Keep).map_err(io::Error::other)?;
        report.written.push(written);
    }
    Ok(report)
}

const MAIN: &str = include_str!("templates/main.luau");

const VSCODE: &str = r#"{
  "luau-lsp.platform.type": "standard",
  "luau-lsp.sourcemap.enabled": false,
  "luau-lsp.fflags.enableNewSolver": true,
  "luau-lsp.types.definitionFiles": {
    "@jess": "types/jess.d.luau"
  },
  "files.associations": {
    "*.luau": "luau"
  }
}
"#;

const ZED: &str = r#"{
  "lsp": {
    "luau-lsp": {
      "settings": {
        "roblox": {
          "enabled": false
        },
        "fflags": {
          "enable_new_solver": true
        },
        "definitions": ["types/jess.d.luau"],
        "luau-lsp": {
          "platform": {
            "type": "standard"
          },
          "sourcemap": {
            "enabled": false
          }
        }
      }
    }
  }
}
"#;

const HELIX: &str = r#"# luau-lsp with jess's types, for the scripts in this folder. Helix starts language servers in
# the folder it finds .helix in, so the definitions path is relative to here. LuauSolverV2 is the
# new type solver, which some of jess's types (view fields that read one type and take another) need.
[language-server.luau-lsp]
command = "luau-lsp"
args = ["lsp", "--definitions:@jess=types/jess.d.luau", "--flag:LuauSolverV2=true"]

[language-server.luau-lsp.config.luau-lsp]
platform = { type = "standard" }
sourcemap = { enabled = false }

[[language]]
name = "luau"
language-servers = ["luau-lsp"]
"#;

const NEOVIM: &str = r"-- luau-lsp with jess's types, for the scripts in this folder. Neovim reads this file only with
-- 'exrc' on (vim.o.exrc = true in your init.lua), and asks once whether to trust it. Needs
-- Neovim 0.11 or later and luau-lsp on your PATH.
local root = vim.fs.dirname(vim.fs.normalize(debug.getinfo(1, 'S').source:sub(2)))

vim.lsp.config('luau_lsp', {
  cmd = {
    'luau-lsp',
    'lsp',
    '--definitions:@jess=' .. vim.fs.joinpath(root, 'types', 'jess.d.luau'),
    -- The new type solver: some of jess's types (view fields that read one type and take
    -- another) need it.
    '--flag:LuauSolverV2=true',
  },
  filetypes = { 'luau' },
  root_markers = { '.luaurc', '.git' },
  settings = {
    ['luau-lsp'] = {
      platform = { type = 'standard' },
      sourcemap = { enabled = false },
    },
  },
})
vim.lsp.enable 'luau_lsp'
";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerunning_refreshes_the_types_and_keeps_the_users_files() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("scripts");
        let options = InitOptions {
            editor: Editor::Helix,
            force: false,
        };
        let first = init(&folder, &options).unwrap();
        assert_eq!(first.written.len(), 3);
        assert_eq!(first.skipped, Vec::<PathBuf>::new());
        assert!(folder.join(".helix/languages.toml").is_file());
        assert!(folder.join("types/jess.d.luau").is_file());
        assert!(folder.join("types/jess/log.luau").is_file());

        std::fs::write(folder.join("main.luau"), "print('mine')").unwrap();
        std::fs::write(folder.join("types/jess.d.luau"), "stale").unwrap();
        std::fs::write(folder.join("types/jess/gone.luau"), "return nil").unwrap();
        let second = init(&folder, &options).unwrap();
        assert_eq!(second.written, []);
        assert_eq!(second.skipped.len(), 3);
        assert_eq!(second.types_written.removed, [folder.join("types/jess/gone.luau")]);
        assert_eq!(std::fs::read_to_string(folder.join("main.luau")).unwrap(), "print('mine')");
        assert_eq!(
            std::fs::read_to_string(folder.join("types/jess.d.luau")).unwrap(),
            crate::lua::definitions()
        );

        let forced = init(&folder, &InitOptions { force: true, ..options }).unwrap();
        assert_eq!(forced.written.len(), 3);
        let main = forced
            .written
            .iter()
            .find(|written| written.path.ends_with("main.luau"))
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(main.backup.as_ref().unwrap()).unwrap(),
            "print('mine')"
        );
    }
}
