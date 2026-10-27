//! Windows: native windows through winit, each with a GPU surface, and their keyboard and pointer
//! as input.
//!
//! A window is what a person sees an application in, and nothing more yet: what a window means to
//! an application (which ones it has, how they are managed) is a later decision and Luau's. The
//! kernel opens one, sizes its surface with it, shows a plan in it every frame, and reports what
//! the person does in it.
//!
//! # The event loop
//!
//! winit allows one event loop per process, on the thread that made it. Jess makes it when the
//! first window opens and pumps it, never blocking, from whoever drives the frames: an
//! application's host does it before every frame ([`pump`]) and draws every window after it
//! ([`present`]); a script with no host calls `jess.gpu.pump()`. The loop never runs a script and
//! never waits on one, and the host stays the one loop of the application, windowed or not.
//!
//! # Showing a plan
//!
//! A window shows a [`Plan`] whose graph imports the window's image as one of its targets: every
//! [`present`] takes the surface's next image, runs the plan into it with the draw sets as they
//! are, and presents it. Nothing on that path enters Luau, so a static scene draws frame after
//! frame without a single call into a script. A resized window resizes the plan with it.
//!
//! # Input
//!
//! Key presses, pointer buttons, motion, and the wheel in any window reach the host's input
//! service through [`WindowInput`], a [`Backend`] that stamps them with the host's clock: one
//! keyboard and one pointer, whichever window they happened in. Keys are named by position, as
//! every jess backend names them.

#![warn(missing_docs)]

use std::cell::RefCell;
use std::fmt;
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{
    DeviceEvent, DeviceId as RawDeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent as RawEvent,
};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::{Window as RawWindow, WindowAttributes, WindowId};

use crate::gpu::{Buffer, DrawSets, Gpu, GpuError, Import, Plan, PresentMode, Surface, Texture};
use crate::input::{
    Backend, BackendError, Control, DeviceDescriptor, DeviceId, DeviceIdentity, EventSender, Key, PointerAxis,
    PointerButton, Timestamp,
};
use crate::render::{Extent, ResourceOrigin, TargetDesc, TextureFormat};

/// Why a window couldn't open or draw.
#[derive(Debug)]
pub enum WindowError {
    /// The machine has no window system jess can reach, or the event loop belongs to another
    /// thread.
    System(String),
    /// The GPU refused the window's surface, or a frame of it.
    Gpu(GpuError),
    /// A plan that can't be shown in the window.
    Invalid(String),
}

impl fmt::Display for WindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WindowError::System(message) | WindowError::Invalid(message) => formatter.write_str(message),
            WindowError::Gpu(problem) => problem.fmt(formatter),
        }
    }
}

impl std::error::Error for WindowError {}

impl From<GpuError> for WindowError {
    fn from(problem: GpuError) -> Self {
        WindowError::Gpu(problem)
    }
}

/// What a window opens as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowDesc {
    /// Its title.
    pub title: String,
    /// Its inner width, in physical pixels.
    pub width: u32,
    /// Its inner height, in physical pixels.
    pub height: u32,
    /// Whether the person can resize it.
    pub resizable: bool,
    /// When a presented image reaches the screen.
    pub present: PresentMode,
    /// The format its surface draws in, or `None` for the first plain 8-bit one it offers.
    pub format: Option<TextureFormat>,
    /// Whether the person asking it to close closes it, or only says so in its events, for the
    /// application to decide.
    pub closes: bool,
}

/// What happened to a window, in the order it happened, for whoever drives it to read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WindowEvent {
    /// Its inner size changed, to this many physical pixels; the plan it shows resized with it.
    Resized(Extent),
    /// The display it is on has this many physical pixels to a logical one now.
    ScaleChanged(f64),
    /// It gained or lost the keyboard.
    Focused(bool),
    /// The person asked it to close: with [`WindowDesc::closes`], it did.
    CloseRequested,
    /// The window system closed it, or a close request did.
    Closed,
}

/// The most events a window keeps for whoever hasn't read them: the oldest go first.
const EVENTS: usize = 256;

impl Default for WindowDesc {
    fn default() -> Self {
        WindowDesc {
            title: "jess".to_owned(),
            width: 800,
            height: 600,
            resizable: true,
            present: PresentMode::Vsync,
            format: None,
            closes: true,
        }
    }
}

/// A resource a shown plan imports besides the window's image, held while it is shown.
#[derive(Clone, Debug)]
pub enum Imported {
    /// A texture, for an imported target.
    Texture(String, Arc<Texture>),
    /// A buffer, for an imported buffer.
    Buffer(String, Arc<Buffer>),
}

/// What a window draws every frame: a plan, the draw sets its draw passes choose from, the
/// target its image is, and the rest of what it imports.
#[derive(Debug)]
pub struct Showing {
    /// The plan, resized with the window.
    pub plan: Plan,
    /// The draw sets.
    pub sets: Arc<DrawSets>,
    /// The imported target the window's image is.
    pub target: String,
    /// Everything else it imports.
    pub imports: Vec<Imported>,
}

/// One window's state, shared by its handle and the event loop.
struct State {
    raw: Option<Arc<RawWindow>>,
    surface: Option<Surface>,
    gpu: Arc<Gpu>,
    title: String,
    size: Extent,
    scale: f64,
    showing: Option<Showing>,
    /// What went wrong resizing it, reported at the next present.
    problem: Option<WindowError>,
    /// What happened to it since the last read, oldest first.
    events: Vec<WindowEvent>,
    closes: bool,
}

impl State {
    fn happened(&mut self, event: WindowEvent) {
        if self.events.len() == EVENTS {
            self.events.remove(0);
        }
        self.events.push(event);
    }
}

/// A window on the screen. Dropping every handle to it closes it.
#[derive(Clone)]
pub struct Window {
    id: WindowId,
    state: Rc<RefCell<State>>,
}

impl fmt::Debug for Window {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.borrow();
        formatter
            .debug_struct("Window")
            .field("title", &state.title)
            .field("size", &state.size)
            .field("open", &state.raw.is_some())
            .finish_non_exhaustive()
    }
}

/// The event loop and the windows it serves.
struct System {
    event_loop: EventLoop<()>,
    windows: Vec<(WindowId, Weak<RefCell<State>>)>,
}

thread_local! {
    static SYSTEM: RefCell<Option<System>> = const { RefCell::new(None) };
}

/// Whether the event loop has been made, on any thread: winit allows it once per process.
static MADE: AtomicBool = AtomicBool::new(false);

/// What winit calls back with, for one pump.
struct Handler<'a> {
    windows: &'a [(WindowId, Weak<RefCell<State>>)],
    /// A window to open at the first callback that can, and what opening it gave.
    opening: Option<WindowAttributes>,
    opened: Option<Result<Arc<RawWindow>, String>>,
}

impl Handler<'_> {
    fn open_pending(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(attributes) = self.opening.take() {
            self.opened = Some(
                event_loop
                    .create_window(attributes)
                    .map(Arc::new)
                    .map_err(|problem| problem.to_string()),
            );
        }
    }

    fn state(&self, id: WindowId) -> Option<Rc<RefCell<State>>> {
        self.windows
            .iter()
            .find(|(known, _)| *known == id)
            .and_then(|(_, state)| state.upgrade())
    }
}

impl ApplicationHandler for Handler<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.open_pending(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.open_pending(event_loop);
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, id: WindowId, event: RawEvent) {
        let Some(state) = self.state(id) else {
            return;
        };
        match event {
            RawEvent::Resized(size) => resized(&mut state.borrow_mut(), size),
            RawEvent::ScaleFactorChanged { scale_factor, .. } => {
                let mut state = state.borrow_mut();
                state.scale = scale_factor;
                state.happened(WindowEvent::ScaleChanged(scale_factor));
            }
            RawEvent::CloseRequested => {
                let mut state = state.borrow_mut();
                state.happened(WindowEvent::CloseRequested);
                if state.closes {
                    close(&mut state);
                    state.happened(WindowEvent::Closed);
                }
            }
            RawEvent::Destroyed => {
                let mut state = state.borrow_mut();
                if state.raw.is_some() {
                    close(&mut state);
                    state.happened(WindowEvent::Closed);
                }
            }
            RawEvent::Focused(focused) => {
                state.borrow_mut().happened(WindowEvent::Focused(focused));
                queue(Raw::Focus(focused));
            }
            RawEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(key) = key(code)
                    && !event.repeat
                {
                    queue(Raw::Key(key, event.state == ElementState::Pressed));
                }
            }
            RawEvent::MouseInput {
                state: pressed, button, ..
            } => {
                if let Some(button) = pointer_button(button) {
                    queue(Raw::Button(button, pressed == ElementState::Pressed));
                }
            }
            RawEvent::CursorMoved { position, .. } => {
                // Positions are pixels: a window wider than f32 counts exactly is no window.
                #[allow(clippy::cast_possible_truncation)]
                queue(Raw::Position(position.x as f32, position.y as f32));
            }
            RawEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x, y),
                    // A line is about 20 pixels on every desktop jess draws on.
                    #[allow(clippy::cast_possible_truncation)]
                    MouseScrollDelta::PixelDelta(pixels) => ((pixels.x / 20.0) as f32, (pixels.y / 20.0) as f32),
                };
                queue(Raw::Wheel(x, y));
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _device: RawDeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (x, y) } = event {
            // Motion is pixels since the last event: small numbers.
            #[allow(clippy::cast_possible_truncation)]
            queue(Raw::Motion(x as f32, y as f32));
        }
    }
}

/// `state` at its new `size`: its surface, and the plan it shows, sized again.
fn resized(state: &mut State, size: PhysicalSize<u32>) {
    let size = Extent {
        width: size.width.max(1),
        height: size.height.max(1),
    };
    if size == state.size {
        return;
    }
    state.size = size;
    state.happened(WindowEvent::Resized(size));
    let gpu = Arc::clone(&state.gpu);
    if let Some(surface) = &mut state.surface
        && let Err(problem) = surface.resize(&gpu, state.size.width, state.size.height)
    {
        state.problem = Some(problem.into());
    }
    if let Some(showing) = &mut state.showing {
        match showing.plan.resized(&gpu, state.size) {
            Ok(plan) => showing.plan = plan,
            Err(problem) => state.problem = Some(problem.into()),
        }
    }
}

fn close(state: &mut State) {
    state.showing = None;
    state.surface = None;
    state.raw = None;
}

/// Opens a window on `gpu` as `desc` describes, making the event loop on this thread if it is the
/// first.
///
/// # Errors
///
/// No window system (no display), an event loop another thread made, a window the system
/// refuses to open, or a surface the device can't present to.
pub fn open(gpu: &Arc<Gpu>, desc: &WindowDesc) -> Result<Window, WindowError> {
    SYSTEM.with_borrow_mut(|system| {
        if system.is_none() {
            *system = Some(System::new()?);
        }
        let Some(system) = system else {
            unreachable!("made above");
        };
        let attributes = WindowAttributes::default()
            .with_title(desc.title.clone())
            .with_inner_size(PhysicalSize::new(desc.width.max(1), desc.height.max(1)))
            .with_resizable(desc.resizable);
        let mut handler = Handler {
            windows: &system.windows,
            opening: Some(attributes),
            opened: None,
        };
        // The first pump of a loop resumes it, which opens the window; later ones open it before
        // they wait. Either way one pump does it, and a few more only cover a slow compositor.
        for _ in 0..8 {
            system.event_loop.pump_app_events(Some(Duration::ZERO), &mut handler);
            if handler.opened.is_some() {
                break;
            }
        }
        let raw = handler
            .opened
            .take()
            .ok_or_else(|| WindowError::System("the window system never opened the window".to_owned()))?
            .map_err(|problem| WindowError::System(format!("the window system refused the window: {problem}")))?;
        let size = raw.inner_size();
        let surface = gpu.surface(Arc::clone(&raw), size.width, size.height, desc.format, desc.present)?;
        let id = raw.id();
        let state = Rc::new(RefCell::new(State {
            scale: raw.scale_factor(),
            raw: Some(raw),
            surface: Some(surface),
            gpu: Arc::clone(gpu),
            title: desc.title.clone(),
            size: Extent {
                width: size.width.max(1),
                height: size.height.max(1),
            },
            showing: None,
            problem: None,
            events: Vec::new(),
            closes: desc.closes,
        }));
        system.windows.push((id, Rc::downgrade(&state)));
        Ok(Window { id, state })
    })
}

impl System {
    fn new() -> Result<System, WindowError> {
        if MADE.swap(true, Ordering::AcqRel) {
            return Err(WindowError::System(
                "windows belong to the thread that opened the first one".to_owned(),
            ));
        }
        let mut builder = EventLoop::builder();
        // Tests and workers aren't the main thread; on the desktops that allow it, neither need
        // the first window's thread be.
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
        winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
        #[cfg(target_os = "windows")]
        winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(&mut builder, true);
        let event_loop = builder
            .build()
            .map_err(|problem| WindowError::System(format!("no window system: {problem}")))?;
        Ok(System {
            event_loop,
            windows: Vec::new(),
        })
    }
}

/// Handles everything the window system reported since the last pump, without waiting: sizes,
/// closes, and input. Nothing happens on a thread that never opened a window.
pub fn pump() {
    SYSTEM.with_borrow_mut(|system| {
        let Some(system) = system else {
            return;
        };
        system.windows.retain(|(_, state)| state.strong_count() > 0);
        let mut handler = Handler {
            windows: &system.windows,
            opening: None,
            opened: None,
        };
        system.event_loop.pump_app_events(Some(Duration::ZERO), &mut handler);
    });
}

/// Draws every open window showing a plan, and presents it. A window whose image isn't ready
/// (hidden, covered, or just resized) skips the frame.
///
/// # Errors
///
/// Each window that failed, by title, and why; the others drew.
pub fn present() -> Result<(), Vec<(String, WindowError)>> {
    let states: Vec<Rc<RefCell<State>>> = SYSTEM.with_borrow(|system| {
        system
            .iter()
            .flat_map(|system| &system.windows)
            .filter_map(|(_, state)| state.upgrade())
            .collect()
    });
    let mut failures = Vec::new();
    for state in states {
        let mut state = state.borrow_mut();
        if let Err(problem) = draw(&mut state) {
            failures.push((state.title.clone(), problem));
        }
    }
    if failures.is_empty() { Ok(()) } else { Err(failures) }
}

fn draw(state: &mut State) -> Result<(), WindowError> {
    if let Some(problem) = state.problem.take() {
        return Err(problem);
    }
    let gpu = Arc::clone(&state.gpu);
    let (Some(surface), Some(showing)) = (&mut state.surface, &state.showing) else {
        return Ok(());
    };
    let Some(frame) = surface.frame(&gpu)? else {
        return Ok(());
    };
    let mut imports = vec![Import::Texture(&showing.target, frame.texture())];
    imports.extend(showing.imports.iter().map(|imported| match imported {
        Imported::Texture(name, texture) => Import::Texture(name, texture),
        Imported::Buffer(name, buffer) => Import::Buffer(name, buffer),
    }));
    gpu.run(&showing.plan, &showing.sets, &imports)?;
    gpu.present(frame);
    Ok(())
}

impl Window {
    /// Its title.
    #[must_use]
    pub fn title(&self) -> String {
        self.state.borrow().title.clone()
    }

    /// Its inner size in physical pixels, what its surface and the plan it shows are sized to.
    #[must_use]
    pub fn size(&self) -> Extent {
        self.state.borrow().size
    }

    /// How many physical pixels a logical one is on the display it is on.
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.state.borrow().scale
    }

    /// The format its images are in, which the target a shown plan imports it as must have;
    /// `None` once it is closed.
    #[must_use]
    pub fn format(&self) -> Option<TextureFormat> {
        self.state.borrow().surface.as_ref().map(Surface::format)
    }

    /// What happened to it since the last call, oldest first: at most the last 256 of it.
    #[must_use]
    pub fn events(&self) -> Vec<WindowEvent> {
        std::mem::take(&mut self.state.borrow_mut().events)
    }

    /// Whether it is open: neither closed by the person nor by [`Window::close`].
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.state.borrow().raw.is_some()
    }

    /// Draws `showing` in it from the next [`present`] on, its plan resized to the window, in
    /// place of whatever it showed.
    ///
    /// # Errors
    ///
    /// A closed window, or a plan whose graph has no imported target called `showing.target` of
    /// the window's format.
    pub fn show(&self, mut showing: Showing) -> Result<(), WindowError> {
        let mut state = self.state.borrow_mut();
        let Some(surface) = &state.surface else {
            return Err(WindowError::Invalid(format!("the window {} is closed", state.title)));
        };
        let format = surface.format();
        let graph = showing.plan.graph();
        let declared = graph
            .resource_id(&showing.target)
            .map(|id| &graph.resources()[id.index()])
            .filter(|resource| resource.origin == ResourceOrigin::Imported);
        let Some(resource) = declared else {
            return Err(WindowError::Invalid(format!(
                "the plan imports no target called {}, which the window's image would be",
                showing.target
            )));
        };
        if !matches!(resource.target(), Some(target) if target == TargetDesc::new(format, target.size)) {
            return Err(WindowError::Invalid(format!(
                "the plan's {} isn't a {format} target of one level, one layer, and one sample, which the window's image is",
                showing.target
            )));
        }
        if showing.plan.viewport() != state.size {
            showing.plan = showing.plan.resized(&state.gpu, state.size)?;
        }
        state.showing = Some(showing);
        Ok(())
    }

    /// Stops drawing what it shows; it keeps its last image.
    pub fn hide(&self) {
        self.state.borrow_mut().showing = None;
    }

    /// Closes it.
    pub fn close(&self) {
        close(&mut self.state.borrow_mut());
    }

    /// The id the window system knows it by.
    #[must_use]
    pub fn id(&self) -> WindowId {
        self.id
    }
}

/// What a window reported, before a host's clock stamps it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Raw {
    Key(Key, bool),
    Button(PointerButton, bool),
    Position(f32, f32),
    Motion(f32, f32),
    Wheel(f32, f32),
    Focus(bool),
}

/// The most input kept for a host that hasn't polled: a frame's worth many times over.
const QUEUED: usize = 4096;

/// Input waiting for [`WindowInput`], once one exists.
static QUEUE: Mutex<Vec<Raw>> = Mutex::new(Vec::new());

/// Whether a host listens: without one, window input is dropped, not kept.
static LISTENING: AtomicBool = AtomicBool::new(false);

fn queue(raw: Raw) {
    if !LISTENING.load(Ordering::Acquire) {
        return;
    }
    let mut queue = QUEUE.lock().unwrap_or_else(PoisonError::into_inner);
    if queue.len() < QUEUED {
        queue.push(raw);
    }
}

/// A host's input backend for its windows, polled with any other backend it wraps: one keyboard
/// and one pointer, connected at the first poll.
#[derive(Debug)]
pub struct WindowInput {
    inner: Option<Box<dyn Backend>>,
    devices: Option<(DeviceId, DeviceId)>,
}

impl WindowInput {
    /// The backend for windows, polling `inner` (the platform's, say) first.
    #[must_use]
    pub fn new(inner: Option<Box<dyn Backend>>) -> WindowInput {
        LISTENING.store(true, Ordering::Release);
        WindowInput { inner, devices: None }
    }
}

impl Backend for WindowInput {
    fn name(&self) -> &'static str {
        "window"
    }

    fn poll(&mut self, sender: &EventSender, now: Timestamp) -> Result<(), BackendError> {
        if let Some(inner) = &mut self.inner {
            inner.poll(sender, now)?;
        }
        let drained = std::mem::take(&mut *QUEUE.lock().unwrap_or_else(PoisonError::into_inner));
        if drained.is_empty() {
            return Ok(());
        }
        let (keyboard, pointer) = *self.devices.get_or_insert_with(|| {
            (
                sender.connect(
                    &DeviceIdentity::new("window:keyboard"),
                    DeviceDescriptor::keyboard("window keyboard"),
                    now,
                ),
                sender.connect(
                    &DeviceIdentity::new("window:pointer"),
                    DeviceDescriptor::pointer("window pointer"),
                    now,
                ),
            )
        });
        for raw in drained {
            match raw {
                Raw::Key(key, pressed) => sender.button(keyboard, Control::Key(key), pressed, now),
                Raw::Button(button, pressed) => sender.button(pointer, Control::PointerButton(button), pressed, now),
                Raw::Position(x, y) => {
                    sender.axis(pointer, Control::PointerAxis(PointerAxis::X), x, now);
                    sender.axis(pointer, Control::PointerAxis(PointerAxis::Y), y, now);
                }
                Raw::Motion(x, y) => {
                    sender.axis(pointer, Control::PointerAxis(PointerAxis::DeltaX), x, now);
                    sender.axis(pointer, Control::PointerAxis(PointerAxis::DeltaY), y, now);
                }
                Raw::Wheel(x, y) => {
                    sender.axis(pointer, Control::PointerAxis(PointerAxis::WheelX), x, now);
                    sender.axis(pointer, Control::PointerAxis(PointerAxis::WheelY), y, now);
                }
                Raw::Focus(focused) => sender.focus(focused, now),
            }
        }
        Ok(())
    }
}

fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    Some(match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Back,
        MouseButton::Forward => PointerButton::Forward,
        MouseButton::Other(_) => return None,
    })
}

/// The key at `code`, which winit names by position after the same W3C list jess does.
#[allow(clippy::too_many_lines)] // one arm per key
const fn key(code: KeyCode) -> Option<Key> {
    Some(match code {
        KeyCode::KeyA => Key::KeyA,
        KeyCode::KeyB => Key::KeyB,
        KeyCode::KeyC => Key::KeyC,
        KeyCode::KeyD => Key::KeyD,
        KeyCode::KeyE => Key::KeyE,
        KeyCode::KeyF => Key::KeyF,
        KeyCode::KeyG => Key::KeyG,
        KeyCode::KeyH => Key::KeyH,
        KeyCode::KeyI => Key::KeyI,
        KeyCode::KeyJ => Key::KeyJ,
        KeyCode::KeyK => Key::KeyK,
        KeyCode::KeyL => Key::KeyL,
        KeyCode::KeyM => Key::KeyM,
        KeyCode::KeyN => Key::KeyN,
        KeyCode::KeyO => Key::KeyO,
        KeyCode::KeyP => Key::KeyP,
        KeyCode::KeyQ => Key::KeyQ,
        KeyCode::KeyR => Key::KeyR,
        KeyCode::KeyS => Key::KeyS,
        KeyCode::KeyT => Key::KeyT,
        KeyCode::KeyU => Key::KeyU,
        KeyCode::KeyV => Key::KeyV,
        KeyCode::KeyW => Key::KeyW,
        KeyCode::KeyX => Key::KeyX,
        KeyCode::KeyY => Key::KeyY,
        KeyCode::KeyZ => Key::KeyZ,
        KeyCode::Digit0 => Key::Digit0,
        KeyCode::Digit1 => Key::Digit1,
        KeyCode::Digit2 => Key::Digit2,
        KeyCode::Digit3 => Key::Digit3,
        KeyCode::Digit4 => Key::Digit4,
        KeyCode::Digit5 => Key::Digit5,
        KeyCode::Digit6 => Key::Digit6,
        KeyCode::Digit7 => Key::Digit7,
        KeyCode::Digit8 => Key::Digit8,
        KeyCode::Digit9 => Key::Digit9,
        KeyCode::Minus => Key::Minus,
        KeyCode::Equal => Key::Equal,
        KeyCode::BracketLeft => Key::BracketLeft,
        KeyCode::BracketRight => Key::BracketRight,
        KeyCode::Backslash => Key::Backslash,
        KeyCode::Semicolon => Key::Semicolon,
        KeyCode::Quote => Key::Quote,
        KeyCode::Backquote => Key::Backquote,
        KeyCode::Comma => Key::Comma,
        KeyCode::Period => Key::Period,
        KeyCode::Slash => Key::Slash,
        KeyCode::IntlBackslash => Key::IntlBackslash,
        KeyCode::IntlRo => Key::IntlRo,
        KeyCode::IntlYen => Key::IntlYen,
        KeyCode::Space => Key::Space,
        KeyCode::Enter => Key::Enter,
        KeyCode::Tab => Key::Tab,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Escape => Key::Escape,
        KeyCode::CapsLock => Key::CapsLock,
        KeyCode::ShiftLeft => Key::ShiftLeft,
        KeyCode::ShiftRight => Key::ShiftRight,
        KeyCode::ControlLeft => Key::ControlLeft,
        KeyCode::ControlRight => Key::ControlRight,
        KeyCode::AltLeft => Key::AltLeft,
        KeyCode::AltRight => Key::AltRight,
        KeyCode::SuperLeft => Key::MetaLeft,
        KeyCode::SuperRight => Key::MetaRight,
        KeyCode::ContextMenu => Key::ContextMenu,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::Insert => Key::Insert,
        KeyCode::Delete => Key::Delete,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        KeyCode::F13 => Key::F13,
        KeyCode::F14 => Key::F14,
        KeyCode::F15 => Key::F15,
        KeyCode::F16 => Key::F16,
        KeyCode::F17 => Key::F17,
        KeyCode::F18 => Key::F18,
        KeyCode::F19 => Key::F19,
        KeyCode::F20 => Key::F20,
        KeyCode::F21 => Key::F21,
        KeyCode::F22 => Key::F22,
        KeyCode::F23 => Key::F23,
        KeyCode::F24 => Key::F24,
        KeyCode::PrintScreen => Key::PrintScreen,
        KeyCode::ScrollLock => Key::ScrollLock,
        KeyCode::Pause => Key::Pause,
        KeyCode::NumLock => Key::NumLock,
        KeyCode::Numpad0 => Key::Numpad0,
        KeyCode::Numpad1 => Key::Numpad1,
        KeyCode::Numpad2 => Key::Numpad2,
        KeyCode::Numpad3 => Key::Numpad3,
        KeyCode::Numpad4 => Key::Numpad4,
        KeyCode::Numpad5 => Key::Numpad5,
        KeyCode::Numpad6 => Key::Numpad6,
        KeyCode::Numpad7 => Key::Numpad7,
        KeyCode::Numpad8 => Key::Numpad8,
        KeyCode::Numpad9 => Key::Numpad9,
        KeyCode::NumpadAdd => Key::NumpadAdd,
        KeyCode::NumpadSubtract => Key::NumpadSubtract,
        KeyCode::NumpadMultiply => Key::NumpadMultiply,
        KeyCode::NumpadDivide => Key::NumpadDivide,
        KeyCode::NumpadDecimal => Key::NumpadDecimal,
        KeyCode::NumpadEnter => Key::NumpadEnter,
        KeyCode::NumpadEqual => Key::NumpadEqual,
        KeyCode::NumpadComma => Key::NumpadComma,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_named_by_position() {
        assert_eq!(key(KeyCode::KeyW), Some(Key::KeyW));
        assert_eq!(key(KeyCode::SuperLeft), Some(Key::MetaLeft));
        assert_eq!(key(KeyCode::NumpadEnter), Some(Key::NumpadEnter));
        assert_eq!(key(KeyCode::MediaPlayPause), None, "media keys aren't jess keys");
    }

    #[test]
    fn window_input_reaches_the_host_at_its_clock() {
        let mut backend = WindowInput::new(None);
        queue(Raw::Key(Key::KeyW, true));
        queue(Raw::Motion(3.0, -2.0));
        let input = crate::input::InputQueue::new();
        let now = Timestamp::ZERO + Duration::from_millis(16);
        backend.poll(&input.sender(), now).unwrap();
        let mut events = Vec::new();
        input.drain(&mut events);
        assert!(events.iter().all(|event| event.timestamp == now));
        let kinds: Vec<&crate::input::InputEventKind> = events.iter().map(|event| &event.kind).collect();
        assert_eq!(kinds.len(), 5, "a keyboard and a pointer connect, then a key and two axes");
        assert!(matches!(
            kinds[2],
            crate::input::InputEventKind::Button {
                control: Control::Key(Key::KeyW),
                pressed: true,
                ..
            }
        ));
        assert!(matches!(
            kinds[4],
            crate::input::InputEventKind::Axis { control: Control::PointerAxis(PointerAxis::DeltaY), value, .. }
                if (*value + 2.0).abs() < f32::EPSILON
        ));

        backend.poll(&input.sender(), now).unwrap();
        input.drain(&mut events);
        assert_eq!(events.len(), 5, "nothing new, and the devices connect once");
    }
}
