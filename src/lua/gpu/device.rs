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
use l3i::stack::{Scope, ValueView};
use l3i::userdata::{Owned, Userdata};

use super::pipeline::options;
use super::resources::shown;
use crate::gpu::{DrawSets, Gpu, Power};
use crate::lua::{error, push_table};

/// The GPU a runtime opened, shared by every handle to it, and the draw sets made on it.
#[derive(Clone, Debug)]
pub struct LuaGpuDevice {
    pub(crate) gpu: Arc<Gpu>,
    pub(crate) sets: Arc<DrawSets>,
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
    /// The domain's device, opened now on the adapter `power` prefers if no generation has opened
    /// it yet. Once it is open, whichever adapter it is on is the one every generation gets.
    ///
    /// # Errors
    ///
    /// As [`Gpu::headless`].
    pub fn device(&self, power: Power) -> Result<Arc<Gpu>, crate::gpu::GpuError> {
        let mut shared = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(gpu) = &*shared {
            return Ok(Arc::clone(gpu));
        }
        let gpu = Arc::new(Gpu::headless_with(power)?);
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
    /// The runtime's device, opened on the first call on the adapter `power` prefers (low power
    /// without one): the domain's, under a domain that shares one, and the runtime's own
    /// otherwise. Its draw sets are the runtime's either way. Asking an open device for another
    /// adapter than it was opened on is an error, not a quiet second best.
    pub fn device(&self, power: Option<Power>) -> l3i::Result<Owned<LuaGpuDevice>> {
        let mut opened = self.0.borrow_mut();
        if let Some(device) = &*opened {
            return same(device.gpu.as_ref(), power).map(|()| Owned(device.clone()));
        }
        let wanted = power.unwrap_or_default();
        let gpu = crate::runtime::service::<SharedDevice>()
            .map_or_else(|| Gpu::headless_with(wanted).map(Arc::new), |shared| shared.device(wanted))
            .map_err(|problem| error(format!("jess.gpu.device: {problem}")))?;
        // A domain's device may have been opened by an earlier generation, on another adapter.
        same(gpu.as_ref(), power)?;
        let device = LuaGpuDevice {
            gpu,
            sets: Arc::new(DrawSets::new()),
        };
        *opened = Some(device.clone());
        Ok(Owned(device))
    }
}

/// The adapter `jess.gpu.device`'s options ask for, if they ask: `{ power = 'low' | 'high' }`.
pub fn power(call: &Call<'_>, given: Option<ValueView<'_>>) -> l3i::Result<Option<Power>> {
    const FUNCTION: &str = "jess.gpu.device";
    let Some(given) = given.filter(|given| !given.is_nil()) else {
        return Ok(None);
    };
    call.with_frame(|frame| {
        let fields = options(frame, FUNCTION, "its argument", given, &["power"])?;
        let power = fields.raw_get(frame, "power")?;
        if power.is_nil() {
            return Ok(None);
        }
        let name = if power.is_string() { power.read::<&str>().ok() } else { None };
        name.and_then(Power::named)
            .map(Some)
            .ok_or_else(|| error(format!("{FUNCTION}: power is 'low' or 'high', not {}", shown(power))))
    })
}

/// Whether `gpu` is on the adapter `power` asks for: nothing asked is always so.
fn same(gpu: &Gpu, power: Option<Power>) -> l3i::Result<()> {
    let info = gpu.info();
    match power {
        Some(power) if power != info.power => Err(error(format!(
            "jess.gpu.device: the device is already open on the {}-power adapter ({}), so it can't be {}-power: \
             ask for power on the first call, before anything opens it",
            info.power.name(),
            info.name,
            power.name()
        ))),
        _ => Ok(()),
    }
}

/// `device.memory`: the bytes its live buffers and textures hold.
fn memory(this: &LuaGpuDevice, call: &Call<'_>) -> l3i::Result<StackResults> {
    let held = this.gpu.memory();
    push_table(call, 0, 2, |frame, table| {
        // A device holds far fewer than 2^53 bytes.
        #[allow(clippy::cast_precision_loss)]
        {
            table.raw_set_value(frame, "buffers", &(held.buffers as f64))?;
            table.raw_set_value(frame, "textures", &(held.textures as f64))
        }
    })?;
    Ok(StackResults)
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
        .getter("power", |this: &LuaGpuDevice| this.gpu.info().power.name())
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
        .getter("indirectFirstInstance", |this: &LuaGpuDevice| {
            this.gpu.info().indirect_first_instance
        })
        .untyped();
    device
        .getter("baseVertex", |this: &LuaGpuDevice| this.gpu.info().base_vertex)
        .untyped();
    device
        .getter("live", |this: &LuaGpuDevice, call: &Call<'_>| counts(call, this.gpu.live()))
        .untyped();
    device
        .getter("made", |this: &LuaGpuDevice, call: &Call<'_>| counts(call, this.gpu.made()))
        .untyped();
    device.getter("memory", memory).untyped();
    device
        .method("commit", |this: &LuaGpuDevice| -> l3i::Result<bool> {
            this.gpu
                .commit()
                .map_err(|problem| super::pipeline::gpu_error("GpuDevice:commit", &problem))
        })
        .untyped();
    device
        .getter("room", |this: &LuaGpuDevice, call: &Call<'_>| -> l3i::Result<StackResults> {
            let room = this.gpu.room();
            push_table(call, 0, 2, |frame, table| {
                // The room left is at most a few hundred million: well below 2^53.
                #[allow(clippy::cast_precision_loss)]
                {
                    table.raw_set_value(frame, "changes", &(room.changes as f64))?;
                    table.raw_set_value(frame, "bytes", &(room.bytes as f64))
                }
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

/// `{ buffers, textures, samplers, bindGroups, pipelines }` of `counts`.
fn counts(call: &Call<'_>, counts: crate::gpu::LiveCounts) -> l3i::Result<StackResults> {
    push_table(call, 0, 5, |frame, table| {
        // Counts of objects stay far below 2^53.
        #[allow(clippy::cast_precision_loss)]
        for (name, count) in [
            ("buffers", counts.buffers),
            ("textures", counts.textures),
            ("samplers", counts.samplers),
            ("bindGroups", counts.bind_groups),
            ("pipelines", counts.pipelines),
        ] {
            table.raw_set_value(frame, name, &(count as f64))?;
        }
        Ok(())
    })?;
    Ok(StackResults)
}
