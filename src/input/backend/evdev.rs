//! The Linux backend: `/dev/input/event*` through evdev.
//!
//! Controllers don't agree on how they say things, and a backend that listens to one event class
//! hears half a controller. What this one handles, and why:
//!
//! - Shoulder buttons and face buttons arrive as `EV_KEY` events (`BTN_TL`, `BTN_SOUTH`).
//! - A D-pad arrives as `EV_KEY` events (`BTN_DPAD_*`) on some pads and as hat axes
//!   (`ABS_HAT0X`, `ABS_HAT0Y`, each `-1`, `0`, or `1`) on others; a few send both. Hats become
//!   D-pad button events here, and a pad that has hat axes has its `BTN_DPAD_*` keys ignored,
//!   so one press is one press.
//! - Axes report raw integers in a range only the device knows (`EVIOCGABS`), so each is
//!   normalized from its own minimum and maximum. No deadzone is applied: that is the action
//!   graph's decision, and the raw value is the truth.
//! - Axis and button meanings follow the kernel's gamepad specification
//!   (`Documentation/input/gamepad.rst`): `ABS_X`/`ABS_Y` and `ABS_RX`/`ABS_RY` are the sticks,
//!   `ABS_Z`/`ABS_RZ` the triggers, and `Y` grows downward, which Jess inverts.
//!
//! Devices come and go without notice, so the device folder is rescanned about once a second,
//! and a read that fails with anything but "nothing to read" is a disconnect. Opening
//! `/dev/input` needs permission (the `input` group, or a seat's ACLs); a device that can't be
//! opened is skipped, and a machine where none can be is a machine with no devices, not an error.

use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ::evdev::{AbsoluteAxisCode, Device, EventType, KeyCode, RelativeAxisCode};

use super::{Backend, BackendError};
use crate::input::{
    Control, DeviceCapability, DeviceDescriptor, DeviceId, DeviceIdentity, DeviceKind, EventSender, GamepadAxis,
    GamepadButton, Key, PointerAxis, PointerButton, PresentationFamily, Timestamp,
};

const DEVICE_FOLDER: &str = "/dev/input";
const RESCAN_INTERVAL: Duration = Duration::from_secs(1);

const BTN_LEFT: u16 = 0x110;
const BTN_SOUTH: u16 = 0x130;
const BTN_TL: u16 = 0x136;
const BTN_TR: u16 = 0x137;
const BTN_SELECT: u16 = 0x13a;
const BTN_START: u16 = 0x13b;
const BTN_DPAD_UP: u16 = 0x220;
const BTN_DPAD_RIGHT: u16 = 0x223;
const KEY_ENTER: u16 = 28;
const KEY_A: u16 = 30;

const ABS_X: u16 = 0x00;
const ABS_Y: u16 = 0x01;
const ABS_Z: u16 = 0x02;
const ABS_RX: u16 = 0x03;
const ABS_RY: u16 = 0x04;
const ABS_RZ: u16 = 0x05;
const ABS_HAT0X: u16 = 0x10;
const ABS_HAT0Y: u16 = 0x11;

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_HWHEEL: u16 = 0x06;
const REL_WHEEL: u16 = 0x08;

/// The key at a Linux key code (`linux/input-event-codes.h`), which is already positional.
#[allow(clippy::too_many_lines)] // one arm per key
const fn key_from_code(code: u16) -> Option<Key> {
    Some(match code {
        1 => Key::Escape,
        2 => Key::Digit1,
        3 => Key::Digit2,
        4 => Key::Digit3,
        5 => Key::Digit4,
        6 => Key::Digit5,
        7 => Key::Digit6,
        8 => Key::Digit7,
        9 => Key::Digit8,
        10 => Key::Digit9,
        11 => Key::Digit0,
        12 => Key::Minus,
        13 => Key::Equal,
        14 => Key::Backspace,
        15 => Key::Tab,
        16 => Key::KeyQ,
        17 => Key::KeyW,
        18 => Key::KeyE,
        19 => Key::KeyR,
        20 => Key::KeyT,
        21 => Key::KeyY,
        22 => Key::KeyU,
        23 => Key::KeyI,
        24 => Key::KeyO,
        25 => Key::KeyP,
        26 => Key::BracketLeft,
        27 => Key::BracketRight,
        28 => Key::Enter,
        29 => Key::ControlLeft,
        30 => Key::KeyA,
        31 => Key::KeyS,
        32 => Key::KeyD,
        33 => Key::KeyF,
        34 => Key::KeyG,
        35 => Key::KeyH,
        36 => Key::KeyJ,
        37 => Key::KeyK,
        38 => Key::KeyL,
        39 => Key::Semicolon,
        40 => Key::Quote,
        41 => Key::Backquote,
        42 => Key::ShiftLeft,
        43 => Key::Backslash,
        44 => Key::KeyZ,
        45 => Key::KeyX,
        46 => Key::KeyC,
        47 => Key::KeyV,
        48 => Key::KeyB,
        49 => Key::KeyN,
        50 => Key::KeyM,
        51 => Key::Comma,
        52 => Key::Period,
        53 => Key::Slash,
        54 => Key::ShiftRight,
        55 => Key::NumpadMultiply,
        56 => Key::AltLeft,
        57 => Key::Space,
        58 => Key::CapsLock,
        59 => Key::F1,
        60 => Key::F2,
        61 => Key::F3,
        62 => Key::F4,
        63 => Key::F5,
        64 => Key::F6,
        65 => Key::F7,
        66 => Key::F8,
        67 => Key::F9,
        68 => Key::F10,
        69 => Key::NumLock,
        70 => Key::ScrollLock,
        71 => Key::Numpad7,
        72 => Key::Numpad8,
        73 => Key::Numpad9,
        74 => Key::NumpadSubtract,
        75 => Key::Numpad4,
        76 => Key::Numpad5,
        77 => Key::Numpad6,
        78 => Key::NumpadAdd,
        79 => Key::Numpad1,
        80 => Key::Numpad2,
        81 => Key::Numpad3,
        82 => Key::Numpad0,
        83 => Key::NumpadDecimal,
        86 => Key::IntlBackslash,
        87 => Key::F11,
        88 => Key::F12,
        89 => Key::IntlRo,
        96 => Key::NumpadEnter,
        97 => Key::ControlRight,
        98 => Key::NumpadDivide,
        99 => Key::PrintScreen,
        100 => Key::AltRight,
        102 => Key::Home,
        103 => Key::ArrowUp,
        104 => Key::PageUp,
        105 => Key::ArrowLeft,
        106 => Key::ArrowRight,
        107 => Key::End,
        108 => Key::ArrowDown,
        109 => Key::PageDown,
        110 => Key::Insert,
        111 => Key::Delete,
        117 => Key::NumpadEqual,
        119 => Key::Pause,
        121 => Key::NumpadComma,
        124 => Key::IntlYen,
        125 => Key::MetaLeft,
        126 => Key::MetaRight,
        127 => Key::ContextMenu,
        183 => Key::F13,
        184 => Key::F14,
        185 => Key::F15,
        186 => Key::F16,
        187 => Key::F17,
        188 => Key::F18,
        189 => Key::F19,
        190 => Key::F20,
        191 => Key::F21,
        192 => Key::F22,
        193 => Key::F23,
        194 => Key::F24,
        _ => return None,
    })
}

/// The gamepad button at a `BTN_*` code, by the kernel's positional names.
const fn gamepad_button_from_code(code: u16) -> Option<GamepadButton> {
    Some(match code {
        0x130 => GamepadButton::South,
        0x131 => GamepadButton::East,
        0x133 => GamepadButton::North,
        0x134 => GamepadButton::West,
        0x136 => GamepadButton::LeftBumper,
        0x137 => GamepadButton::RightBumper,
        0x138 => GamepadButton::LeftTrigger,
        0x139 => GamepadButton::RightTrigger,
        0x13a => GamepadButton::Select,
        0x13b => GamepadButton::Start,
        0x13c => GamepadButton::Guide,
        0x13d => GamepadButton::LeftStick,
        0x13e => GamepadButton::RightStick,
        0x220 => GamepadButton::DPadUp,
        0x221 => GamepadButton::DPadDown,
        0x222 => GamepadButton::DPadLeft,
        0x223 => GamepadButton::DPadRight,
        _ => return None,
    })
}

const fn pointer_button_from_code(code: u16) -> Option<PointerButton> {
    Some(match code {
        0x110 => PointerButton::Primary,
        0x111 => PointerButton::Secondary,
        0x112 => PointerButton::Middle,
        0x113 => PointerButton::Back,
        0x114 => PointerButton::Forward,
        _ => return None,
    })
}

/// The gamepad axis at an `ABS_*` code, and whether evdev's direction is the reverse of Jess's.
const fn gamepad_axis_from_code(code: u16) -> Option<(GamepadAxis, bool)> {
    Some(match code {
        ABS_X => (GamepadAxis::LeftStickX, false),
        ABS_Y => (GamepadAxis::LeftStickY, true),
        ABS_RX => (GamepadAxis::RightStickX, false),
        ABS_RY => (GamepadAxis::RightStickY, true),
        ABS_Z => (GamepadAxis::LeftTrigger, false),
        ABS_RZ => (GamepadAxis::RightTrigger, false),
        _ => return None,
    })
}

/// A raw axis value in `minimum..=maximum` as `-1.0..=1.0` for a stick, `0.0..=1.0` for a trigger.
#[allow(clippy::cast_precision_loss)] // Axis ranges are 16 bits or fewer.
fn normalize(value: i32, minimum: i32, maximum: i32, centered: bool) -> f32 {
    if maximum <= minimum {
        return 0.0;
    }
    let unit = ((value.clamp(minimum, maximum) - minimum) as f32) / ((maximum - minimum) as f32);
    if centered { unit.mul_add(2.0, -1.0) } else { unit }
}

/// What a device node is, from what it says it has. `None` is a node jess has no use for: a
/// power button, a lid switch, a webcam's shutter key.
fn classify(
    has_key: impl Fn(u16) -> bool,
    has_absolute: impl Fn(u16) -> bool,
    has_relative: impl Fn(u16) -> bool,
) -> Option<DeviceKind> {
    let pad_button = [BTN_SOUTH, BTN_TL, BTN_TR, BTN_SELECT, BTN_START].into_iter().any(&has_key);
    let dpad_keys = (BTN_DPAD_UP..=BTN_DPAD_RIGHT).any(&has_key);
    let sticks = has_absolute(ABS_X) && has_absolute(ABS_Y);
    let hats = has_absolute(ABS_HAT0X) && has_absolute(ABS_HAT0Y);
    // A controller has controller buttons and something to steer with; an absolute pointer
    // (a tablet, a touchpad) has the axes but not the buttons.
    if (pad_button || dpad_keys) && (sticks || hats || dpad_keys) {
        return Some(DeviceKind::Gamepad);
    }
    if has_key(KEY_A) && has_key(KEY_ENTER) {
        return Some(DeviceKind::Keyboard);
    }
    if has_key(BTN_LEFT) && has_relative(REL_X) && has_relative(REL_Y) {
        return Some(DeviceKind::Pointer);
    }
    None
}

/// The identity of a device node: its unique name (a serial or a Bluetooth address) with the
/// interface it is on the device, else the port it is plugged into, else what it says it is.
fn identity(
    kind: DeviceKind,
    unique: Option<&str>,
    physical: Option<&str>,
    vendor: u16,
    product: u16,
    name: &str,
) -> DeviceIdentity {
    fn present(text: Option<&str>) -> Option<&str> {
        text.map(str::trim).filter(|text| !text.is_empty())
    }
    let key = match (present(unique), present(physical)) {
        (Some(unique), physical) => {
            let interface = physical.and_then(|path| path.rsplit('/').next()).unwrap_or_default();
            format!("evdev:{unique}/{interface}/{kind}")
        }
        (None, Some(physical)) => format!("evdev:{physical}/{kind}"),
        (None, None) => format!("evdev:{vendor:04x}:{product:04x}:{name}/{kind}"),
    };
    DeviceIdentity::new(key)
}

#[derive(Clone, Copy, Debug)]
struct AxisInfo {
    code: u16,
    minimum: i32,
    maximum: i32,
}

#[derive(Debug)]
struct OpenDevice {
    device: Device,
    id: DeviceId,
    kind: DeviceKind,
    axes: Vec<AxisInfo>,
    has_hats: bool,
    /// The hat's last `[x, y]`, each `-1`, `0`, or `1`.
    hat: [i32; 2],
}

impl OpenDevice {
    /// Opens and announces the device at `path`, if jess has a use for it.
    fn open(path: &Path, sender: &EventSender, now: Timestamp) -> Option<Self> {
        let device = Device::open(path).ok()?;
        let has_key = |code: u16| device.supported_keys().is_some_and(|keys| keys.contains(KeyCode::new(code)));
        let has_absolute = |code: u16| {
            device
                .supported_absolute_axes()
                .is_some_and(|axes| axes.contains(AbsoluteAxisCode(code)))
        };
        let has_relative = |code: u16| {
            device
                .supported_relative_axes()
                .is_some_and(|axes| axes.contains(RelativeAxisCode(code)))
        };
        let kind = classify(has_key, has_absolute, has_relative)?;
        device.set_nonblocking(true).ok()?;

        let input_id = device.input_id();
        let (vendor, product) = (input_id.vendor(), input_id.product());
        let name = device.name().unwrap_or("Unknown device").to_owned();
        let mut descriptor = match kind {
            DeviceKind::Gamepad => DeviceDescriptor::gamepad(name.clone(), Some(vendor), Some(product)),
            DeviceKind::Pointer => DeviceDescriptor::pointer(name.clone()),
            _ => DeviceDescriptor::keyboard(name.clone()),
        };
        if kind != DeviceKind::Gamepad {
            descriptor.vendor_id = Some(vendor);
            descriptor.product_id = Some(product);
            descriptor.family = PresentationFamily::KeyboardMouse;
        }
        if kind == DeviceKind::Pointer && !has_relative(REL_WHEEL) {
            descriptor
                .capabilities
                .retain(|capability| *capability != DeviceCapability::Wheel);
        }
        descriptor.backend = Some(format!("evdev:{}", path.display()));

        let axes = device
            .get_absinfo()
            .map(|info| {
                info.map(|(code, info)| AxisInfo {
                    code: code.0,
                    minimum: info.minimum(),
                    maximum: info.maximum(),
                })
                .collect()
            })
            .unwrap_or_default();
        let has_hats = has_absolute(ABS_HAT0X) && has_absolute(ABS_HAT0Y);
        let identity = identity(kind, device.unique_name(), device.physical_path(), vendor, product, &name);
        let id = sender.connect(&identity, descriptor, now);
        Some(Self {
            device,
            id,
            kind,
            axes,
            has_hats,
            hat: [0, 0],
        })
    }

    /// Pushes everything the device has queued. `Err` is the device being gone.
    fn poll(&mut self, sender: &EventSender, now: Timestamp) -> Result<(), ()> {
        loop {
            let events: Vec<_> = match self.device.fetch_events() {
                Ok(events) => events.collect(),
                Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => return Ok(()),
                Err(_) => return Err(()),
            };
            if events.is_empty() {
                return Ok(());
            }
            for event in events {
                self.translate(sender, now, event.event_type(), event.code(), event.value());
            }
        }
    }

    fn translate(&mut self, sender: &EventSender, now: Timestamp, event_type: EventType, code: u16, value: i32) {
        match event_type {
            // 0 is a release, 1 a press, 2 the kernel's autorepeat, which is nobody's press.
            EventType::KEY if value < 2 => {
                let control = match self.kind {
                    DeviceKind::Gamepad => gamepad_button_from_code(code)
                        .filter(|_| !(self.has_hats && (BTN_DPAD_UP..=BTN_DPAD_RIGHT).contains(&code)))
                        .map(Control::GamepadButton),
                    DeviceKind::Pointer => pointer_button_from_code(code).map(Control::PointerButton),
                    _ => key_from_code(code).map(Control::Key),
                };
                if let Some(control) = control {
                    sender.button(self.id, control, value == 1, now);
                }
            }
            EventType::ABSOLUTE if self.kind == DeviceKind::Gamepad => match code {
                ABS_HAT0X | ABS_HAT0Y => self.hat(sender, now, usize::from(code - ABS_HAT0X), value.signum()),
                _ => {
                    let Some((axis, reversed)) = gamepad_axis_from_code(code) else {
                        return;
                    };
                    let Some(info) = self.axes.iter().find(|info| info.code == code) else {
                        return;
                    };
                    let centered = !matches!(axis, GamepadAxis::LeftTrigger | GamepadAxis::RightTrigger);
                    let value = normalize(value, info.minimum, info.maximum, centered);
                    let value = if reversed { -value } else { value };
                    sender.axis(self.id, Control::GamepadAxis(axis), value, now);
                }
            },
            EventType::RELATIVE if self.kind == DeviceKind::Pointer => {
                let axis = match code {
                    REL_X => PointerAxis::DeltaX,
                    REL_Y => PointerAxis::DeltaY,
                    REL_WHEEL => PointerAxis::WheelY,
                    REL_HWHEEL => PointerAxis::WheelX,
                    _ => return,
                };
                #[allow(clippy::cast_precision_loss)] // Motion counts are small.
                sender.axis(self.id, Control::PointerAxis(axis), value as f32, now);
            }
            _ => {}
        }
    }

    /// A hat axis moved: release the D-pad button it left and press the one it reached.
    fn hat(&mut self, sender: &EventSender, now: Timestamp, axis: usize, value: i32) {
        let previous = std::mem::replace(&mut self.hat[axis], value);
        if previous == value {
            return;
        }
        if let Some(released) = hat_button(axis, previous) {
            sender.button(self.id, Control::GamepadButton(released), false, now);
        }
        if let Some(pressed) = hat_button(axis, value) {
            sender.button(self.id, Control::GamepadButton(pressed), true, now);
        }
    }
}

/// The D-pad button a hat axis (`0` is `ABS_HAT0X`, `1` is `ABS_HAT0Y`) holds at `value`. A hat's
/// `Y` is `-1` upward, like every evdev `Y`.
const fn hat_button(axis: usize, value: i32) -> Option<GamepadButton> {
    match (axis, value) {
        (0, -1) => Some(GamepadButton::DPadLeft),
        (0, 1) => Some(GamepadButton::DPadRight),
        (1, -1) => Some(GamepadButton::DPadUp),
        (1, 1) => Some(GamepadButton::DPadDown),
        _ => None,
    }
}

/// The evdev backend: every keyboard, pointer, and gamepad under `/dev/input` this process may
/// open.
#[derive(Debug, Default)]
pub struct EvdevBackend {
    devices: BTreeMap<PathBuf, OpenDevice>,
    last_scan: Option<Timestamp>,
}

impl EvdevBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// The `eventN` nodes in the device folder, sorted.
    fn event_paths() -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(DEVICE_FOLDER) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .and_then(|name| name.strip_prefix("event"))
                    .is_some_and(|number| !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()))
            })
            .collect();
        paths.sort();
        paths
    }

    fn rescan(&mut self, sender: &EventSender, now: Timestamp) {
        let paths = Self::event_paths();
        let gone: Vec<PathBuf> = self.devices.keys().filter(|path| !paths.contains(path)).cloned().collect();
        for path in gone {
            if let Some(open) = self.devices.remove(&path) {
                sender.disconnect(open.id, now);
            }
        }
        for path in paths {
            if !self.devices.contains_key(&path)
                && let Some(open) = OpenDevice::open(&path, sender, now)
            {
                self.devices.insert(path, open);
            }
        }
        self.last_scan = Some(now);
    }
}

impl Backend for EvdevBackend {
    fn name(&self) -> &'static str {
        "evdev"
    }

    fn poll(&mut self, sender: &EventSender, now: Timestamp) -> Result<(), BackendError> {
        if self.last_scan.is_none_or(|last| now.since(last) >= RESCAN_INTERVAL) {
            self.rescan(sender, now);
        }
        let mut gone = Vec::new();
        for (path, open) in &mut self.devices {
            if open.poll(sender, now).is_err() {
                gone.push((path.clone(), open.id));
            }
        }
        for (path, id) in gone {
            self.devices.remove(&path);
            sender.disconnect(id, now);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pad_with_sticks_or_a_hat_or_dpad_keys_is_a_gamepad() {
        let sticks = classify(|code| code == BTN_SOUTH, |code| code <= ABS_Y, |_| false);
        assert_eq!(sticks, Some(DeviceKind::Gamepad));
        let hat_only = classify(
            |code| code == BTN_TL,
            |code| code == ABS_HAT0X || code == ABS_HAT0Y,
            |_| false,
        );
        assert_eq!(hat_only, Some(DeviceKind::Gamepad));
        let dpad_keys = classify(|code| (BTN_DPAD_UP..=BTN_DPAD_RIGHT).contains(&code), |_| false, |_| false);
        assert_eq!(dpad_keys, Some(DeviceKind::Gamepad));
    }

    #[test]
    fn an_absolute_pointer_without_controller_buttons_is_not_a_gamepad() {
        let tablet = classify(|code| code == BTN_LEFT, |code| code <= ABS_Y, |_| false);
        assert_eq!(tablet, None);
        let mouse = classify(|code| code == BTN_LEFT, |_| false, |code| code <= REL_Y);
        assert_eq!(mouse, Some(DeviceKind::Pointer));
        let keyboard = classify(|code| code == KEY_A || code == KEY_ENTER, |_| false, |_| false);
        assert_eq!(keyboard, Some(DeviceKind::Keyboard));
        let power_button = classify(|code| code == 116, |_| false, |_| false);
        assert_eq!(power_button, None);
    }

    #[test]
    fn axes_normalize_from_the_devices_own_range() {
        assert!((normalize(0, -32768, 32767, true) - 0.0).abs() < 1e-4);
        assert!((normalize(-32768, -32768, 32767, true) + 1.0).abs() < f32::EPSILON);
        assert!((normalize(32767, -32768, 32767, true) - 1.0).abs() < f32::EPSILON);
        assert!((normalize(128, 0, 255, true) - 0.003_921_6).abs() < 1e-4);
        assert!((normalize(255, 0, 255, false) - 1.0).abs() < f32::EPSILON);
        assert!((normalize(0, 0, 1023, false) - 0.0).abs() < f32::EPSILON);
        assert!(
            (normalize(9000, 0, 255, false) - 1.0).abs() < f32::EPSILON,
            "out of range clamps"
        );
        assert!(
            (normalize(5, 7, 7, true) - 0.0).abs() < f32::EPSILON,
            "a degenerate range is rest"
        );
    }

    #[test]
    fn codes_translate_to_the_jess_vocabulary() {
        assert_eq!(key_from_code(KEY_A), Some(Key::KeyA));
        assert_eq!(key_from_code(29), Some(Key::ControlLeft));
        assert_eq!(key_from_code(194), Some(Key::F24));
        assert_eq!(key_from_code(116), None, "the power key is nobody's binding");
        assert_eq!(gamepad_button_from_code(BTN_TL), Some(GamepadButton::LeftBumper));
        assert_eq!(gamepad_button_from_code(0x138), Some(GamepadButton::LeftTrigger));
        assert_eq!(gamepad_axis_from_code(ABS_Y), Some((GamepadAxis::LeftStickY, true)));
        assert_eq!(gamepad_axis_from_code(ABS_RZ), Some((GamepadAxis::RightTrigger, false)));
        assert_eq!(pointer_button_from_code(0x113), Some(PointerButton::Back));
    }

    #[test]
    fn identities_prefer_the_most_stable_fact() {
        let bluetooth = identity(DeviceKind::Gamepad, Some("aa:bb:cc"), Some("00:11:22"), 0x054c, 0x0ce6, "Pad");
        assert_eq!(bluetooth.as_str(), "evdev:aa:bb:cc/00:11:22/gamepad");
        let usb = identity(
            DeviceKind::Keyboard,
            Some(""),
            Some("usb-0000:00:14.0-1/input0"),
            1,
            2,
            "Keyboard",
        );
        assert_eq!(usb.as_str(), "evdev:usb-0000:00:14.0-1/input0/keyboard");
        let nameless = identity(DeviceKind::Pointer, None, None, 0x046d, 0xc077, "Mouse");
        assert_eq!(nameless.as_str(), "evdev:046d:c077:Mouse/pointer");
    }

    #[test]
    fn hats_are_dpad_buttons() {
        assert_eq!(hat_button(0, -1), Some(GamepadButton::DPadLeft));
        assert_eq!(hat_button(0, 1), Some(GamepadButton::DPadRight));
        assert_eq!(hat_button(1, -1), Some(GamepadButton::DPadUp));
        assert_eq!(hat_button(1, 1), Some(GamepadButton::DPadDown));
        assert_eq!(hat_button(0, 0), None);
    }
}
