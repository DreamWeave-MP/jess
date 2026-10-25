//! The device handle: the runtime's one GPU, opened when a script first asks for it.

use std::cell::RefCell;
use std::rc::Rc;

use l3i::bind::{Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::userdata::{Owned, Userdata};

use crate::gpu::Gpu;
use crate::lua::{error, push_table};

/// The GPU a runtime opened, shared by every handle to it.
#[derive(Clone, Debug)]
pub struct LuaGpuDevice {
    pub(crate) gpu: Rc<Gpu>,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuDevice {
    const NAME: &'static str = "jess.gpu.Device";
}

/// The device a runtime opened, once it has.
#[derive(Clone, Debug, Default)]
pub struct Opened(Rc<RefCell<Option<Rc<Gpu>>>>);

impl Opened {
    /// The runtime's device, opened on the first call.
    pub fn device(&self) -> l3i::Result<Owned<LuaGpuDevice>> {
        let mut opened = self.0.borrow_mut();
        if let Some(gpu) = &*opened {
            return Ok(Owned(LuaGpuDevice { gpu: gpu.clone() }));
        }
        let gpu = Rc::new(Gpu::headless().map_err(|problem| error(format!("jess.gpu.device: {problem}")))?);
        *opened = Some(gpu.clone());
        Ok(Owned(LuaGpuDevice { gpu }))
    }
}

/// Declares the device type and the resources it makes.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut device = d.userdata::<LuaGpuDevice>(LuaGpuDevice::NAME);
    device
        .tag(TagPolicy::Never)
        .doc("The GPU a script draws and computes with: an adapter and the device jess opened on it.");
    device
        .getter("name", |this: &LuaGpuDevice| this.gpu.info().name.clone())
        .untyped();
    device
        .getter("backend", |this: &LuaGpuDevice| this.gpu.info().backend)
        .untyped();
    device
        .getter("kind", |this: &LuaGpuDevice| this.gpu.info().kind.name())
        .untyped();
    device
        .getter(
            "limits",
            |this: &LuaGpuDevice, call: &Call<'_>| -> l3i::Result<StackResults> {
                let info = this.gpu.info();
                push_table(call, 0, 4, |frame, table| {
                    table.raw_set_value(frame, "textureSize", &f64::from(info.max_texture_size))?;
                    table.raw_set_value(frame, "textureLayers", &f64::from(info.max_texture_layers))?;
                    // Buffer sizes are far below 2^53: a Luau number holds them exactly.
                    #[allow(clippy::cast_precision_loss)]
                    table.raw_set_value(frame, "bufferSize", &(info.max_buffer_size as f64))?;
                    table.raw_set_value(frame, "workgroups", &f64::from(info.max_workgroups))
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    super::resources::device_methods(&mut device);
    crate::lua::plan::metamethod(&mut device, "__tostring", |this: &LuaGpuDevice| {
        let info = this.gpu.info();
        format!("GpuDevice({}, {})", info.name, info.backend)
    });
    super::resources::describe(d);
}
