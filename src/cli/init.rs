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
    /// Set the folder up as a Package Source with this id (org.example.name) instead of a script
    /// folder: a jess.toml, a module, a test, and a command target, ready for jess package test,
    /// check, run, and pack.
    #[arg(long, value_name = "ID", conflicts_with = "application")]
    package: Option<String>,
    /// Set the folder up as an application with this package id (org.example.name): a jess.toml
    /// with an application target, and a module the host runs every frame that opens a window,
    /// draws a triangle in it, and asks a worker for a greeting. It runs as it stands, with
    /// jess package run --trusted.
    #[arg(long, value_name = "ID")]
    application: Option<String>,
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
        package: args.package.clone(),
        application: args.application.clone(),
    };
    let report = jess::lua::init::init(&args.dir, &options)
        .map_err(|error| format!("failed to set up '{}': {error}", args.dir.display()))?;
    let dir = args.dir.display();
    let mut next_steps = if args.application.is_some() {
        vec![
            format!("run it; close its window to stop: jess package run --trusted {dir}"),
            format!("type-check it: jess package check {dir}"),
            format!(
                "what runs every frame is {}; what the host calls in it, and when, is in types/jess.d.jsl under \
                 jess.application and jess.context",
                args.dir.join("src/app.jsl").display()
            ),
        ]
    } else if args.package.is_some() {
        vec![
            format!("test the package, from its source and packed: jess package test {dir}"),
            format!("type-check it: jess package check {dir}"),
            format!("run its command: jess package run --capability process.environment {dir}"),
        ]
    } else {
        let main = args.dir.join("main.jsl").display().to_string();
        vec![
            format!("run the example: jess run {main}"),
            format!("type-check it: jess check {main}"),
        ]
    };
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
