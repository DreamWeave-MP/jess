//! Running an application target from the command line: frames at a steady pace until the
//! application asks to stop, `--frames` runs out, or the person running it presses Ctrl+C, and
//! the application's diagnostics on stderr as they happen.

use std::sync::atomic::{AtomicBool, Ordering};

use jess::lua::application::Application;
use jess::lua::package::{Launched, Target};

/// Set when the person running the application asks it to stop.
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Refuses `--frames` for a target that isn't an application, before anything of it runs.
pub fn check_frames(frames: Option<u64>, target: &Target, package: &str) -> Result<(), String> {
    if frames.is_some() && target.kind != "application" {
        return Err(format!(
            "--frames is for application targets; target '{}' of {package} is a {} target",
            target.id, target.kind
        ));
    }
    Ok(())
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
