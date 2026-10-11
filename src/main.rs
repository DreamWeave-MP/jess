// clap turns doc comments into terminal help, where Markdown backticks print literally.
#![allow(clippy::doc_markdown)]

use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use clap_complete::Shell;

#[path = "cli/init.rs"]
mod init;
#[path = "cli/output.rs"]
mod output;
#[path = "cli/package.rs"]
mod package;
#[path = "cli/run.rs"]
mod run;
#[path = "cli/test.rs"]
mod test;
#[path = "cli/types.rs"]
mod types;

/// Where `jess --help` sends people new to jess.
const START_HERE: &str = "Start here:
  jess init my-scripts        Set up a folder for Luau scripts, with types for your editor
  jess run script.luau        Run a Luau script with the jess API

Docs: https://DreamWeave-MP.github.io/jess";

/// The version line. A build without l3i's verified toolchain (`L3I_UNVERIFIED_TOOLCHAIN`, as the
/// no-LTO development setup sets) runs Luau without cross-language LTO, so it says so: the bench
/// harness refuses to measure it.
const VERSION: &str = if option_env!("L3I_UNVERIFIED_TOOLCHAIN").is_some() {
    concat!(
        env!("CARGO_PKG_VERSION"),
        " (unverified toolchain, no cross-language LTO: not for measuring)"
    )
} else {
    env!("CARGO_PKG_VERSION")
};

#[derive(Debug, Parser)]
#[command(
    name = "jess",
    version = VERSION,
    propagate_version = true,
    about = "Run Luau applications and manage packages",
    after_help = START_HERE
)]
struct Cli {
    /// Output format: YAML (the default), compact one-line JSON, or TOML.
    #[arg(long, global = true, value_enum)]
    format: Option<output::Format>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a Luau script with the jess API.
    Run(Box<run::RunArgs>),
    /// Run Luau test modules returning an @jess/test suite.
    Test(Box<test::TestArgs>),
    /// Pack a Package Source folder into a JPK.
    Pack(Box<package::PackArgs>),
    /// Inspect, resolve, and test packages: Package Source folders and JPKs.
    Package(Box<package::PackageArgs>),
    /// Set up a folder for Luau scripts: jess's types, an example, and editor settings.
    Init(Box<init::InitArgs>),
    /// Write jess's Luau types into a folder: jess.d.luau and one stub module per require name.
    LuauTypes(Box<types::LuauTypesArgs>),
    /// Generate shell completion scripts.
    Completions(Box<CompletionsArgs>),
    /// Generate man pages: jess(1) to stdout, or one page per command into a folder.
    Man(Box<ManArgs>),
}

#[derive(Debug, Parser)]
struct CompletionsArgs {
    /// The shell to generate completions for.
    shell: Shell,
}

#[derive(Debug, Parser)]
struct ManArgs {
    /// Folder to write jess.1 and a page per subcommand (jess-package-inspect.1, ...) into.
    #[arg(long)]
    output: Option<std::path::PathBuf>,
}

fn main() -> ExitCode {
    restore_default_sigpipe();
    let matches = command().get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Leaves `value` for the process's exit to reclaim, for what a command holds when it's done:
/// the exit comes right after, and freeing a whole load order piece by piece only delays it.
pub fn leave<T>(value: T) {
    std::mem::forget(value);
}

fn command() -> clap::Command {
    Cli::command()
}

/// Prints jess(1), or writes a page for every command into `folder` and reports them.
fn man(folder: Option<&std::path::Path>, format: output::Format) -> Result<(), String> {
    fn pages(
        command: clap::Command,
        prefix: &str,
        folder: &std::path::Path,
        written: &mut Vec<String>,
    ) -> Result<(), String> {
        let name = if prefix.is_empty() {
            command.get_name().to_owned()
        } else {
            format!("{prefix}-{}", command.get_name())
        };
        for subcommand in command.get_subcommands().filter(|subcommand| !subcommand.is_hide_set()) {
            pages(subcommand.clone(), &name, folder, written)?;
        }
        let page = clap_mangen::Man::new(command.display_name(&name).bin_name(name.replace('-', " ")))
            .source(concat!("jess ", env!("CARGO_PKG_VERSION")));
        let path = page
            .generate_to(folder)
            .map_err(|error| format!("failed to write the man page for {name}: {error}"))?;
        written.push(path.display().to_string());
        Ok(())
    }

    let mut command = command().disable_help_subcommand(true);
    command.build();
    let Some(folder) = folder else {
        return clap_mangen::Man::new(command)
            .render(&mut std::io::stdout())
            .map_err(|error| format!("failed to render man page: {error}"));
    };
    std::fs::create_dir_all(folder).map_err(|error| format!("failed to create '{}': {error}", folder.display()))?;
    let mut written = Vec::new();
    pages(command, "", folder, &mut written)?;
    written.sort();
    output::print(&serde_json::json!({ "written": written }), format)
}

/// Rust ignores SIGPIPE, turning `jess ... | head` into write errors and panics. Restore the
/// default so a closed pipe ends the process quietly, like other command-line tools.
fn restore_default_sigpipe() {
    #[cfg(unix)]
    // SAFETY: called first thing in `main`, before any threads exist; SIG_DFL is always valid.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

fn run(cli: Cli) -> Result<u8, String> {
    let Some(subcommand) = cli.command else {
        return command()
            .print_help()
            .map(|()| 0)
            .map_err(|error| format!("failed to print the help: {error}"));
    };
    match subcommand {
        Command::Completions(args) => {
            clap_complete::generate(args.shell, &mut command(), "jess", &mut std::io::stdout());
            Ok(0)
        }
        Command::Man(args) => man(args.output.as_deref(), cli.format.unwrap_or(output::Format::Yaml)).map(|()| 0),
        Command::LuauTypes(args) => types::run(&args, cli.format.unwrap_or(output::Format::Yaml)).map(|()| 0),
        Command::Run(args) => run::run(&args),
        Command::Test(args) => test::run(&args, cli.format.unwrap_or(output::Format::Yaml)).map(|()| 0),
        Command::Pack(args) => package::pack(&args, cli.format.unwrap_or(output::Format::Yaml)).map(|()| 0),
        Command::Package(args) => package::run(&args, cli.format.unwrap_or(output::Format::Yaml)),
        Command::Init(args) => init::run(&args, cli.format.unwrap_or(output::Format::Yaml)).map(|()| 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every command below `command`, with its full name.
    fn all_commands(command: &clap::Command, prefix: &str, out: &mut Vec<(String, clap::Command)>) {
        let name = format!("{prefix} {}", command.get_name()).trim().to_owned();
        for subcommand in command.get_subcommands() {
            all_commands(subcommand, &name, out);
        }
        out.push((name, command.clone()));
    }

    #[test]
    fn help_explains_every_argument_in_plain_text() {
        let mut command = command();
        command.build();
        let mut commands = Vec::new();
        all_commands(&command, "", &mut commands);
        for (name, command) in commands {
            if name != "jess" && !name.ends_with(" help") {
                assert!(command.get_about().is_some(), "{name} has no summary");
            }
            let texts = command.get_about().into_iter().chain(command.get_long_about());
            for text in texts.map(ToString::to_string) {
                assert!(!text.contains('`'), "{name}'s summary has backticks: {text}");
            }
            for arg in command.get_arguments().filter(|arg| !arg.is_hide_set()) {
                let id = arg.get_id();
                let help = arg.get_help().map(ToString::to_string);
                assert!(help.is_some(), "{name}'s {id} has no help");
                let help = help.unwrap();
                assert!(!help.contains('`'), "{name}'s {id} help has backticks: {help}");
            }
        }
    }

    #[test]
    fn native_format_commands_are_not_registered() {
        let command = command();
        for name in ["plugin", "save", "nif"] {
            assert!(command.find_subcommand(name).is_none(), "{name}");
            assert!(Cli::try_parse_from(["jess", name]).is_err(), "{name}");
        }
        for name in ["run", "test", "pack", "package", "init", "luau-types", "completions", "man"] {
            assert!(command.find_subcommand(name).is_some(), "{name}");
        }
    }
}
