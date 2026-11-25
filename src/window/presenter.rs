//! The presenter: a thread for each window that draws it, so presenting never waits on whoever
//! drives the frames, they never wait on a display, and no window waits on another's.
//!
//! A window draws when there is something new to draw: a commit applied changes on its device
//! since it last drew, it was shown a plan or resized, its present mode changed, or its last try
//! didn't present. A scene nothing changes costs nothing. It records under its device's frame
//! lock, so a commit's changes are drawn whole or not at all, then presents outside it. A try that
//! fails presents nothing and counts no frame: the image goes back unshown, the failure is
//! reported once, and the window is tried again until it draws.
//!
//! Each window has its own thread because taking a surface's next image waits: a refresh for a
//! window that waits on its display, and up to the driver's timeout for one that can't draw
//! (minimized, covered). One thread serving every window in turn made each wait on the slowest.
//!
//! Locks are taken in one order: a window's screen, then what is coming to it, then its device's
//! frame lock. A commit takes the frame lock, then what is coming, and never a screen. Which device
//! a window draws with, what went wrong drawing it, what is asked of it next, and how far it has
//! got are locked apart, last, and never across a wait, so whoever drives the frames, which reads
//! them every frame, never waits on a display.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, TryLockError, Weak};
use std::time::{Duration, Instant};

use super::{Extent, Imported, Showing, WindowError};
use crate::gpu::{Gpu, GpuError, Import, PresentMode, Surface};

/// What the presenter draws a window with, shared by the window's handle and its presenter.
pub(super) struct Screen {
    pub(super) surface: Option<Surface>,
    pub(super) showing: Option<Showing>,
    /// The device's commit count when it last drew.
    pub(super) drawn: u64,
    /// Whether it must draw again, whatever the commits say.
    pub(super) stale: bool,
    /// Whether its last try failed, so a failure that lasts is reported once, not every retry.
    pub(super) failing: bool,
}

/// What a commit shows on a window next.
pub(super) enum Incoming {
    Show(Box<Showing>),
    Hide,
}

/// What the window asks of its surface, applied by its presenter before it next draws, with the
/// screen held, so no image is ever outstanding while the surface changes.
#[derive(Default)]
pub(super) struct Asked {
    /// A new size.
    pub(super) size: Option<Extent>,
    /// A new present mode.
    pub(super) present: Option<PresentMode>,
}

/// How far the window's presenter has got, which waiters wait on, and its signal: held by the
/// presenter's thread apart from the window, so the thread holds nothing of the window while it
/// waits, and a window dropped without being closed ends it.
#[derive(Default)]
struct Signal {
    progress: Mutex<Progress>,
    /// Signalled when it is asked to look, when it has looked, when it presented, and when it is
    /// retired.
    changed: Condvar,
}

#[derive(Default)]
struct Progress {
    /// How many times it was asked to look for something to draw.
    asked: u64,
    /// How many times it looked.
    rounds: u64,
    /// Whether its window is gone, so its thread ends.
    closed: bool,
}

/// A window as its presenter sees it: its screen, what a commit shows on it next, the device it
/// draws with, what went wrong drawing it, what it asks of its surface, and how far it has got.
///
/// The screen is held while the presenter waits for the surface's next image; nothing else is
/// held across a wait, and everything the frames' driver reads is outside the screen.
pub(super) struct Shared {
    pub(super) screen: Mutex<Screen>,
    /// The device it draws with: swapped, with the surface, only while the screen is held.
    pub(super) device: Mutex<Arc<Gpu>>,
    /// What went wrong drawing or resizing it, reported at the next present.
    pub(super) problem: Mutex<Option<WindowError>>,
    /// What a commit shows on it, taken when it next draws.
    pub(super) incoming: Mutex<Option<Incoming>>,
    /// What it asks of its surface next.
    pub(super) asked: Mutex<Asked>,
    /// How many frames it presented.
    frames: AtomicU64,
    /// The device's commit count the last frame it presented drew: what a frame's driver waits to
    /// see reach the commit it submitted.
    presented: AtomicU64,
    /// Whether presenting to it will wait for its display: the mode asked for, as the surface's
    /// modes resolve it.
    waits: AtomicBool,
    /// Whether its surface is configured now in a mode that waits for its display: what the
    /// presenter last applied, which a frame's driver paces by.
    applied: AtomicBool,
    signal: Arc<Signal>,
}

impl Shared {
    /// A window's shared part, drawing on `surface` with `gpu`.
    pub(super) fn new(surface: Option<Surface>, gpu: Arc<Gpu>, waits: bool) -> Shared {
        Shared {
            screen: Mutex::new(Screen {
                surface,
                showing: None,
                drawn: 0,
                stale: false,
                failing: false,
            }),
            device: Mutex::new(gpu),
            problem: Mutex::default(),
            incoming: Mutex::default(),
            asked: Mutex::default(),
            frames: AtomicU64::new(0),
            presented: AtomicU64::new(0),
            waits: AtomicBool::new(waits),
            applied: AtomicBool::new(waits),
            signal: Arc::default(),
        }
    }

    pub(super) fn screen(&self) -> MutexGuard<'_, Screen> {
        lock(&self.screen)
    }

    fn incoming(&self) -> MutexGuard<'_, Option<Incoming>> {
        lock(&self.incoming)
    }

    /// The device it draws with.
    pub(super) fn device(&self) -> Arc<Gpu> {
        Arc::clone(&lock(&self.device))
    }

    /// What went wrong drawing or resizing it since this was last asked.
    pub(super) fn problem(&self) -> MutexGuard<'_, Option<WindowError>> {
        lock(&self.problem)
    }

    /// Shows `incoming` when the presenter next draws it: what a commit does.
    pub(super) fn arrive(&self, incoming: Incoming) {
        *self.incoming() = Some(incoming);
    }

    /// Has its presenter apply `size` to the surface and the shown plan before it next draws.
    pub(super) fn ask_size(&self, size: Extent) {
        lock(&self.asked).size = Some(size);
        self.wake();
    }

    /// Has its presenter configure its surface for `present` before it next draws. `waits` is
    /// whether presenting will wait for the display once it has, as the surface's modes say.
    pub(super) fn ask_present(&self, present: PresentMode, waits: bool) {
        lock(&self.asked).present = Some(present);
        self.waits.store(waits, Ordering::Release);
        self.wake();
    }

    /// How many frames it presented.
    pub(super) fn frames(&self) -> u64 {
        self.frames.load(Ordering::Acquire)
    }

    /// Whether presenting to it will wait for its display, once what was asked is applied.
    pub(super) fn waits(&self) -> bool {
        self.waits.load(Ordering::Acquire)
    }

    /// Whether presenting to it waits for its display now, as its surface is configured.
    pub(super) fn applied(&self) -> bool {
        self.applied.load(Ordering::Acquire)
    }

    /// Records the mode its surface is configured in now: what a surface made again, or moved to
    /// another device, was made in.
    pub(super) fn set_applied(&self, waits: bool) {
        self.applied.store(waits, Ordering::Release);
    }

    /// The device's commit count its last presented frame drew.
    pub(super) fn presented(&self) -> u64 {
        self.presented.load(Ordering::Acquire)
    }

    /// Asks its presenter to look for something to draw.
    pub(super) fn wake(&self) {
        lock(&self.signal.progress).asked += 1;
        self.signal.changed.notify_all();
    }

    /// Ends its presenter's thread: the window is closed.
    pub(super) fn retire(&self) {
        lock(&self.signal.progress).closed = true;
        self.signal.changed.notify_all();
    }

    /// Whether it has drawn everything there is: nothing waiting, nothing stale, every commit. A
    /// screen the presenter holds is being drawn, or waiting for an image, so it isn't, and asking
    /// never waits for it.
    pub(super) fn settled(&self) -> bool {
        let screen = match self.screen.try_lock() {
            Ok(screen) => screen,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return false,
        };
        if screen.surface.is_none() {
            return true;
        }
        let asked = {
            let asked = lock(&self.asked);
            asked.size.is_some() || asked.present.is_some()
        };
        let incoming = self.incoming().is_some();
        let Some(_) = &screen.showing else {
            return !incoming && !asked;
        };
        !incoming && !asked && !screen.stale && screen.drawn == self.device().commits()
    }

    /// Waits until it has presented a frame that drew the device's commit `commit` or a later one,
    /// or `deadline` passes: whether it did. Counted by commit, not by frames, so a present that
    /// happened before the waiter looked still counts, and one of an older commit doesn't.
    pub(super) fn wait_presented(&self, commit: u64, deadline: Instant) -> bool {
        let mut progress = lock(&self.signal.progress);
        loop {
            if self.presented() >= commit {
                return true;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            progress = self
                .signal
                .changed
                .wait_timeout(progress, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Waits until it is settled or `deadline` passes: a window whose image never comes ready
    /// (covered, minimized) is not waited on forever.
    fn wait_settled(&self, deadline: Instant) {
        while !self.settled() {
            let progress = lock(&self.signal.progress);
            let seen = progress.rounds;
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return;
            };
            // A round that finishes between the look and the wait is caught a moment later.
            drop(
                self.signal
                    .changed
                    .wait_timeout_while(progress, left.min(RETRY), |progress| progress.rounds == seen)
                    .unwrap_or_else(PoisonError::into_inner),
            );
        }
    }

    /// How many times its presenter has looked for something to draw.
    #[cfg(test)]
    fn rounds(&self) -> u64 {
        lock(&self.signal.progress).rounds
    }
}

/// Every window a presenter serves, to wake them all.
static SERVED: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());

/// How long a presenter waits before trying a window whose image wasn't ready again.
const RETRY: Duration = Duration::from_millis(8);

/// How long it waits before trying a window that failed to draw again: long enough not to spin on
/// a failure that lasts, short enough that a passing one costs a few frames.
const RECOVER: Duration = Duration::from_millis(100);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts a presenter for `shared`, which draws it until it is retired or dropped.
pub(super) fn serve(shared: &Arc<Shared>) {
    {
        let mut served = lock(&SERVED);
        served.retain(|served| served.strong_count() > 0);
        served.push(Arc::downgrade(shared));
    }
    let serving = Arc::downgrade(shared);
    let signal = Arc::clone(&shared.signal);
    std::thread::Builder::new()
        .name("jess presenter".to_owned())
        .spawn(move || run(&serving, &signal))
        .expect("the system starts a thread");
}

/// Asks every window's presenter to look for something to draw.
pub(super) fn wake() {
    let served: Vec<Arc<Shared>> = lock(&SERVED).iter().filter_map(Weak::upgrade).collect();
    for shared in served {
        shared.wake();
    }
}

/// Waits until every one of `shared` has drawn everything there is to draw, or `timeout` passes.
pub(super) fn wait(shared: &[Arc<Shared>], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    for shared in shared {
        shared.wait_settled(deadline);
    }
}

/// What ending a frame needs of `shared`: what went wrong drawing it since the last frame, and
/// the device whose commit it draws. Neither waits on the presenter, whatever it is waiting for.
pub(super) fn ending(shared: &Shared) -> (Option<WindowError>, Arc<Gpu>) {
    (shared.problem().take(), shared.device())
}

/// What one try at drawing a window came to.
enum Drawing {
    /// Nothing new to draw, or nothing to draw it with.
    Nothing,
    /// It drew and presented.
    Drew,
    /// Its image wasn't ready: try again soon.
    NotReady,
    /// Drawing it failed and presented nothing: try again, less soon.
    Failed,
}

/// A window's presenter: draws it whenever asked, and again while a try didn't present.
fn run(serving: &Weak<Shared>, signal: &Signal) {
    let mut seen = 0;
    loop {
        let Some(shared) = serving.upgrade() else {
            return;
        };
        let retry = match draw(&shared) {
            Drawing::NotReady => Some(RETRY),
            Drawing::Failed => Some(RECOVER),
            Drawing::Nothing | Drawing::Drew => None,
        };
        // Nothing of the window is held while it waits.
        drop(shared);
        let mut progress = lock(&signal.progress);
        progress.rounds += 1;
        signal.changed.notify_all();
        while progress.asked == seen && !progress.closed {
            // Without a retry it still looks now and then, so a window dropped without being
            // closed ends its thread.
            let (waited, timeout) = signal
                .changed
                .wait_timeout(progress, retry.unwrap_or(Duration::from_secs(1)))
                .unwrap_or_else(PoisonError::into_inner);
            progress = waited;
            if timeout.timed_out() {
                break;
            }
        }
        if progress.closed {
            return;
        }
        seen = progress.asked;
    }
}

/// Applies what the window asked of its surface: a size, and a present mode.
fn apply(shared: &Shared, screen: &mut Screen, gpu: &Gpu) {
    let asked = std::mem::take(&mut *lock(&shared.asked));
    if asked.size.is_none() && asked.present.is_none() {
        return;
    }
    screen.stale = true;
    let mut problems = Vec::new();
    if let Some(surface) = &mut screen.surface {
        if let Some(present) = asked.present {
            match surface.set_present(gpu, present) {
                Ok(()) => shared.set_applied(surface.vsync()),
                Err(problem) => problems.push(problem),
            }
        }
        if let Some(size) = asked.size
            && let Err(problem) = surface.resize(gpu, size.width, size.height)
        {
            problems.push(problem);
        }
    }
    if let (Some(size), Some(showing)) = (asked.size, &screen.showing) {
        match showing.plan.get().resized(gpu, size) {
            Ok(plan) => showing.plan.set(plan),
            Err(problem) => problems.push(problem),
        }
    }
    if let Some(problem) = problems.pop() {
        *shared.problem() = Some(problem.into());
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
    let gpu = shared.device();
    apply(shared, &mut screen, &gpu);
    let commits = gpu.commits();
    if !screen.stale && screen.drawn == commits {
        return Drawing::Nothing;
    }
    let Screen {
        surface,
        showing,
        stale,
        drawn,
        failing,
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
        Err(failure) => return fail(&mut shared.problem(), failing, stale, failure),
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
        // The image goes back to the surface unpresented: a frame that failed isn't one.
        drop(frame);
        return fail(&mut shared.problem(), failing, stale, failure);
    }
    gpu.present(frame);
    *drawn = commits;
    *stale = false;
    *failing = false;
    // Counted under the progress lock, so a waiter that looked and found none can't miss it.
    {
        let _progress = lock(&shared.signal.progress);
        shared.frames.fetch_add(1, Ordering::AcqRel);
        shared.presented.fetch_max(commits, Ordering::AcqRel);
    }
    shared.signal.changed.notify_all();
    Drawing::Drew
}

/// Records that drawing a window failed: it stays stale, so it is tried again, and the failure is
/// reported unless the last try failed too.
fn fail(problem: &mut Option<WindowError>, failing: &mut bool, stale: &mut bool, failure: GpuError) -> Drawing {
    if !*failing {
        *problem = Some(failure.into());
    }
    *failing = true;
    *stale = true;
    Drawing::Failed
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc::channel;
    use std::time::{Duration, Instant};

    use super::{Drawing, Gpu, GpuError, Shared, WindowError, ending, fail, serve};

    fn gpu(test: &str) -> Option<Arc<Gpu>> {
        match Gpu::headless() {
            Ok(gpu) => Some(Arc::new(gpu)),
            Err(GpuError::NoAdapter) => {
                eprintln!("SKIPPED {test}: no adapter");
                None
            }
            Err(problem) => panic!("{problem}"),
        }
    }

    /// Generous: only a failing run waits this long.
    const LONG: Duration = Duration::from_secs(30);

    /// A frame ends while the presenter holds a window's screen, as it does all the while it
    /// waits for the display to give it an image: the frame's end reads what it needs past it,
    /// and display pacing sees a window still being drawn without waiting either. The presenter
    /// here lets go only once the frame has ended, so a frame that waited on it never ends.
    #[test]
    fn a_frame_ends_without_waiting_on_a_window_the_presenter_holds() {
        let Some(gpu) = gpu("a_frame_ends_without_waiting_on_a_window_the_presenter_holds") else {
            return;
        };
        let shared = Arc::new(Shared::new(None, Arc::clone(&gpu), true));
        *shared.problem() = Some(WindowError::System("lost".to_owned()));
        shared.screen().stale = true;
        let (held, holding) = channel();
        let (ended, waiting) = channel::<()>();
        let presenter = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                let _screen = shared.screen();
                held.send(()).unwrap();
                waiting.recv().unwrap();
            })
        };
        holding.recv().unwrap();

        let (problem, device) = ending(&shared);
        assert!(problem.is_some_and(|problem| problem.to_string() == "lost"));
        assert!(Arc::ptr_eq(&device, &gpu));
        assert!(!shared.settled(), "a window the presenter holds is still being drawn");
        assert_eq!(shared.frames(), 0, "how far it got is read past the screen");
        assert!(shared.waits(), "and so is whether it waits on its display");

        ended.send(()).unwrap();
        presenter.join().unwrap();
        assert!(shared.settled(), "a window with no surface has nothing to draw");
    }

    /// A window whose presenter is stuck waiting for an image (covered, minimized: its screen
    /// held) holds no other window up: the other's presenter keeps looking, and waiting for the
    /// other to settle doesn't wait on the stuck one. The stuck one is let go only once the other
    /// has been served, so a presenter that served them in turn never gets there.
    #[test]
    fn a_window_that_cannot_draw_holds_no_other_up() {
        let Some(gpu) = gpu("a_window_that_cannot_draw_holds_no_other_up") else {
            return;
        };
        let stuck = Arc::new(Shared::new(None, Arc::clone(&gpu), true));
        let primary = Arc::new(Shared::new(None, Arc::clone(&gpu), true));
        let (held, holding) = channel();
        let (release, released) = channel::<()>();
        let holder = {
            let stuck = Arc::clone(&stuck);
            std::thread::spawn(move || {
                let _screen = stuck.screen();
                held.send(()).unwrap();
                released.recv().unwrap();
            })
        };
        holding.recv().unwrap();
        serve(&stuck);
        serve(&primary);
        let before = primary.rounds();
        super::wake();
        let deadline = Instant::now() + LONG;
        while primary.rounds() <= before {
            assert!(Instant::now() < deadline, "the primary window's presenter never looked");
            std::thread::yield_now();
        }
        super::wait(std::slice::from_ref(&primary), LONG);
        assert!(primary.settled());
        assert!(!stuck.settled(), "the stuck window is still held");

        release.send(()).unwrap();
        holder.join().unwrap();
        stuck.retire();
        primary.retire();
    }

    /// Waiting for a window to present returns once it has presented past what the waiter saw,
    /// and at the deadline when it never does: a frame's driver never waits longer than it said.
    #[test]
    fn waiting_for_a_present_ends_at_the_deadline() {
        let Some(gpu) = gpu("waiting_for_a_present_ends_at_the_deadline") else {
            return;
        };
        let shared = Shared::new(None, gpu, true);
        assert!(!shared.wait_presented(1, Instant::now()), "nothing of commit 1 was presented");
        shared.presented.store(2, std::sync::atomic::Ordering::Release);
        assert!(
            shared.wait_presented(1, Instant::now()),
            "a later commit's frame counts, however early"
        );
        assert!(!shared.wait_presented(3, Instant::now()), "an older commit's frame doesn't");
    }

    /// A presenter holds nothing of its window while it waits, so a window dropped without being
    /// closed goes at once, and its thread with it.
    #[test]
    fn a_waiting_presenter_holds_nothing_of_its_window() {
        let Some(gpu) = gpu("a_waiting_presenter_holds_nothing_of_its_window") else {
            return;
        };
        let shared = Arc::new(Shared::new(None, gpu, true));
        serve(&shared);
        let deadline = Instant::now() + LONG;
        while shared.rounds() == 0 {
            assert!(Instant::now() < deadline, "the presenter never looked");
            std::thread::yield_now();
        }
        let weak = Arc::downgrade(&shared);
        drop(shared);
        assert!(weak.upgrade().is_none(), "the waiting presenter kept the window alive");
    }

    #[test]
    fn a_failure_that_lasts_is_reported_once_and_tried_again() {
        let (mut problem, mut failing, mut stale) = (None, false, false);
        let first = fail(&mut problem, &mut failing, &mut stale, GpuError::Device("lost".to_owned()));
        assert!(matches!(first, Drawing::Failed));
        assert!(stale, "a window that failed must be drawn again");
        assert!(problem.take().is_some_and(|problem| problem.to_string().contains("lost")));

        let again = fail(&mut problem, &mut failing, &mut stale, GpuError::Device("lost".to_owned()));
        assert!(matches!(again, Drawing::Failed));
        assert!(stale);
        assert!(problem.is_none(), "the same failure is not reported every retry");

        failing = false;
        fail(
            &mut problem,
            &mut failing,
            &mut stale,
            GpuError::Device("lost again".to_owned()),
        );
        assert!(problem.is_some(), "a failure after a drawn frame is reported");
    }
}
