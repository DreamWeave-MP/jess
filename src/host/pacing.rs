//! Frame pacing: when an application's next frame begins, decided apart from how its frames are
//! simulated (the domain's [`FixedStep`](crate::runtime::FixedStep)) and from how its windows
//! present (each window's present mode).
//!
//! Three paces:
//!
//! - **Display**: frames follow the pacing window's display. A frame that gave the pacing window
//!   something to draw is followed once that window has presented it: a window that waits for
//!   its display (vsync) presents at most once a refresh, so that wait is the pace. Never sooner
//!   than half a refresh after the last frame began, so a driver whose presenting doesn't wait
//!   can't make a spin of it, and never later than two, so a window that can't draw (minimized,
//!   covered) doesn't stop the application. A frame that gave it nothing to draw is followed a
//!   refresh after it began. With no pacing window, or one that doesn't wait for its display, the
//!   pace is a timer at the display's refresh (or the fallback a driver gives, its fixed step):
//!   what [`resolve`] says is in effect.
//! - **Capped**: at most that many frames a second, on a timer. Frames keep to the schedule, so
//!   they average the rate; one that runs late is followed at once and the schedule resumes from
//!   it, so no burst catches up.
//! - **Unpaced**: as fast as frames finish.
//!
//! Who decides: the application's manifest gives a default (`pace`), the application's code may
//! ask for another at any time (`@jess/application`'s `setPace`), and whoever runs it (`--pace`,
//! or an embedder) may override both. [`Pacing`] holds all three, as a service the application's
//! domain carries; the override is authoritative, and what the code asked for is kept and reported,
//! not lost, while it lasts.
//!
//! The [`Scheduler`] is the decision itself, over instants it is given: it never reads a clock or
//! waits. A driver that blocks (the command line's) waits for what it says; one called back by
//! the platform once a frame (a browser's animation frame, an Android choreographer) asks it the
//! same questions and schedules its callback instead. Changes apply at a frame's end, the only
//! boundary there is: the next frame's deadline is rebased on when the last one began, and a
//! frame's delta is always the time since the last began, so no elapsed time is lost or invented
//! and the fixed step's accumulation carries over any change of pace.

use std::fmt;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The most frames a second a capped pace asks for.
pub const MAX_RATE: u32 = 1000;

/// How often an application's frames run. See the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pace {
    /// As the pacing window's display takes frames.
    Display,
    /// At most this many frames a second, from 1 to [`MAX_RATE`].
    Capped(u32),
    /// As fast as frames finish.
    Unpaced,
}

impl Pace {
    /// A capped pace of `rate` frames a second, if `rate` is from 1 to [`MAX_RATE`].
    #[must_use]
    pub fn capped(rate: u32) -> Option<Pace> {
        (1..=MAX_RATE).contains(&rate).then_some(Pace::Capped(rate))
    }

    /// `display`, `unpaced`, or a whole number of frames a second from 1 to [`MAX_RATE`].
    #[must_use]
    pub fn parse(text: &str) -> Option<Pace> {
        match text {
            "display" => Some(Pace::Display),
            "unpaced" => Some(Pace::Unpaced),
            _ => text.parse::<u32>().ok().and_then(Pace::capped),
        }
    }
}

impl fmt::Display for Pace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Pace::Display => f.write_str("display"),
            Pace::Capped(rate) => write!(f, "{rate}"),
            Pace::Unpaced => f.write_str("unpaced"),
        }
    }
}

/// Where the pace an application asks for came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Nobody said: the host's default for the kind of application it is.
    Default,
    /// Its manifest's `pace`.
    Manifest,
    /// Its code, at run time.
    Application,
}

/// What the presentation side says at a frame's end, for display pacing: all of it about the
/// pacing window, none about any other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Presentation {
    /// How long a refresh of the pacing window's display is, where the window system says.
    pub refresh: Option<Duration>,
    /// Whether that is the refresh of the display the pacing window is on, not a guess from the
    /// displays there are (a window not yet mapped is on none).
    pub refresh_confirmed: bool,
    /// Whether there is a pacing window and its presenting waits for its display.
    pub waits: bool,
    /// Whether the pacing window has something the frame just ended gave it to draw.
    pub pending: bool,
}

/// The pace in effect at a frame's end: what was asked for, as what there is to pace by allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effective {
    /// By the pacing window's presenting, a refresh of this long.
    Display(Duration),
    /// On a timer of this interval: a capped pace, or display pacing with nothing to pace by.
    Timer(Duration),
    /// Not at all.
    Unpaced,
}

impl Effective {
    /// As a [`Pace`]: a timer is the rate it runs at, rounded to whole frames a second.
    #[must_use]
    pub fn pace(self) -> Pace {
        match self {
            Effective::Display(_) => Pace::Display,
            Effective::Timer(interval) => {
                let rate = (1.0 / interval.as_secs_f64()).round();
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "clamped to 1..=MAX_RATE first"
                )]
                Pace::Capped(rate.clamp(1.0, f64::from(MAX_RATE)) as u32)
            }
            Effective::Unpaced => Pace::Unpaced,
        }
    }
}

/// The pace in effect when `pace` is asked for and the pacing window is as `presentation` says.
///
/// Display pacing without a window that waits for its display is a timer at the display's
/// refresh, or at `fallback` where none says how fast it is.
#[must_use]
pub fn resolve(pace: Pace, presentation: &Presentation, fallback: Duration) -> Effective {
    match pace {
        Pace::Unpaced => Effective::Unpaced,
        Pace::Capped(rate) => Effective::Timer(Duration::from_secs(1) / rate.max(1)),
        Pace::Display if presentation.waits => Effective::Display(presentation.refresh.unwrap_or(fallback)),
        Pace::Display => Effective::Timer(presentation.refresh.unwrap_or(fallback)),
    }
}

/// What a driver waits for before the next frame begins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// Nothing: it begins now.
    Now,
    /// Until this instant, which may have passed.
    Until(Instant),
    /// Until the pacing window has presented what it has to draw, but at least until
    /// `not_before` and at most until `give_up`.
    Presented {
        /// The soonest the next frame begins, whatever the window does.
        not_before: Instant,
        /// The latest it begins, whatever the window does.
        give_up: Instant,
    },
}

/// When frames begin: see the module docs. It reads no clock; every instant is the driver's.
#[derive(Clone, Copy, Debug)]
pub struct Scheduler {
    /// When the last frame began, or the scheduler was made.
    last: Instant,
    /// The timer's next deadline and its interval, while a timer paces.
    timer: Option<(Instant, Duration)>,
}

impl Scheduler {
    /// A scheduler whose first frame's delta is counted from `now`.
    #[must_use]
    pub fn new(now: Instant) -> Scheduler {
        Scheduler { last: now, timer: None }
    }

    /// When the last frame began.
    #[must_use]
    pub fn last(&self) -> Instant {
        self.last
    }

    /// A frame begins at `now`: its delta, the time since the last began.
    pub fn begin(&mut self, now: Instant) -> Duration {
        let delta = now.saturating_duration_since(self.last);
        self.last = now;
        delta
    }

    /// The last frame's work is done at `now` and `effective` is the pace: what to wait for before
    /// the next begins.
    pub fn wait(&mut self, now: Instant, effective: Effective, presentation: &Presentation) -> Wait {
        match effective {
            Effective::Unpaced => {
                self.timer = None;
                Wait::Now
            }
            Effective::Display(refresh) => {
                self.timer = None;
                if presentation.pending {
                    Wait::Presented {
                        not_before: self.last + refresh / 2,
                        give_up: self.last + refresh * 2,
                    }
                } else {
                    Wait::Until(self.last + refresh)
                }
            }
            Effective::Timer(interval) => {
                // The same interval keeps to its schedule; a new one, or a timer taking over from
                // another pace, starts from when the last frame began.
                let next = match self.timer {
                    Some((deadline, running)) if running == interval => deadline + interval,
                    _ => self.last + interval,
                };
                // Behind: the next frame begins now, and the schedule resumes from it.
                let next = next.max(now);
                self.timer = Some((next, interval));
                Wait::Until(next)
            }
        }
    }
}

/// A clock a driver waits on: the real one, or a test's.
pub trait Clock {
    /// Now.
    fn now(&mut self) -> Instant;
    /// Sleeps until `at`, or a little after, as sleeps do.
    fn sleep_until(&mut self, at: Instant);
}

/// The system's clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&mut self) -> Instant {
        Instant::now()
    }

    fn sleep_until(&mut self, at: Instant) {
        if let Some(left) = at.checked_duration_since(Instant::now()) {
            std::thread::sleep(left);
        }
    }
}

/// Waits as `wait` says on `clock`, and says how long it waited.
///
/// `presented` waits for the pacing window to present what the frame submitted, until the instant
/// it is given, and says whether it did. Whatever the window does, a display wait lasts at least
/// until its `not_before`.
pub fn wait(wait: Wait, clock: &mut impl Clock, presented: impl FnOnce(Instant) -> bool) -> Duration {
    let started = clock.now();
    match wait {
        Wait::Now => {}
        Wait::Until(at) => clock.sleep_until(at),
        Wait::Presented { not_before, give_up } => {
            let _presented = presented(give_up);
            clock.sleep_until(not_before);
        }
    }
    clock.now().saturating_duration_since(started)
}

/// What a frame took and how the frames are going: what [`Pacing::stats`] reports, without
/// waiting on anything.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    /// Frames begun.
    pub frames: u64,
    /// The last frame's delta: the time since the one before it began.
    pub delta: Duration,
    /// How long the last frame's work took, from its beginning to its windows told to draw it.
    pub work: Duration,
    /// How long the driver waited before the last frame, for the pace.
    pub waited: Duration,
    /// Frames a second over the last second or so; 0 before there is one to measure.
    pub rate: f64,
    /// The pacing window's display's refresh at the last frame's end, where the window system
    /// says, and whether it is that display's or a guess.
    pub refresh: Option<(Duration, bool)>,
    /// The last frame's collection ([`crate::host::Host::collect`]), a part of its `work`.
    pub collection: crate::host::Collection,
    /// Every frame's collections together.
    pub collections: Collections,
}

/// Every frame's collections together, as [`FrameStats::collections`] counts them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Collections {
    /// Cycles they finished.
    pub cycles: u64,
    /// Collections that took longer than their budget.
    pub overruns: u64,
    /// The time they took.
    pub spent: Duration,
}

/// How long [`FrameStats::rate`] measures over.
const RATE_WINDOW: Duration = Duration::from_secs(1);

#[derive(Debug)]
struct PacingState {
    /// What it asks for when its code hasn't asked, and where that came from.
    default: (Pace, Origin),
    fixed_step: Duration,
    requested: Pace,
    origin: Origin,
    overridden: Option<Pace>,
    effective: Pace,
    stats: FrameStats,
    /// When the frames the rate counts began counting, and how many there were since.
    counting: Option<(Instant, u64)>,
}

/// An application's pace: what it asks for, what overrides that, what is in effect, and how its
/// frames are going. A service its domain carries; see the module docs.
#[derive(Debug)]
pub struct Pacing(Mutex<PacingState>);

impl Pacing {
    /// Pacing that asks for `requested`, from `origin`, with no override, of an application whose
    /// fixed step is `fixed_step`.
    #[must_use]
    pub fn new(requested: Pace, origin: Origin, fixed_step: Duration) -> Pacing {
        Pacing(Mutex::new(PacingState {
            default: (requested, origin),
            fixed_step,
            requested,
            origin,
            overridden: None,
            effective: requested,
            stats: FrameStats::default(),
            counting: None,
        }))
    }

    fn state(&self) -> MutexGuard<'_, PacingState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks for `pace`, as the application's code does: in effect from the end of the frame it
    /// is asked in, unless an override holds.
    pub fn request(&self, pace: Pace) {
        let mut state = self.state();
        state.requested = pace;
        state.origin = Origin::Application;
    }

    /// Forgets what the code asked for, as a restart does: the new generation asks again if it
    /// wants another pace. The override, which isn't the code's, stays.
    pub fn reset(&self) {
        let mut state = self.state();
        (state.requested, state.origin) = state.default;
    }

    /// The application's fixed step.
    #[must_use]
    pub fn fixed_step(&self) -> Duration {
        self.state().fixed_step
    }

    /// What is asked for, and where that came from.
    #[must_use]
    pub fn requested(&self) -> (Pace, Origin) {
        let state = self.state();
        (state.requested, state.origin)
    }

    /// Overrides whatever is asked for with `pace`, as whoever runs the application does; `None`
    /// lifts the override, and what was asked for meanwhile is what paces.
    pub fn set_override(&self, pace: Option<Pace>) {
        self.state().overridden = pace;
    }

    /// The override, if one holds.
    #[must_use]
    pub fn overridden(&self) -> Option<Pace> {
        self.state().overridden
    }

    /// The pace to resolve: the override, or what is asked for.
    #[must_use]
    pub fn chosen(&self) -> Pace {
        let state = self.state();
        state.overridden.unwrap_or(state.requested)
    }

    /// The pace in effect at the last frame's end.
    #[must_use]
    pub fn effective(&self) -> Pace {
        self.state().effective
    }

    /// How the frames are going.
    #[must_use]
    pub fn stats(&self) -> FrameStats {
        self.state().stats
    }

    /// A frame began at `began` after `waited` for the pace, with `delta` since the last one.
    pub fn began(&self, began: Instant, delta: Duration, waited: Duration) {
        let mut state = self.state();
        state.stats.frames += 1;
        state.stats.delta = delta;
        state.stats.waited = waited;
        match state.counting {
            Some((since, counted)) if began.saturating_duration_since(since) >= RATE_WINDOW => {
                #[expect(clippy::cast_precision_loss, reason = "a second's frames are few")]
                let rate = counted as f64 / began.saturating_duration_since(since).as_secs_f64();
                state.stats.rate = rate;
                state.counting = Some((began, 1));
            }
            Some((since, counted)) => state.counting = Some((since, counted + 1)),
            None => state.counting = Some((began, 1)),
        }
    }

    /// The frame's collection was `collection`.
    pub fn collected(&self, collection: crate::host::Collection) {
        let mut state = self.state();
        state.stats.collection = collection;
        state.stats.collections.cycles += u64::from(collection.finished);
        state.stats.collections.overruns += u64::from(collection.overran());
        state.stats.collections.spent += collection.spent;
    }

    /// The frame's work took `work`, `effective` was the pace at its end, and `presentation` what
    /// the pacing window said then.
    pub fn ended(&self, work: Duration, effective: Pace, presentation: &Presentation) {
        let mut state = self.state();
        state.stats.work = work;
        state.stats.refresh = presentation.refresh.map(|refresh| (refresh, presentation.refresh_confirmed));
        state.effective = effective;
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Clock, Effective, Pace, Presentation, Scheduler, Wait, resolve, wait};

    const MS: Duration = Duration::from_millis(1);

    fn deadline(wait: Wait) -> Instant {
        match wait {
            Wait::Until(at) => at,
            other => panic!("a timer's wait, not {other:?}"),
        }
    }

    /// A capped pace keeps to its schedule: a frame that finishes early waits for its slot, and
    /// the slots stay a fixed interval apart however long each frame worked.
    #[test]
    fn a_capped_pace_keeps_to_its_schedule() {
        let origin = Instant::now();
        let mut scheduler = Scheduler::new(origin);
        let timer = Effective::Timer(MS * 10);
        let mut now = origin;
        for frame in 1..=5u32 {
            let next = deadline(scheduler.wait(now + MS * 3, timer, &Presentation::default()));
            assert_eq!(next, origin + MS * 10 * frame);
            now = next;
            assert_eq!(scheduler.begin(now), MS * 10);
        }
    }

    /// A frame that runs past its slot is followed at once, and the schedule resumes from it:
    /// no burst of frames makes up the slots it missed.
    #[test]
    fn a_late_frame_does_not_start_a_burst() {
        let origin = Instant::now();
        let mut scheduler = Scheduler::new(origin);
        let timer = Effective::Timer(MS * 10);
        // The first frame's work takes 35 ms: three slots missed.
        let late = origin + MS * 35;
        assert_eq!(deadline(scheduler.wait(late, timer, &Presentation::default())), late);
        assert_eq!(scheduler.begin(late), MS * 35);
        let next = deadline(scheduler.wait(late + MS, timer, &Presentation::default()));
        assert_eq!(next, late + MS * 10);
    }

    /// A change of pace applies at the frame's end and rebases on when the last frame began; every
    /// delta is the time since the last began, so the deltas add up to the time that passed and
    /// the fixed step's accumulation loses nothing to the change.
    #[test]
    fn a_change_of_pace_rebases_and_loses_no_time() {
        let origin = Instant::now();
        let mut scheduler = Scheduler::new(origin);
        let mut now = origin;
        let mut total = Duration::ZERO;
        let presentation = Presentation::default();
        let paces = [
            Effective::Timer(MS * 10),
            Effective::Timer(MS * 10),
            Effective::Unpaced,
            Effective::Timer(MS * 4),
            Effective::Timer(MS * 25),
            Effective::Unpaced,
            Effective::Timer(MS * 10),
        ];
        for pace in paces {
            let worked = now + MS * 2;
            let began = scheduler.last();
            let wait = scheduler.wait(worked, pace, &presentation);
            now = match (pace, wait) {
                (Effective::Unpaced, Wait::Now) => worked,
                (Effective::Timer(interval), Wait::Until(at)) => {
                    // A timer's first slot after a change is an interval after the last began.
                    assert!(at <= began + interval && at >= worked.min(began + interval), "{pace:?}");
                    at.max(worked)
                }
                other => panic!("{other:?}"),
            };
            total += scheduler.begin(now);
        }
        assert_eq!(total, now - origin);
    }

    /// Display pacing waits for the pacing window to present a frame it was given, between half a
    /// refresh and two; a frame that gave it nothing waits a refresh.
    #[test]
    fn display_pacing_waits_on_the_pacing_window_alone() {
        let origin = Instant::now();
        let mut scheduler = Scheduler::new(origin);
        let refresh = MS * 16;
        let given = Presentation {
            refresh: Some(refresh),
            refresh_confirmed: true,
            waits: true,
            pending: true,
        };
        assert_eq!(
            scheduler.wait(origin + MS, Effective::Display(refresh), &given),
            Wait::Presented {
                not_before: origin + MS * 8,
                give_up: origin + MS * 32,
            }
        );
        let idle = Presentation { pending: false, ..given };
        assert_eq!(
            scheduler.wait(origin + MS, Effective::Display(refresh), &idle),
            Wait::Until(origin + refresh)
        );
    }

    /// Display pacing with nothing to pace by is a timer at the display's refresh, or the
    /// fallback; a cap and no pace resolve to themselves, and the effective pace says which.
    #[test]
    fn display_pacing_without_a_waiting_window_is_a_timer() {
        let fallback = MS * 20;
        let unknown = Presentation::default();
        assert_eq!(resolve(Pace::Display, &unknown, fallback), Effective::Timer(fallback));
        assert_eq!(resolve(Pace::Display, &unknown, fallback).pace(), Pace::Capped(50));
        let immediate = Presentation {
            refresh: Some(Duration::from_secs(1) / 144),
            refresh_confirmed: true,
            waits: false,
            pending: true,
        };
        assert_eq!(resolve(Pace::Display, &immediate, fallback).pace(), Pace::Capped(144));
        let vsync = Presentation {
            waits: true,
            ..immediate
        };
        assert_eq!(resolve(Pace::Display, &vsync, fallback).pace(), Pace::Display);
        assert_eq!(resolve(Pace::Capped(30), &vsync, fallback).pace(), Pace::Capped(30));
        assert_eq!(resolve(Pace::Unpaced, &vsync, fallback), Effective::Unpaced);
    }

    /// A clock that moves only when slept on, and oversleeps by `late` every time, as a busy
    /// machine's sleeps do.
    struct Late {
        now: Instant,
        late: Duration,
    }

    impl Clock for Late {
        fn now(&mut self) -> Instant {
            self.now
        }

        fn sleep_until(&mut self, at: Instant) {
            if at > self.now {
                self.now = at + self.late;
            }
        }
    }

    /// Sleeps that overshoot don't drift a cap: each deadline is the last one plus the interval,
    /// not the late wake plus the interval, so a hundred frames take a hundred intervals and one
    /// overshoot, and no frame begins before its slot.
    #[test]
    fn oversleeping_does_not_drift_a_cap() {
        let origin = Instant::now();
        let mut clock = Late {
            now: origin,
            late: MS * 3,
        };
        let mut scheduler = Scheduler::new(origin);
        let timer = Effective::Timer(MS * 10);
        for frame in 1..=100u32 {
            let worked = clock.now + MS;
            clock.now = worked;
            let until = scheduler.wait(worked, timer, &Presentation::default());
            wait(until, &mut clock, |_| unreachable!("a timer waits on no window"));
            assert!(clock.now >= origin + MS * 10 * frame, "frame {frame} began before its slot");
            scheduler.begin(clock.now);
        }
        assert_eq!(clock.now, origin + MS * 1000 + MS * 3, "a hundred slots and one overshoot");
    }

    /// An interval shorter than a frame's work is behind every frame: each begins as soon as the
    /// last ends, with no wait and no burst, and the deltas are the work.
    #[test]
    fn an_interval_shorter_than_the_work_runs_back_to_back() {
        let origin = Instant::now();
        let mut scheduler = Scheduler::new(origin);
        let timer = Effective::Timer(MS);
        let mut now = origin;
        for _ in 0..50 {
            now += MS * 4;
            assert_eq!(scheduler.wait(now, timer, &Presentation::default()), Wait::Until(now));
            assert_eq!(scheduler.begin(now), MS * 4);
        }
    }

    /// Display pacing waits for the window until it gives up, and then at least until half a
    /// refresh has passed: a present that came at once, from a driver whose presenting doesn't
    /// wait, still waits out the half refresh, and one that never comes waits only to the give-up.
    #[test]
    fn a_display_wait_is_floored_at_half_a_refresh_and_capped_at_two() {
        let origin = Instant::now();
        let refresh = MS * 16;
        let pending = Presentation {
            refresh: Some(refresh),
            refresh_confirmed: true,
            waits: true,
            pending: true,
        };
        let mut scheduler = Scheduler::new(origin);
        let at_once = scheduler.wait(origin + MS, Effective::Display(refresh), &pending);
        let mut clock = Late {
            now: origin + MS,
            late: Duration::ZERO,
        };
        let mut asked = None;
        let waited = wait(at_once, &mut clock, |give_up| {
            asked = Some(give_up);
            true
        });
        assert_eq!(asked, Some(origin + MS * 32), "it waits on the window at most two refreshes");
        assert_eq!(clock.now, origin + MS * 8, "and at least half of one");
        assert_eq!(waited, MS * 7);
    }

    /// Paces can change in any order, frame after frame, and every frame's wait is decided by the
    /// pace in effect at its end: display to timer to display again, through no pace at all.
    #[test]
    fn transitions_between_every_pace_are_decided_at_the_frame_s_end() {
        let origin = Instant::now();
        let mut scheduler = Scheduler::new(origin);
        let refresh = MS * 8;
        let display = Presentation {
            refresh: Some(refresh),
            refresh_confirmed: true,
            waits: true,
            pending: true,
        };
        let mut now = origin;
        for (effective, expected) in [
            (Effective::Display(refresh), "presented"),
            (Effective::Timer(MS * 10), "until"),
            (Effective::Display(refresh), "presented"),
            (Effective::Unpaced, "now"),
            (Effective::Timer(MS * 10), "until"),
            (Effective::Timer(MS * 10), "until"),
            (Effective::Display(refresh), "presented"),
        ] {
            let began = scheduler.last();
            let decided = scheduler.wait(now + MS, effective, &display);
            // The driver honours each wait: the next frame begins when it says, or after the work.
            let (kind, next) = match decided {
                Wait::Presented { not_before, .. } => {
                    assert_eq!(not_before, began + refresh / 2);
                    ("presented", not_before.max(now + MS))
                }
                Wait::Until(at) => {
                    assert!(at <= began + MS * 10, "a timer is never later than its interval");
                    ("until", at.max(now + MS))
                }
                Wait::Now => ("now", now + MS),
            };
            assert_eq!(kind, expected, "{effective:?}");
            now = next;
            scheduler.begin(now);
        }
    }

    #[test]
    fn paces_parse_and_print_as_the_command_line_takes_them() {
        for (text, pace) in [
            ("display", Some(Pace::Display)),
            ("unpaced", Some(Pace::Unpaced)),
            ("1", Some(Pace::Capped(1))),
            ("1000", Some(Pace::Capped(1000))),
            ("0", None),
            ("1001", None),
            ("60.5", None),
            ("fast", None),
            ("", None),
        ] {
            assert_eq!(Pace::parse(text), pace, "{text:?}");
            if let Some(pace) = pace {
                assert_eq!(pace.to_string(), text);
            }
        }
    }
}
