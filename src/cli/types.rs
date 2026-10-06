//! `jess luau-types`: jess's Luau types as a folder, over [`jess::lua::types`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::Serialize;

use crate::output::{self, Format};

#[derive(Debug, Args)]
pub struct LuauTypesArgs {
    /// The folder to write: jess.d.luau, and a stub module per name require resolves (jess/log.luau
    /// for @jess/log). Created if it's missing; jess's files in it are replaced, and stubs a
    /// previous jess wrote that this one doesn't are removed.
    folder: PathBuf,
}

/// What writing a types folder did, as `jess luau-types` and `jess init` report it.
#[derive(Serialize)]
pub struct TypesReport {
    /// The folder.
    folder: String,
    /// The definitions file, for luau-lsp's definitions setting.
    definitions: String,
    /// How many files were written.
    files: usize,
    /// Stale stubs removed.
    removed: Vec<String>,
    /// The `.luaurc` aliases that resolve `@scope/...` to the stubs, relative to the folder's parent.
    aliases: BTreeMap<String, String>,
}

impl TypesReport {
    pub fn new(folder: &Path, written: &jess::lua::types::Written) -> Self {
        let name = folder
            .file_name()
            .map_or_else(|| ".".to_owned(), |name| name.to_string_lossy().into_owned());
        Self {
            folder: folder.display().to_string(),
            definitions: folder.join(jess::lua::types::DEFINITIONS).display().to_string(),
            files: written.written.len(),
            removed: written.removed.iter().map(|path| path.display().to_string()).collect(),
            aliases: written
                .scopes
                .iter()
                .map(|scope| (scope.clone(), format!("{name}/{scope}")))
                .collect(),
        }
    }
}

pub fn run(args: &LuauTypesArgs, format: Format) -> Result<(), String> {
    let types = jess::lua::types::types().map_err(|error| error.to_string())?;
    let written = jess::lua::types::write(&args.folder, &types)
        .map_err(|error| format!("failed to write '{}': {error}", args.folder.display()))?;
    output::print(&TypesReport::new(&args.folder, &written), format)
}
