//! Windows from Luau: `jess.gpu.window` opens one on a device, `window:show` gives it a plan to draw
//! every frame, and `jess.gpu.pump` handles the window system and draws, for a script no
//! application host drives.

use std::sync::Arc;

use l3i::bind::{Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::stack::{Scope, ValueView};
use l3i::userdata::{Owned, Userdata, receiver};

use super::device::LuaGpuDevice;
use super::pipeline::{flag, options};
use super::plan::{Given, LuaGpuPlan, given};
use super::resources::{format_name, format_of, whole};
use crate::gpu::PresentMode;
use crate::lua::{error, kind, push_table};
use crate::window::{Imported, Showing, Window, WindowDesc, WindowEvent};

/// A window and the device its surface is on.
#[derive(Debug)]
pub struct LuaGpuWindow {
    window: Window,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuWindow {
    const NAME: &'static str = "jess.gpu.Window";
}

const WINDOW_KEYS: [&str; 8] = ["closes", "format", "height", "key", "resizable", "title", "vsync", "width"];

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

/// Declares the window type.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut window = d.userdata::<LuaGpuWindow>(LuaGpuWindow::NAME);
    window
        .tag(TagPolicy::Never)
        .doc("A native window, its surface on a GPU device, and the plan it draws every frame.");
    window.getter("title", |this: &LuaGpuWindow| this.window.title()).untyped();
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
    window
        .method(
            "events",
            |this: &LuaGpuWindow, call: &Call<'_>| -> l3i::Result<StackResults> {
                let events = this.window.events();
                l3i::stack::push_list(call, events.iter(), |frame, event| {
                    push_table(frame, 0, 3, |frame, table| match *event {
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
                    })
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    crate::lua::plan::metamethod(&mut window, "__tostring", |this: &LuaGpuWindow| {
        let size = this.window.size();
        let closed = if this.window.is_open() { "" } else { ", closed" };
        format!("GpuWindow({}, {}×{}{closed})", this.window.title(), size.width, size.height)
    });
}
