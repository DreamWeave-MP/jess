//! Input for jess applications: device truth, an action graph, and semantic intents.
//!
//! Input is a stack, and each layer only knows the one under it:
//!
//! ```text
//! physical device truth      Control, InputEvent, InputState        (backends, SyntheticSource)
//!     v
//! binding document           GraphSpec, NodeSpec                    (serialized, remapped)
//!     v
//! compiled action graph      ActionGraph                            (validated)
//!     v
//! sets and layers            Evaluator                              (allocation-free tick)
//!     v
//! semantic actions           ActionState, ActionEvent
//!     v
//! intents                    IntentRouter, IntentDispatch           (a consumer supplies scope)
//! ```
//!
//! The module knows what a key is and what a chord is. It doesn't know what jumping is: action
//! names belong to the packages that declare them, and nothing here reserves one. It draws
//! nothing either; [`presentations`] hands a GUI the shape of a binding and the label each
//! control's [`PresentationFamily`] gives it, and the GUI decides what that looks like.
//!
//! The default build is headless and has no platform dependencies. Real devices come from a
//! [`Backend`] behind a feature; tests, replays, and scripts use a [`SyntheticSource`], whose
//! clock moves only when told to.
//!
//! # Example
//!
//! `Ctrl+S` as a chord, driven without hardware:
//!
//! ```
//! use std::time::Duration;
//! use jess::input::{Control, GraphSpec, InputRuntime, Key, NodeSpec, Owner, ValueType};
//!
//! let mut spec = GraphSpec::new();
//! spec.action("editor.save", ValueType::Digital).set("editor", "@jess/example");
//! spec.add_binding(
//!     &Owner::Set("editor".to_owned()),
//!     "editor.save",
//!     NodeSpec::chord([
//!         NodeSpec::button(Control::Key(Key::ControlLeft)),
//!         NodeSpec::button(Control::Key(Key::KeyS)),
//!     ]),
//! )?;
//!
//! let mut runtime = InputRuntime::new();
//! runtime.install(spec).map_err(|mut errors| errors.remove(0))?;
//! runtime.set_active("editor", true)?;
//!
//! let mut source = runtime.synthetic();
//! let keyboard = source.connect_keyboard();
//! source.press(keyboard, Control::Key(Key::ControlLeft));
//! runtime.tick(source.now());
//! assert!(!runtime.action("editor.save").unwrap().active, "half a chord is nothing");
//!
//! source.advance(Duration::from_millis(30));
//! source.press(keyboard, Control::Key(Key::KeyS));
//! runtime.tick(source.now());
//! assert!(runtime.action("editor.save").unwrap().pressed_this_tick);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Sharing one runtime
//!
//! An application has one input. [`InputService`] is the runtime plus what a script configures
//! around it (intent routes by name, a synthetic side), as one unit a host can put behind a
//! `Mutex`, tick at its input phase, and hand to a script runtime as a service. `jess::host`
//! ticks it; `jess.input` binds to it when the script's domain carries one.
//!
//! # What is reserved
//!
//! Haptics: a [`DeviceDescriptor`] lists [`OutputChannel`]s and nothing drives them. Text input
//! and IME belong to the GUI and the platform. Capability enforcement belongs to the script
//! boundary: [`capability`] names what each group of calls needs, and `jess.input` checks them
//! under a script domain.

#![warn(missing_docs)]

pub mod backend;
mod control;
mod device;
mod event;
mod family;
pub mod graph;
mod id;
mod intent;
mod presentation;
mod queue;
mod runtime;
mod service;
mod state;
mod synthetic;
mod time;

pub use backend::{Backend, BackendError, StubBackend, platform_backend};
pub use control::{
    AxisRange, Control, ControlKind, ControlParseError, GamepadAxis, GamepadButton, Key, PointerAxis, PointerButton,
    TouchAxis,
};
pub use device::{DeviceCapability, DeviceDescriptor, DeviceIdentity, DeviceKind, OutputChannel, OutputChannelKind};
pub use event::{InputEvent, InputEventKind};
pub use family::PresentationFamily;
pub use graph::compile::{Action, ActionGraph, Binding, GraphError, GraphErrorKind, OwnerId};
pub use graph::eval::{ActionEvent, ActionEventKind, ActionState, EvaluationStats, Evaluator, Value};
pub use graph::spec::{
    ActionSpec, BindingLocation, BindingSpec, FORMAT_VERSION, GraphSpec, LayerSpec, NodeSpec, Origin, Owner, SetSpec,
    SpecError, ValueType,
};
pub use id::{ActionId, BindingId, DeviceId, Generation, IntentId, LayerId, NodeId, SetId};
pub use intent::{IntentDispatch, IntentRegistry, IntentRoute, IntentRouter, IntentScope};
pub use presentation::{
    BindingPresentation, Conflict, ConflictKind, ControlPresentation, Presentation, binding_presentation, conflicts,
    presentations,
};
pub use queue::{EventSender, InputQueue};
pub use runtime::{InputRuntime, RuntimeError, RuntimeStats};
pub use service::{InputService, NamedRoute, Tick};
pub use state::{AxisState, ButtonState, DeviceState, InputState};
pub use synthetic::SyntheticSource;
pub use time::{MonotonicClock, Timestamp};

/// The capabilities a script host grants, by the dotted names the script runtime's broker knows
/// them by.
///
/// `jess::input` enforces none of them: a native caller holding an [`InputRuntime`] already has
/// everything. They are named here so the binding layer that stands between a script and the
/// runtime checks the same three things everywhere, and so a package that only consumes actions
/// never needs to be trusted with raw keystrokes. `jess.input` checks them through
/// `jess::runtime::require_capability` on every call that touches a runtime, whenever the script
/// runs under a `ScriptDomain`; outside any domain (a plain `jess run`, a test) nothing holds
/// authority and every call succeeds, since the process's own user is running it.
pub mod capability {
    /// Reading raw device truth: [`crate::input::InputState`], device descriptors, and device events.
    /// A package granted this can see every key pressed while it runs.
    pub const OBSERVE: &str = "input.observe";

    /// Reading semantic actions and intents: [`crate::input::ActionState`], [`crate::input::ActionEvent`],
    /// intent dispatches, and binding presentation. No raw device state.
    pub const ACTIONS: &str = "input.actions";

    /// Changing input: installing a graph, remapping, activating sets, pushing layers, adding
    /// intent routes, and pushing synthetic events.
    pub const CONFIGURE: &str = "input.configure";
}
