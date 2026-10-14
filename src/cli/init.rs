//! `jess init`: a folder for Luau scripts, over [`jess::lua::init`].

use std::path::PathBuf;

use clap::{Args, ValueEnum};
use jess::lua::init::{Editor, InitOptions};
use serde::Serialize;

use crate::output::{self, Format, Written};

#[derive(Debug, Args)]
pub struct InitArgs {
    /// The folder to set up; created if it's missing. Defaults to the current folder.
    #[arg(default_value = ".")]
    dir: PathBuf,
    /// The editor to write luau-lsp settings for, so it knows jess's types.
    #[arg(long, value_enum, default_value_t = CliEditor::Vscode)]
    editor: CliEditor,
    /// Replace files that are already there, after backing them up. Without it, only the types
    /// folder is rewritten.
    #[arg(long)]
    force: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CliEditor {
    /// Visual Studio Code with the Luau Language Server extension: .vscode/settings.json.
    Vscode,
    /// Neovim 0.11+ with luau-lsp: .nvim.lua, read with 'exrc' on.
    Neovim,
    /// Zed with the Luau extension: .zed/settings.json.
    Zed,
    /// Helix with luau-lsp: .helix/languages.toml.
    Helix,
    /// No editor settings.
    None,
}

impl From<CliEditor> for Editor {
    fn from(editor: CliEditor) -> Self {
        match editor {
            CliEditor::Vscode => Self::VsCode,
            CliEditor::Neovim => Self::Neovim,
            CliEditor::Zed => Self::Zed,
            CliEditor::Helix => Self::Helix,
            CliEditor::None => Self::None,
        }
    }
}

#[derive(Serialize)]
struct InitResult {
    dir: String,
    editor: &'static str,
    types: crate::types::TypesReport,
    written: Vec<Written>,
    /// Files already there, left as they are.
    skipped: Vec<String>,
    next_steps: Vec<String>,
}

pub fn run(args: &InitArgs, format: Format) -> Result<(), String> {
    let editor = Editor::from(args.editor);
    let options = InitOptions {
        editor,
        force: args.force,
    };
    let report = jess::lua::init::init(&args.dir, &options)
        .map_err(|error| format!("failed to set up '{}': {error}", args.dir.display()))?;
    let main = args.dir.join("main.jsl");
    let mut next_steps = vec![format!("run the example: jess run {}", main.display())];
    if let Some(hint) = editor.hint() {
        next_steps.push(format!("{}: {hint}", editor.name()));
    }
    next_steps.push("rerun jess init after updating jess to refresh types/".to_owned());
    let result = InitResult {
        dir: report.dir.display().to_string(),
        editor: editor.name(),
        types: crate::types::TypesReport::new(&report.types, &report.types_written),
        written: report
            .written
            .into_iter()
            .map(|written| Written {
                path: written.path.display().to_string(),
                backup: written.backup.map(|path| path.display().to_string()),
            })
            .collect(),
        skipped: report.skipped.iter().map(|path| path.display().to_string()).collect(),
        next_steps,
    };
    output::print(&result, format)
}
