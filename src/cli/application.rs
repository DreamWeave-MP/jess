//! Running an application target from the command line: frames at the pace `--pace` asks for, or
//! the target's own, until the application asks to stop, `--frames` runs out, or the person
//! running it presses Ctrl+C, and the application's diagnostics on stderr as they happen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use clap::Args;
use jess::lua::application::{Application, Pace};
use jess::lua::package::{LaunchOptions, Launched, Target};

/// Set when the person running the application asks it to stop.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// How an application target's run goes: how many frames, and at what pace.
#[derive(Debug, Args)]
pub struct RunOptions {
    /// For an application target: stop after N frames, shutdown handlers and all, unless it
    /// stops itself first.
    #[arg(long, value_name = "N")]
    pub frames: Option<u64>,
    /// For an application target: how often frames run: display follows its windows' displays,
    /// unpaced runs them as fast as they finish, and a number runs that many a second (1 to
    /// 1000). By default, display for a target that may open windows, and its fixed rate for one
    /// that may not. Its fixed steps are its own whatever the pace.
    #[arg(long, value_name = "PACE", value_parser = pace)]
    pub pace: Option<Pace>,
}

impl RunOptions {
    /// Whether the run asks for anything only an application target has.
    pub fn any(&self) -> bool {
        self.frames.is_some() || self.pace.is_some()
    }

    /// Refuses what is only for an application target when `target` isn't one, before anything
    /// of it runs.
    pub fn check(&self, target: &Target, package: &str) -> Result<(), String> {
        if target.kind == "application" {
            return Ok(());
        }
        let option = if self.frames.is_some() { "--frames" } else { "--pace" };
        Err(format!(
            "{option} is for application targets; target '{}' of {package} is a {} target",
            target.id, target.kind
        ))
    }

    /// How the application's host is set up for this run.
    pub fn launch(&self) -> LaunchOptions {
        LaunchOptions {
            pace: self.pace,
            ..LaunchOptions::default()
        }
    }
}

/// `--pace`: `display`, `unpaced`, or frames a second.
fn pace(text: &str) -> Result<Pace, String> {
    match text {
        "display" => return Ok(Pace::Display),
        "unpaced" => return Ok(Pace::Unpaced),
        _ => {}
    }
    match text.parse::<u32>() {
        Ok(rate @ 1..=1000) => Ok(Pace::Fixed(Duration::from_secs(1) / rate)),
        _ => Err(format!(
            "'{text}' is no pace: display, unpaced, or frames a second from 1 to 1000"
        )),
    }
}

/// Runs what a launch started: a command target's status from what its entry returned, an
/// application's from what it asked to stop with.
pub fn finish(launched: Launched, frames: Option<u64>) -> Result<u8, String> {
    match launched {
        Launched::Command(ran) => {
            let status = crate::run::script_status(&ran.runtime, &ran.values);
            crate::leave(ran.runtime, ran.values);
            status
        }
        Launched::Application(application) => drive(application, frames),
    }
}

fn drive(application: Application, frames: Option<u64>) -> Result<u8, String> {
    INTERRUPTED.store(false, Ordering::Release);
    catch_interrupts();
    application
        .run(frames, &INTERRUPTED, |line| eprintln!("jess: {line}"))
        .map_err(|error| error.to_string())
}

/// The first SIGINT or SIGTERM asks the application to stop, so its shutdown handlers run; the
/// handler is reset as it runs, so a second one ends the process at once. A signal the process
/// inherited as ignored (a background job's SIGINT) stays ignored.
#[cfg(unix)]
fn catch_interrupts() {
    extern "C" fn interrupted(_: libc::c_int) {
        INTERRUPTED.store(true, Ordering::Release);
    }

    // SAFETY: the handler only stores to an atomic, which is async-signal-safe; both sigaction
    // structs are fully initialized before the kernel reads them, and the query installs nothing.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        let handler: extern "C" fn(libc::c_int) = interrupted;
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESETHAND;
        libc::sigemptyset(&raw mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            let mut inherited: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal, std::ptr::null(), &raw mut inherited) != 0 || inherited.sa_sigaction == libc::SIG_IGN
            {
                continue;
            }
            libc::sigaction(signal, &raw const action, std::ptr::null_mut());
        }
    }
}

/// The first Ctrl+C or Ctrl+Break asks the application to stop, so its shutdown handlers run; a
/// second one, and every other console event, goes on to the next handler, Windows' default,
/// which ends the process. A console that ignores Ctrl+C never calls the handler for it.
#[cfg(windows)]
fn catch_interrupts() {
    /// `CTRL_C_EVENT` and `CTRL_BREAK_EVENT`; `TRUE` and `FALSE` as a handler returns them.
    const LAST_INTERRUPT: u32 = 1;
    const HANDLED: i32 = 1;
    const PASSED_ON: i32 = 0;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
    }

    // It runs on a thread of its own, which Windows starts for the event.
    extern "system" fn interrupted(event: u32) -> i32 {
        if event > LAST_INTERRUPT || INTERRUPTED.swap(true, Ordering::AcqRel) {
            return PASSED_ON;
        }
        HANDLED
    }

    let handler: unsafe extern "system" fn(u32) -> i32 = interrupted;
    // SAFETY: the handler is a plain function that lives as long as the process and only swaps
    // an atomic; adding it changes nothing else.
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

/// Elsewhere, the platform's default ends the process; an application that needs its shutdown
/// handlers there stops itself with `application.exit`.
#[cfg(not(any(unix, windows)))]
fn catch_interrupts() {}
