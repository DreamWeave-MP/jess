//! The device handle: the runtime's one GPU, opened when a script first asks for it.
//!
//! Under a script domain that carries a [`SharedDevice`], the device is the domain's: opened once,
//! by whichever generation asks first, and handed to every later one, so a restart reloads code
//! without reopening the GPU. What a generation made on it (buffers, textures, pipelines, draw
//! sets) is its own, and goes when its runtime does.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use l3i::bind::{Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy};
use l3i::userdata::{Owned, Userdata};

use crate::gpu::{DrawSets, Gpu};
use crate::lua::{error, push_table};

/// The GPU a runtime opened, shared by every handle to it, and the draw sets made on it.
#[derive(Clone, Debug)]
pub struct LuaGpuDevice {
    pub(crate) gpu: Arc<Gpu>,
    pub(crate) sets: Rc<DrawSets>,
}

// SAFETY: an `Rc` to plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuDevice {
    const NAME: &'static str = "jess.gpu.Device";
}

/// The GPU a script domain's generations share: the service a host's domain carries, opened by
/// the first `jess.gpu.device()` of any generation.
#[derive(Debug, Default)]
pub struct SharedDevice(Mutex<Option<Arc<Gpu>>>);

impl SharedDevice {
    /// The domain's device, opened now if no generation has yet.
    ///
    /// # Errors
    ///
    /// As [`Gpu::headless`].
    pub fn device(&self) -> Result<Arc<Gpu>, crate::gpu::GpuError> {
        let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(gpu) = &*shared {
            return Ok(Arc::clone(gpu));
        }
        let gpu = Arc::new(Gpu::headless()?);
        *shared = Some(Arc::clone(&gpu));
        drop(shared);
        Ok(gpu)
    }

    /// The device, if a generation opened it.
    #[must_use]
    pub fn opened(&self) -> Option<Arc<Gpu>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

/// The device a runtime opened, once it has.
#[derive(Clone, Debug, Default)]
pub struct Opened(Rc<RefCell<Option<LuaGpuDevice>>>);

impl Opened {
    /// The runtime's device, opened on the first call: the domain's, under a domain that shares
    /// one, and the runtime's own otherwise. Its draw sets are the runtime's either way.
    pub fn device(&self) -> l3i::Result<Owned<LuaGpuDevice>> {
        let mut opened = self.0.borrow_mut();
        if let Some(device) = &*opened {
            return Ok(Owned(device.clone()));
        }
        let gpu = crate::runtime::service::<SharedDevice>()
            .map_or_else(|| Gpu::headless().map(Arc::new), |shared| shared.device())
            .map_err(|problem| error(format!("jess.gpu.device: {problem}")))?;
        let device = LuaGpuDevice {
            gpu,
            sets: Rc::new(DrawSets::new()),
        };
        *opened = Some(device.clone());
        Ok(Owned(device))
    }
}

/// Declares the device type, the resources it makes, and its pipelines.
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
    device
        .getter("timestamps", |this: &LuaGpuDevice| this.gpu.info().timestamps)
        .untyped();
    device
        .getter("live", |this: &LuaGpuDevice, call: &Call<'_>| -> l3i::Result<StackResults> {
            let live = this.gpu.live();
            push_table(call, 0, 5, |frame, table| {
                // Counts of live objects stay far below 2^53.
                #[allow(clippy::cast_precision_loss)]
                for (name, count) in [
                    ("buffers", live.buffers),
                    ("textures", live.textures),
                    ("samplers", live.samplers),
                    ("bindGroups", live.bind_groups),
                    ("pipelines", live.pipelines),
                ] {
                    table.raw_set_value(frame, name, &(count as f64))?;
                }
                Ok(())
            })?;
            Ok(StackResults)
        })
        .untyped();
    device
        .getter(
            "pipelines",
            |this: &LuaGpuDevice, call: &Call<'_>| -> l3i::Result<StackResults> {
                let stats = this.gpu.pipeline_stats();
                push_table(call, 0, 2, |frame, table| {
                    // Counts of compilations stay far below 2^53.
                    #[allow(clippy::cast_precision_loss)]
                    table.raw_set_value(frame, "compiled", &(stats.compiled as f64))?;
                    #[allow(clippy::cast_precision_loss)]
                    table.raw_set_value(frame, "reused", &(stats.reused as f64))
                })?;
                Ok(StackResults)
            },
        )
        .untyped();
    super::resources::device_methods(&mut device);
    super::pipeline::device_methods(&mut device);
    super::draw_set::device_methods(&mut device);
    super::plan::device_methods(&mut device);
    crate::lua::plan::metamethod(&mut device, "__tostring", |this: &LuaGpuDevice| {
        let info = this.gpu.info();
        format!("GpuDevice({}, {})", info.name, info.backend)
    });
    super::resources::describe(d);
    super::pipeline::describe(d);
    super::draw_set::describe(d);
    super::plan::describe(d);
    #[cfg(feature = "window")]
    super::window::describe(d);
}
