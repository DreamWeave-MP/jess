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
/// handler is reset as it runs, so a second one ends the process at once.
#[cfg(unix)]
fn catch_interrupts() {
    extern "C" fn interrupted(_: libc::c_int) {
        INTERRUPTED.store(true, Ordering::Release);
    }

    // SAFETY: the handler only stores to an atomic, which is async-signal-safe, and the
    // sigaction is fully initialized before it is installed.
    unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        let handler: extern "C" fn(libc::c_int) = interrupted;
        action.sa_sigaction = handler as libc::sighandler_t;
        action.sa_flags = libc::SA_RESETHAND;
        libc::sigemptyset(&raw mut action.sa_mask);
        for signal in [libc::SIGINT, libc::SIGTERM] {
            libc::sigaction(signal, &raw const action, std::ptr::null_mut());
        }
    }
}

/// Windows ends the process on Ctrl+C, without the application's shutdown handlers; an
/// application that needs them stops itself with `application.exit`.
#[cfg(not(unix))]
fn catch_interrupts() {}
