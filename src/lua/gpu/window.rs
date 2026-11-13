//! Windows from Luau: `jess.gpu.window` opens one on a device, `window:show` gives it a plan to draw
//! every frame, and `jess.gpu.pump` handles the window system and draws, for a script no
//! application host drives.

use std::sync::Arc;

use l3i::bind::{Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata, receiver};

use super::device::LuaGpuDevice;
use super::pipeline::{flag, options};
use super::plan::{Given, LuaGpuPlan, given};
use super::resources::{format_name, format_of, shown, whole};
use crate::gpu::PresentMode;
use crate::lua::{error, kind, push_table};
use crate::window::{CursorRequest, Fullscreen, Imported, Monitor, Showing, TextArea, Window, WindowDesc, WindowEvent};

/// A window and the device its surface is on.
#[derive(Debug)]
pub struct LuaGpuWindow {
    window: Window,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuWindow {
    const NAME: &'static str = "jess.gpu.Window";
}

const WINDOW_KEYS: [&str; 9] = [
    "closes",
    "format",
    "fullscreen",
    "height",
    "key",
    "resizable",
    "title",
    "vsync",
    "width",
];

/// How much of a display `view` asks for, for `function`.
fn fullscreen_of(function: &str, view: ValueView<'_>) -> l3i::Result<Fullscreen> {
    match view.is_string().then(|| view.read::<&str>()).transpose()? {
        Some("windowed") => Ok(Fullscreen::Windowed),
        Some("borderless") => Ok(Fullscreen::Borderless),
        Some("exclusive") => Ok(Fullscreen::Exclusive),
        _ => Err(error(format!(
            "{function}: fullscreen is 'windowed', 'borderless', or 'exclusive', not {}",
            shown(view)
        ))),
    }
}

/// Fills `table` with `monitor`, as `jess.gpu.monitors` lists it.
fn monitor_fields(frame: &Frame<'_>, table: &TableView<'_>, monitor: &Monitor) -> l3i::Result<()> {
    if let Some(name) = &monitor.name {
        table.raw_set_value(frame, "name", name.as_str())?;
    }
    table.raw_set_value(frame, "width", &f64::from(monitor.width))?;
    table.raw_set_value(frame, "height", &f64::from(monitor.height))?;
    table.raw_set_value(frame, "x", &f64::from(monitor.x))?;
    table.raw_set_value(frame, "y", &f64::from(monitor.y))?;
    table.raw_set_value(frame, "scale", &monitor.scale)?;
    if let Some(refresh) = monitor.refresh {
        table.raw_set_value(frame, "refreshRate", &refresh)?;
    }
    table.raw_set_value(frame, "primary", &monitor.primary)
}

/// `jess.gpu.monitors()`: the displays the window system has.
pub fn monitors(call: &Call<'_>) -> l3i::Result<StackResults> {
    let monitors = crate::window::monitors().map_err(|problem| error(format!("jess.gpu.monitors: {problem}")))?;
    l3i::stack::push_list(call, monitors.iter(), |frame, monitor| {
        push_table(frame, 0, 8, |frame, table| monitor_fields(frame, table, monitor))
    })?;
    Ok(StackResults)
}

const AREA_KEYS: [&str; 4] = ["height", "width", "x", "y"];

/// `jess.gpu.window(device, { title, width, height, resizable, vsync, format })`.
pub fn open(call: &Call<'_>, device: ValueView<'_>, desc: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaGpuWindow>> {
    const FUNCTION: &str = "jess.gpu.window";
    let Some(device) = receiver::<LuaGpuDevice>(device) else {
        return Err(error(format!("{FUNCTION} takes a GpuDevice, not {}", kind(device))));
    };
    let mut wanted = WindowDesc::default();
    if let Some(desc) = desc.filter(|desc| !desc.is_nil()) {
        call.with_frame(|frame| {
            let fields = options(frame, FUNCTION, "the window", desc, &WINDOW_KEYS)?;
            let title = fields.raw_get(frame, "title")?;
            if !title.is_nil() {
                if !title.is_string() {
                    return Err(error(format!("{FUNCTION}: the title is a string, not {}", kind(title))));
                }
                title.read::<&str>()?.clone_into(&mut wanted.title);
            }
            wanted.width = whole(
                FUNCTION,
                "the width",
                Some(fields.raw_get(frame, "width")?),
                Some(wanted.width),
            )?;
            wanted.height = whole(
                FUNCTION,
                "the height",
                Some(fields.raw_get(frame, "height")?),
                Some(wanted.height),
            )?;
            wanted.resizable = flag(frame, FUNCTION, &fields, "resizable", true)?;
            wanted.closes = flag(frame, FUNCTION, &fields, "closes", true)?;
            let fullscreen = fields.raw_get(frame, "fullscreen")?;
            if !fullscreen.is_nil() {
                wanted.fullscreen = fullscreen_of(FUNCTION, fullscreen)?;
            }
            let key = fields.raw_get(frame, "key")?;
            if key.is_string() {
                wanted.key = Some(key.read::<&str>()?.to_owned());
            } else if !key.is_nil() {
                return Err(error(format!("{FUNCTION}: the key is a string, not {}", kind(key))));
            }
            wanted.present = if flag(frame, FUNCTION, &fields, "vsync", true)? {
                PresentMode::Vsync
            } else {
                PresentMode::Immediate
            };
            let format = fields.raw_get(frame, "format")?;
            if format.is_string() {
                wanted.format = Some(format_of(FUNCTION, format.read::<&str>()?)?);
            } else if !format.is_nil() {
                return Err(error(format!(
                    "{FUNCTION}: the format is a format's name, not {}",
                    kind(format)
                )));
            }
            Ok(())
        })?;
    }
    let window = crate::window::open(&device.gpu, &wanted).map_err(|problem| error(format!("{FUNCTION}: {problem}")))?;
    Ok(Owned(LuaGpuWindow { window }))
}

/// The longest `jess.gpu.pump` waits for the presenter: a window that never comes ready (covered,
/// minimized) doesn't hold a script up longer than this.
const PRESENTED: std::time::Duration = std::time::Duration::from_millis(250);

/// `jess.gpu.pump()`: the window system's news handled, what the script changed committed, and
/// every window drawn with it, at the display's pace.
pub fn pump() -> l3i::Result<()> {
    crate::window::pump();
    let presented = crate::window::present();
    crate::window::wait_presented(PRESENTED);
    presented.map_err(|failures| {
        let lines: Vec<String> = failures
            .iter()
            .map(|(title, problem)| format!("window {title}: {problem}"))
            .collect();
        error(format!("jess.gpu.pump: {}", lines.join("; ")))
    })
}

/// `window:show(plan, target, imports?, sets?)`.
fn show(
    this: &LuaGpuWindow,
    call: &Call<'_>,
    plan: ValueView<'_>,
    target: ValueView<'_>,
    imports: Option<ValueView<'_>>,
    sets: Option<ValueView<'_>>,
) -> l3i::Result<()> {
    const FUNCTION: &str = "GpuWindow:show";
    let Some(plan) = receiver::<LuaGpuPlan>(plan) else {
        return Err(error(format!("{FUNCTION} takes a GpuPlan, not {}", kind(plan))));
    };
    if !target.is_string() {
        return Err(error(format!(
            "{FUNCTION}: the target is the name of the plan's imported target the window's image is, not {}",
            kind(target)
        )));
    }
    let imports = given(call, FUNCTION, imports)?
        .into_iter()
        .map(|given| match given {
            Given::Texture(name, texture) => Imported::Texture(name, texture),
            Given::Buffer(name, buffer) => Imported::Buffer(name, buffer),
        })
        .collect();
    this.window
        .show_later(Showing::new(
            Arc::clone(&plan.plan),
            super::draw_set::collection(FUNCTION, "the draw sets", sets, &plan.sets)?,
            target.read::<&str>()?.to_owned(),
            imports,
        ))
        .map_err(|problem| error(format!("{FUNCTION}: {problem}")))
}

/// `window:cursor(mode?)`: how the window holds the pointer, after doing what `mode` asks.
fn cursor(this: &LuaGpuWindow, mode: Option<ValueView<'_>>) -> l3i::Result<&'static str> {
    const FUNCTION: &str = "GpuWindow:cursor";
    let Some(mode) = mode.filter(|mode| !mode.is_nil()) else {
        return Ok(this.window.cursor().name());
    };
    let wanted = match mode.is_string().then(|| mode.read::<&str>()).transpose()? {
        Some("normal") => CursorRequest::Normal,
        Some("hidden") => CursorRequest::Hidden,
        Some("captured") => CursorRequest::Captured,
        _ => {
            return Err(error(format!(
                "{FUNCTION}: the mode is 'normal', 'hidden', or 'captured', not {}",
                shown(mode)
            )));
        }
    };
    this.window
        .set_cursor(wanted)
        .map(crate::window::Cursor::name)
        .map_err(|problem| error(format!("{FUNCTION}: {problem}")))
}

/// Declares the window type.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut window = d.userdata::<LuaGpuWindow>(LuaGpuWindow::NAME);
    window
        .tag(TagPolicy::Never)
        .doc("A native window, its surface on a GPU device, and the plan it draws every frame.");
    window.getter("title", |this: &LuaGpuWindow| this.window.title()).untyped();
    #[allow(clippy::cast_precision_loss)] // window numbers are small
    window
        .getter("number", |this: &LuaGpuWindow| this.window.number() as f64)
        .untyped();
    window.getter("key", |this: &LuaGpuWindow| this.window.key()).untyped();
    window.getter("frames", |this: &LuaGpuWindow| this.window.frames()).untyped();
    window
        .getter("width", |this: &LuaGpuWindow| this.window.size().width)
        .untyped();
    window
        .getter("height", |this: &LuaGpuWindow| this.window.size().height)
        .untyped();
    window.getter("scale", |this: &LuaGpuWindow| this.window.scale()).untyped();
    window
        .getter("format", |this: &LuaGpuWindow| this.window.format().map(format_name))
        .untyped();
    window.getter("open", |this: &LuaGpuWindow| this.window.is_open()).untyped();
    window.method("show", show).untyped();
    window
        .method("hide", |this: &LuaGpuWindow| -> l3i::Result<()> {
            this.window
                .hide_later()
                .map_err(|problem| error(format!("GpuWindow:hide: {problem}")))
        })
        .untyped();
    window.method("close", |this: &LuaGpuWindow| this.window.close()).untyped();
    describe_modes(&mut window);
}

/// Declares what a window does with the pointer and the display, what it is asked for, and how
/// it prints.
fn describe_modes(window: &mut UserdataBuilder<'_, LuaGpuWindow>) {
    window.method("cursor", cursor).untyped();
    window
        .method(
            "fullscreen",
            |this: &LuaGpuWindow, mode: Option<ValueView<'_>>| -> l3i::Result<&'static str> {
                let Some(mode) = mode.filter(|mode| !mode.is_nil()) else {
                    return Ok(this.window.fullscreen().name());
                };
                let wanted = fullscreen_of("GpuWindow:fullscreen", mode)?;
                Ok(this.window.set_fullscreen(wanted).name())
            },
        )
        .untyped();
    window
        .getter(
            "monitor",
            |this: &LuaGpuWindow, call: &Call<'_>| -> l3i::Result<StackResults> {
                match this.window.monitor() {
                    Some(monitor) => push_table(call, 0, 8, |frame, table| monitor_fields(frame, table, &monitor))?,
                    None => call.with_frame(|frame| frame.push_nil().map(drop))?,
                }
                Ok(StackResults)
            },
        )
        .untyped();
    window
        .method(
            "resize",
            |this: &LuaGpuWindow, width: ValueView<'_>, height: ValueView<'_>| -> l3i::Result<()> {
                const FUNCTION: &str = "GpuWindow:resize";
                this.window.request_size(
                    whole(FUNCTION, "the width", Some(width), None)?,
                    whole(FUNCTION, "the height", Some(height), None)?,
                );
                Ok(())
            },
        )
        .untyped();
    window
        .method(
            "textInput",
            |this: &LuaGpuWindow, call: &Call<'_>, enabled: ValueView<'_>, area: Option<ValueView<'_>>| -> l3i::Result<()> {
                const FUNCTION: &str = "GpuWindow:textInput";
                if !enabled.is_boolean() {
                    return Err(error(format!("{FUNCTION}: enabled is a boolean, not {}", kind(enabled))));
                }
                let area = match area.filter(|area| !area.is_nil()) {
                    None => None,
                    Some(area) => Some(call.with_frame(|frame| {
                        let fields = options(frame, FUNCTION, "the area", area, &AREA_KEYS)?;
                        let field = |key: &str| -> l3i::Result<u32> {
                            whole(
                                FUNCTION,
                                &format!("the area's {key}"),
                                Some(fields.raw_get(frame, key)?),
                                None,
                            )
                        };
                        Ok(TextArea {
                            x: field("x")?,
                            y: field("y")?,
                            width: field("width")?,
                            height: field("height")?,
                        })
                    })?),
                };
                this.window.text_input(enabled.read::<bool>()?, area);
                Ok(())
            },
        )
        .untyped();
    window
        .method(
            "events",
            |this: &LuaGpuWindow, call: &Call<'_>| -> l3i::Result<StackResults> {
                let events = this.window.events();
                l3i::stack::push_list(call, events.iter(), |frame, event| {
                    push_table(frame, 0, 5, |frame, table| event_fields(frame, table, event))
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    crate::lua::plan::metamethod(window, "__tostring", |this: &LuaGpuWindow| {
        let size = this.window.size();
        let closed = if this.window.is_open() { "" } else { ", closed" };
        format!("GpuWindow({}, {}×{}{closed})", this.window.title(), size.width, size.height)
    });
}

/// Fills `table` with `event`'s kind and fields, as `GpuWindow:events` lists it.
fn event_fields(frame: &Frame<'_>, table: &TableView<'_>, event: &WindowEvent) -> l3i::Result<()> {
    match event {
        WindowEvent::Resized(size) => {
            table.raw_set_value(frame, "kind", "resized")?;
            table.raw_set_value(frame, "width", &f64::from(size.width))?;
            table.raw_set_value(frame, "height", &f64::from(size.height))
        }
        WindowEvent::ScaleChanged(scale) => {
            table.raw_set_value(frame, "kind", "scaled")?;
            table.raw_set_value(frame, "scale", &scale)
        }
        WindowEvent::Focused(focused) => {
            table.raw_set_value(frame, "kind", "focused")?;
            table.raw_set_value(frame, "focused", &focused)
        }
        WindowEvent::CloseRequested => table.raw_set_value(frame, "kind", "closeRequested"),
        WindowEvent::Closed => table.raw_set_value(frame, "kind", "closed"),
        WindowEvent::Key { key, pressed, repeat } => {
            table.raw_set_value(frame, "kind", "key")?;
            table.raw_set_value(frame, "key", key.name())?;
            table.raw_set_value(frame, "pressed", pressed)?;
            table.raw_set_value(frame, "repeated", repeat)
        }
        WindowEvent::Text(text) => {
            table.raw_set_value(frame, "kind", "text")?;
            table.raw_set_value(frame, "text", text.as_str())
        }
        WindowEvent::Preedit { text, cursor } => {
            table.raw_set_value(frame, "kind", "preedit")?;
            table.raw_set_value(frame, "text", text.as_str())?;
            let Some((start, end)) = cursor else {
                return Ok(());
            };
            // As `string.sub` takes it: the first byte, and the last, which is
            // the one before the first for a caret.
            #[allow(clippy::cast_precision_loss)] // offsets into a short string
            {
                table.raw_set_value(frame, "from", &((start + 1) as f64))?;
                table.raw_set_value(frame, "to", &(*end as f64))
            }
        }
        WindowEvent::InputMethod(enabled) => {
            table.raw_set_value(frame, "kind", "inputMethod")?;
            table.raw_set_value(frame, "enabled", enabled)
        }
        WindowEvent::PointerEntered => table.raw_set_value(frame, "kind", "pointerEntered"),
        WindowEvent::PointerMoved { x, y } => {
            table.raw_set_value(frame, "kind", "pointerMoved")?;
            table.raw_set_value(frame, "x", x)?;
            table.raw_set_value(frame, "y", y)
        }
        WindowEvent::PointerLeft => table.raw_set_value(frame, "kind", "pointerLeft"),
        WindowEvent::PointerDelta { dx, dy } => {
            table.raw_set_value(frame, "kind", "pointerDelta")?;
            table.raw_set_value(frame, "dx", dx)?;
            table.raw_set_value(frame, "dy", dy)
        }
        WindowEvent::Cursor(cursor) => {
            table.raw_set_value(frame, "kind", "cursor")?;
            table.raw_set_value(frame, "mode", cursor.name())
        }
        WindowEvent::PointerButton { button, pressed } => {
            table.raw_set_value(frame, "kind", "pointerButton")?;
            table.raw_set_value(frame, "button", button.name())?;
            table.raw_set_value(frame, "pressed", pressed)
        }
        WindowEvent::Wheel { x, y } => {
            table.raw_set_value(frame, "kind", "wheel")?;
            table.raw_set_value(frame, "x", &f64::from(*x))?;
            table.raw_set_value(frame, "y", &f64::from(*y))
        }
        WindowEvent::Modifiers(held) => {
            table.raw_set_value(frame, "kind", "modifiers")?;
            table.raw_set_value(frame, "shift", &held.shift)?;
            table.raw_set_value(frame, "control", &held.control)?;
            table.raw_set_value(frame, "alt", &held.alt)?;
            table.raw_set_value(frame, "meta", &held.meta)
        }
    }
}
