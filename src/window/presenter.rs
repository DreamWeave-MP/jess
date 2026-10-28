//! The presenter: a thread of its own that draws every shown window, so presenting never waits on
//! whoever drives the frames, and they never wait on the display.
//!
//! A window draws when there is something new to draw: a commit applied changes on its device
//! since it last drew, it was shown a plan or resized, or its image wasn't ready last time. A
//! scene nothing changes costs nothing. It records under its device's frame lock, so a commit's
//! changes are drawn whole or not at all, then presents outside it.
//!
//! Locks are taken in one order: a window's screen, then what is coming to it, then its device's
//! frame lock. A commit takes the frame lock, then what is coming, and never a screen.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use super::{Imported, Showing, WindowError};
use crate::gpu::{Gpu, Import, Surface};

/// What the presenter draws a window with, shared by the window's handle and the presenter.
pub(super) struct Screen {
    pub(super) surface: Option<Surface>,
    pub(super) gpu: Arc<Gpu>,
    pub(super) showing: Option<Showing>,
    /// What went wrong drawing or resizing it, reported at the next present.
    pub(super) problem: Option<WindowError>,
    /// The device's commit count when it last drew.
    pub(super) drawn: u64,
    /// Whether it must draw again, whatever the commits say.
    pub(super) stale: bool,
    /// How many frames it presented.
    pub(super) frames: u64,
}

/// What a commit shows on a window next.
pub(super) enum Incoming {
    Show(Box<Showing>),
    Hide,
}

/// A window as the presenter sees it: its screen, and what a commit shows on it next.
pub(super) struct Shared {
    pub(super) screen: Mutex<Screen>,
    /// What a commit shows on it, taken when it next draws.
    pub(super) incoming: Mutex<Option<Incoming>>,
}

impl Shared {
    pub(super) fn screen(&self) -> MutexGuard<'_, Screen> {
        self.screen.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn incoming(&self) -> MutexGuard<'_, Option<Incoming>> {
        self.incoming.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Shows `incoming` when the presenter next draws it: what a commit does.
    pub(super) fn arrive(&self, incoming: Incoming) {
        *self.incoming() = Some(incoming);
    }

    /// Whether it has drawn everything there is: nothing waiting, nothing stale, every commit.
    fn settled(&self) -> bool {
        let screen = self.screen();
        if screen.surface.is_none() {
            return true;
        }
        let incoming = self.incoming().is_some();
        let Some(_) = &screen.showing else {
            return !incoming;
        };
        !incoming && !screen.stale && screen.drawn == screen.gpu.commits()
    }
}

/// The presenter's thread and what it serves.
struct Presenter {
    screens: Mutex<Vec<Weak<Shared>>>,
    /// How many times it was asked to draw.
    asked: Mutex<u64>,
    asking: Condvar,
    /// How many rounds it finished.
    rounds: Mutex<u64>,
    finished: Condvar,
}

static PRESENTER: OnceLock<Arc<Presenter>> = OnceLock::new();

/// How long the presenter waits before trying a window whose image wasn't ready again.
const RETRY: Duration = Duration::from_millis(8);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The presenter, its thread started the first time a window needs it.
fn presenter() -> &'static Arc<Presenter> {
    PRESENTER.get_or_init(|| {
        let presenter = Arc::new(Presenter {
            screens: Mutex::default(),
            asked: Mutex::default(),
            asking: Condvar::new(),
            rounds: Mutex::default(),
            finished: Condvar::new(),
        });
        let serving = Arc::clone(&presenter);
        std::thread::Builder::new()
            .name("jess presenter".to_owned())
            .spawn(move || serving.run())
            .expect("the system starts a thread");
        presenter
    })
}

/// Has the presenter draw `shared` from now on, for as long as it lives.
pub(super) fn serve(shared: &Arc<Shared>) {
    lock(&presenter().screens).push(Arc::downgrade(shared));
}

/// Asks the presenter to look for something to draw.
pub(super) fn wake() {
    let presenter = presenter();
    *lock(&presenter.asked) += 1;
    presenter.asking.notify_one();
}

/// Waits until the presenter has drawn everything there is to draw on `shared`, or `timeout`
/// passes: a window whose image never comes ready (covered, minimized) is not waited on forever.
pub(super) fn wait(shared: &[Arc<Shared>], timeout: Duration) {
    let presenter = presenter();
    let deadline = Instant::now() + timeout;
    while !shared.iter().all(|shared| shared.settled()) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return;
        };
        // A round that finishes between the look and the wait is caught a moment later.
        let rounds = lock(&presenter.rounds);
        let seen = *rounds;
        drop(
            presenter
                .finished
                .wait_timeout_while(rounds, left.min(RETRY), |rounds| *rounds == seen)
                .unwrap_or_else(PoisonError::into_inner),
        );
    }
}

/// What one try at drawing a window came to.
enum Drawing {
    /// Nothing new to draw, or nothing to draw it with.
    Nothing,
    /// It drew and presented.
    Drew,
    /// Its image wasn't ready: try again soon.
    NotReady,
}

impl Presenter {
    fn run(&self) {
        let mut seen = *lock(&self.asked);
        loop {
            let screens: Vec<Arc<Shared>> = {
                let mut screens = lock(&self.screens);
                screens.retain(|screen| screen.strong_count() > 0);
                screens.iter().filter_map(Weak::upgrade).collect()
            };
            let mut retry = false;
            for shared in &screens {
                retry |= matches!(draw(shared), Drawing::NotReady);
            }
            drop(screens);
            *lock(&self.rounds) += 1;
            self.finished.notify_all();
            let mut asked = lock(&self.asked);
            while *asked == seen {
                if retry {
                    asked = self
                        .asking
                        .wait_timeout(asked, RETRY)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0;
                    break;
                }
                asked = self.asking.wait(asked).unwrap_or_else(PoisonError::into_inner);
            }
            seen = *asked;
        }
    }
}

/// Draws `shared` if there is something new to draw.
// The screen stays locked until the frame is presented: what is destructured from it is borrowed
// that long, which the lint doesn't see.
#[allow(clippy::significant_drop_tightening)]
fn draw(shared: &Shared) -> Drawing {
    let mut screen = shared.screen();
    let incoming = shared.incoming().take();
    if let Some(incoming) = incoming {
        screen.showing = match incoming {
            Incoming::Show(showing) => Some(*showing),
            Incoming::Hide => None,
        };
        screen.stale = true;
    }
    let gpu = Arc::clone(&screen.gpu);
    let commits = gpu.commits();
    if !screen.stale && screen.drawn == commits {
        return Drawing::Nothing;
    }
    let Screen {
        surface,
        showing,
        problem,
        stale,
        drawn,
        frames,
        ..
    } = &mut *screen;
    let (Some(surface), Some(showing)) = (surface, showing.as_ref()) else {
        return Drawing::Nothing;
    };
    let frame = match surface.frame(&gpu) {
        Ok(Some(frame)) => frame,
        Ok(None) => {
            *stale = true;
            return Drawing::NotReady;
        }
        Err(failure) => {
            *problem = Some(failure.into());
            *stale = false;
            return Drawing::Nothing;
        }
    };
    let mut imports = vec![Import::Texture(&showing.target, frame.texture())];
    imports.extend(showing.imports.iter().map(|imported| match imported {
        Imported::Texture(name, texture) => Import::Texture(name, texture),
        Imported::Buffer(name, buffer) => Import::Buffer(name, buffer),
    }));
    let ran = {
        let _frame = gpu.frame();
        gpu.run(&showing.plan.get(), &showing.sets, &imports)
    };
    drop(imports);
    if let Err(failure) = ran {
        *problem = Some(failure.into());
    }
    gpu.present(frame);
    *drawn = commits;
    *stale = false;
    *frames += 1;
    Drawing::Drew
}
