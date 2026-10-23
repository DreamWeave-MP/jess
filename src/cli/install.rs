//! `jess install` and `jess uninstall`: applications on the PATH, the way cargo install puts
//! programs there, and the question asked before one is.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use clap::Args;
use jess::lua::image::Image;
use jess::lua::install::{self, InstallRequest, Prepared, Source};
use serde::Serialize;

use crate::output::{self, Format};

#[derive(Debug, Args)]
#[allow(clippy::struct_excessive_bools)] // Command-line switches, each independent of the others.
pub struct InstallArgs {
    /// What to install: a package id, taken from its publisher, or a file, a .jsw or an executable
    /// carrying one.
    #[arg(conflicts_with_all = ["path", "list"], required_unless_present_any = ["path", "list"])]
    what: Option<String>,
    /// Install the workspace this folder is in, packed, instead.
    #[arg(long, value_name = "WORKSPACE", conflicts_with = "list")]
    path: Option<PathBuf>,
    /// For a package id: the version to install, or a requirement the newest version installed
    /// must meet (^1.2). Without it, the newest release that isn't a pre-release.
    #[arg(long, value_name = "VERSION")]
    version: Option<String>,
    /// For a package id: resolve it from its publisher's packages and pack it here, rather than
    /// install the packed workspace a release carries.
    #[arg(long)]
    resolve: bool,
    /// List what is installed instead, and whether each command is still what was written.
    #[arg(long)]
    list: bool,
    /// The install root, whose bin folder holds the commands. Defaults to .jess in your home
    /// folder.
    #[arg(long, value_name = "DIR", env = "JESS_INSTALL_ROOT")]
    root: Option<PathBuf>,
    /// Take over command names another install or a stray file has, and install again what is
    /// installed already.
    #[arg(long)]
    force: bool,
    /// Install without asking, though the commands declare capabilities nothing granted before.
    /// Needed to install with no terminal to ask in.
    #[arg(long)]
    yes: bool,
    /// Ask no publisher. JESS_OFFLINE=1 says the same.
    #[arg(long, env = "JESS_OFFLINE", value_parser = clap::builder::FalseyValueParser::new())]
    offline: bool,
    /// A publisher to take the package from (repeatable): a DreamWeave site's dreamweave.json, or
    /// one project manifest.
    #[arg(long, value_name = "URL")]
    publisher: Vec<String>,
    /// A package index that says who publishes an id (repeatable). Replaces the default,
    /// AsHyAmS's; JESS_REGISTRY_URL does the same.
    #[arg(long, value_name = "URL", env = "JESS_REGISTRY_URL", value_delimiter = ',')]
    index: Vec<String>,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// What to remove: installs named by package id or by one of their commands.
    #[arg(required = true)]
    names: Vec<String>,
    /// The install root. Defaults to .jess in your home folder.
    #[arg(long, value_name = "DIR", env = "JESS_INSTALL_ROOT")]
    root: Option<PathBuf>,
}

/// What `jess install` reports when it changed nothing.
#[derive(Serialize)]
struct Unchanged<'a> {
    package: &'a str,
    version: &'a str,
    state: &'static str,
}

fn root(given: Option<&Path>) -> Result<PathBuf, String> {
    let root = match given {
        Some(root) => root.to_path_buf(),
        None => install::default_root()
            .ok_or_else(|| "found no home folder to install into; name a root with --root".to_owned())?,
    };
    std::path::absolute(&root).map_err(|error| format!("failed to resolve '{}': {error}", root.display()))
}

/// The source `args` names: a file that is there is installed from; anything else is a package id.
fn source(args: &InstallArgs) -> Result<Source, String> {
    if let Some(path) = &args.path {
        if args.version.is_some() {
            return Err("--version picks a version of a package id; a workspace is the version it is".to_owned());
        }
        let path = std::path::absolute(path).map_err(|error| format!("failed to resolve '{}': {error}", path.display()))?;
        return Ok(Source::Workspace(path));
    }
    let what = args.what.clone().unwrap_or_default();
    let file = Path::new(&what);
    if file.is_file() {
        if args.version.is_some() {
            return Err(format!("--version picks a version of a package id; '{what}' is a file"));
        }
        let image = Image::open(file).map_err(|error| format!("failed to open '{what}': {error}"))?;
        return Ok(Source::File(image));
    }
    Ok(Source::Package {
        id: what,
        requirement: args.version.clone(),
    })
}

/// Tells whoever is asked what installing `prepared` would do.
fn describe(prepared: &Prepared, root: &Path) {
    let from = prepared
        .manifest
        .as_ref()
        .map_or_else(String::new, |manifest| format!(" from {manifest}"));
    eprintln!(
        "{} {}{from} installs into {}:",
        prepared.package,
        prepared.version,
        root.join("bin").display()
    );
    let width = prepared.commands.iter().map(|command| command.name.len()).max().unwrap_or(0);
    for command in &prepared.commands {
        let capabilities = if command.capabilities.is_empty() {
            "no capabilities".to_owned()
        } else {
            command.capabilities.join(", ")
        };
        eprintln!("  {:<width$}  runs {}, holding {capabilities}", command.name, command.target);
    }
    if let Some(previous) = &prepared.previous {
        eprintln!("It replaces {} {}.", prepared.package, previous.version);
    }
    for take in &prepared.takes {
        match (take.kind.as_str(), &take.from) {
            ("changed", _) => eprintln!("It replaces '{}', which has changed since it was installed.", take.name),
            (_, Some(from)) => eprintln!("It takes '{}' over from {from}.", take.name),
            (_, None) => eprintln!("It replaces the file '{}', which no install owns.", take.name),
        }
    }
}

/// Says what was left where it is, and why.
fn warn_kept(kept: &[String]) {
    for why in kept {
        eprintln!("warning: {why}, so it was left where it is");
    }
}

/// Whoever runs it agrees: with `--yes`, or answering yes in a terminal. With no terminal and no
/// `--yes`, nobody was asked, so nothing is installed.
fn agreed(yes: bool) -> Result<bool, String> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err("installing it needs your yes, and there is no terminal to ask in; pass --yes".to_owned());
    }
    eprint!("Install? [y/N] ");
    drop(std::io::stderr().flush());
    let mut answer = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut answer)
        .map_err(|error| format!("failed to read the answer: {error}"))?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes" | "YES"))
}

/// Whether `bin` is a folder on the PATH.
fn on_path(bin: &Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let wanted = std::fs::canonicalize(bin).unwrap_or_else(|_| bin.to_path_buf());
    std::env::split_paths(&path).any(|folder| std::fs::canonicalize(&folder).unwrap_or(folder) == wanted)
}

pub fn run(args: &InstallArgs, format: Format) -> Result<u8, String> {
    let root = root(args.root.as_deref())?;
    if args.list {
        let lua = crate::run::lua_default()?;
        let listed = install::list(&lua, &root).map_err(|error| error.to_string())?;
        output::print(&listed, format)?;
        return Ok(0);
    }
    let source = source(args)?;
    // Installing is the host acting, in a runtime that runs no package code; reaching publishers is
    // its own grant and never a package's, and only a package id needs one.
    let lua = if matches!(source, Source::Package { .. }) && !args.offline {
        crate::run::lua(&["network.dns.resolve".to_owned(), "network.tcp.connect".to_owned()])?
    } else {
        crate::run::lua_default()?
    };
    let request = InstallRequest {
        root: root.clone(),
        source,
        force: args.force,
        resolve: args.resolve,
        offline: args.offline,
        indexes: if args.index.is_empty() {
            vec![crate::workspace::DEFAULT_INDEX.to_owned()]
        } else {
            args.index.clone()
        },
        publishers: args.publisher.clone(),
    };
    let prepared = install::prepare(&lua, &request).map_err(|error| error.to_string())?;
    for note in &prepared.notes {
        eprintln!("note: {note}");
    }
    if prepared.unchanged {
        install::abandon(&prepared);
        let unchanged = Unchanged {
            package: &prepared.package,
            version: &prepared.version,
            state: "already installed",
        };
        return output::print(&unchanged, format).map(|()| 0);
    }
    describe(&prepared, &root);
    let agreed = if prepared.consent { agreed(args.yes) } else { Ok(true) };
    match agreed {
        Ok(true) => {}
        Ok(false) => {
            install::abandon(&prepared);
            return Err("nothing was installed".to_owned());
        }
        Err(problem) => {
            install::abandon(&prepared);
            return Err(problem);
        }
    }
    if option_env!("L3I_UNVERIFIED_TOOLCHAIN").is_some() {
        eprintln!("warning: this jess is an unverified development build, and the commands carry it");
    }
    let installed = install::commit(&lua, &prepared).map_err(|error| error.to_string())?;
    output::print(&installed, format)?;
    warn_kept(&installed.kept);
    let bin = root.join("bin");
    if !on_path(&bin) {
        eprintln!(
            "warning: {} is not on your PATH; add it to run what you installed by name",
            bin.display()
        );
    }
    Ok(0)
}

pub fn uninstall(args: &UninstallArgs, format: Format) -> Result<u8, String> {
    let root = root(args.root.as_deref())?;
    let lua = crate::run::lua_default()?;
    let removed = install::uninstall(&lua, &root, &args.names).map_err(|error| error.to_string())?;
    output::print(&removed, format)?;
    for install in &removed {
        warn_kept(&install.kept);
    }
    Ok(0)
}
