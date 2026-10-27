//! The GPU, through wgpu: a device, the buffers, textures, and samplers on it, and the pipelines
//! and passes that draw with them.
//!
//! Everything here is generic. It knows bytes, formats, and sizes, never what they mean: what a
//! buffer holds and why is the caller's. No public type names a wgpu type; formats are
//! [`TextureFormat`](crate::render::TextureFormat), jess's own list.
//!
//! A [`Gpu`] is one adapter and the device opened on it. [`Gpu::headless`] takes whatever adapter
//! the machine has, a software one included, which is what offscreen rendering and tests want.
//! Every call that can fail checks its arguments in Jess terms first, then runs inside a wgpu
//! error scope, so a mistake wgpu catches is a [`GpuError`] too, never a panic.

#![warn(missing_docs)]

mod bind;
mod buffer;
mod cache;
mod compute;
mod draw_set;
mod live;
mod pass;
mod pipeline;
mod plan;
mod read;
mod sampler;
#[cfg(feature = "window")]
mod surface;
mod texture;

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

pub use bind::{BindGroup, Binding};
pub use buffer::{Buffer, BufferUsage};
pub use cache::PipelineStats;
pub use compute::ComputePipeline;
pub use draw_set::{DrawSet, DrawSetDesc, DrawSets};
pub use live::LiveCounts;
pub use pass::{Draw, IndexFormat, Resolve, Target};
pub use pipeline::{Blend, ColorState, Cull, DepthState, PipelineState, RenderPipeline, Topology};
pub(crate) use pipeline::{Group, Slot};
pub use plan::{Import, PassTiming, Plan, PlanBinding, PlanGroup, Work};
pub use read::PendingRead;
pub use sampler::{AddressMode, Compare, Filter, Sampler, SamplerDesc};
#[cfg(feature = "window")]
pub use surface::{PresentMode, Surface, SurfaceFrame};
pub use texture::{Texture, TextureDesc, TextureShape, TextureUsage};

/// What went wrong on the GPU.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GpuError {
    /// The machine has no adapter wgpu can use, not even a software one.
    NoAdapter,
    /// The adapter refused to open a device.
    Device(String),
    /// A call's arguments don't fit what it acts on: an offset past a buffer's end, a texture
    /// level it doesn't have, bytes of the wrong length.
    Invalid(String),
    /// wgpu refused the work; its message, which names what it refused.
    Backend(String),
}

impl fmt::Display for GpuError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GpuError::NoAdapter => formatter.write_str("no GPU adapter, not even a software one"),
            GpuError::Device(message) => write!(formatter, "the GPU adapter opened no device: {message}"),
            GpuError::Invalid(message) => formatter.write_str(message),
            GpuError::Backend(message) => write!(formatter, "the GPU refused it: {message}"),
        }
    }
}

impl std::error::Error for GpuError {}

/// What kind of device an adapter is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeviceKind {
    /// A GPU of its own.
    Discrete,
    /// A GPU sharing the CPU's memory.
    Integrated,
    /// A virtual machine's.
    Virtual,
    /// Software rendering on the CPU.
    Cpu,
    /// Something else.
    Other,
}

impl DeviceKind {
    /// The kind's name, as `jess.gpu` reports it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            DeviceKind::Discrete => "discrete",
            DeviceKind::Integrated => "integrated",
            DeviceKind::Virtual => "virtual",
            DeviceKind::Cpu => "cpu",
            DeviceKind::Other => "other",
        }
    }
}

/// The adapter a [`Gpu`] runs on, and what it allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuInfo {
    /// The adapter's name, as its driver gives it.
    pub name: String,
    /// The API wgpu reaches it through: `vulkan`, `metal`, `dx12`, or `gl`.
    pub backend: &'static str,
    /// What kind of device it is.
    pub kind: DeviceKind,
    /// The widest or tallest a 2D texture may be.
    pub max_texture_size: u32,
    /// The most layers an array texture may have.
    pub max_texture_layers: u32,
    /// The largest buffer, in bytes.
    pub max_buffer_size: u64,
    /// The most workgroups a dispatch may have on one axis.
    pub max_workgroups: u32,
    /// Whether it times passes: a plan's [`Gpu::timings`] say how long each took.
    pub timestamps: bool,
}

/// The process's wgpu instances: one of the primary backends (Vulkan, Metal, Direct3D 12), and one
/// of OpenGL for a machine with none. The Vulkan loader isn't safe against instances made and
/// dropped on several threads at once (it crashed jess's tests in
/// `vkEnumerateInstanceExtensionProperties`), so every device opens through these.
static PRIMARY: OnceLock<wgpu::Instance> = OnceLock::new();
static FALLBACK: OnceLock<wgpu::Instance> = OnceLock::new();

/// Held while a device opens: OpenGL's contexts belong to a thread, and enumerating its adapters
/// from two threads at once fails in EGL with `BadAccess`.
static OPENING: Mutex<()> = Mutex::new(());

/// An instance of `backends`, unless `WGPU_BACKEND` names others.
fn instance(backends: wgpu::Backends) -> wgpu::Instance {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    wgpu::Instance::new(descriptor.with_env())
}

/// One adapter and the device opened on it.
pub struct Gpu {
    /// The process-wide instance the adapter was found through, which surfaces are made on.
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    instance: &'static wgpu::Instance,
    #[cfg_attr(not(feature = "window"), allow(dead_code))]
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    info: GpuInfo,
    /// Every bind group layout the device has made, by its entries.
    layouts: Mutex<HashMap<Vec<wgpu::BindGroupLayoutEntry>, Arc<pipeline::Layout>>>,
    /// The pipelines it compiled, by what they were compiled from.
    cache: Mutex<cache::Cache>,
    /// How many of each object it has alive.
    live: Arc<live::Live>,
}

impl fmt::Debug for Gpu {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Gpu")
            .field("info", &self.info)
            .finish_non_exhaustive()
    }
}

impl Gpu {
    /// Opens a device on whatever adapter the machine has: a hardware one if there is one, else
    /// a software one. Nothing is drawn to a window; everything is offscreen.
    ///
    /// # Errors
    ///
    /// No adapter at all, or one that opens no device.
    pub fn headless() -> Result<Gpu, GpuError> {
        let _opening = OPENING.lock().unwrap_or_else(PoisonError::into_inner);
        let mut found = None;
        'search: for (instance, backends) in [(&PRIMARY, wgpu::Backends::PRIMARY), (&FALLBACK, wgpu::Backends::GL)] {
            let instance: &'static wgpu::Instance = instance.get_or_init(|| self::instance(backends));
            for force_fallback_adapter in [false, true] {
                let options = wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    force_fallback_adapter,
                    compatible_surface: None,
                    ..Default::default()
                };
                if let Ok(adapter) = block_on(instance.request_adapter(&options)) {
                    found = Some((instance, adapter));
                    break 'search;
                }
            }
        }
        let (instance, adapter) = found.ok_or(GpuError::NoAdapter)?;
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let descriptor = wgpu::DeviceDescriptor {
            required_features: if timestamps {
                wgpu::Features::TIMESTAMP_QUERY
            } else {
                wgpu::Features::empty()
            },
            ..wgpu::DeviceDescriptor::default()
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&descriptor)).map_err(|error| GpuError::Device(error.to_string()))?;
        // An error no scope catches would panic in wgpu's default handler; every call here opens a
        // scope, so this only ever sees what slips past one, and drops it rather than abort.
        device.on_uncaptured_error(Arc::new(|_| {}));
        let adapter_info = adapter.get_info();
        let limits = device.limits();
        let info = GpuInfo {
            name: adapter_info.name,
            backend: backend_name(adapter_info.backend),
            kind: match adapter_info.device_type {
                wgpu::DeviceType::DiscreteGpu => DeviceKind::Discrete,
                wgpu::DeviceType::IntegratedGpu => DeviceKind::Integrated,
                wgpu::DeviceType::VirtualGpu => DeviceKind::Virtual,
                wgpu::DeviceType::Cpu => DeviceKind::Cpu,
                wgpu::DeviceType::Other => DeviceKind::Other,
            },
            max_texture_size: limits.max_texture_dimension_2d,
            max_texture_layers: limits.max_texture_array_layers,
            max_buffer_size: limits.max_buffer_size,
            max_workgroups: limits.max_compute_workgroups_per_dimension,
            timestamps,
        };
        Ok(Gpu {
            instance,
            adapter,
            device,
            queue,
            info,
            layouts: Mutex::default(),
            cache: Mutex::default(),
            live: Arc::default(),
        })
    }

    /// How many buffers, textures, samplers, bind groups, and pipelines it has alive: what was
    /// made on it and not yet dropped.
    #[must_use]
    pub fn live(&self) -> LiveCounts {
        self.live.counts()
    }

    /// How many buffers, textures, samplers, bind groups, and pipelines were ever made on it,
    /// dropped or not. What grows every frame of a scene that doesn't change is churn.
    #[must_use]
    pub fn made(&self) -> LiveCounts {
        self.live.made()
    }

    /// The adapter this runs on, and what it allows.
    #[must_use]
    pub fn info(&self) -> &GpuInfo {
        &self.info
    }

    /// Runs `work` inside error scopes for validation and memory, turning what they catch into a
    /// [`GpuError::Backend`].
    fn checked<T>(&self, work: impl FnOnce() -> T) -> Result<T, GpuError> {
        let memory = self.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let value = work();
        let caught = block_on(validation.pop()).or_else(|| block_on(memory.pop()));
        if let Some(error) = caught {
            return Err(GpuError::Backend(error.to_string()));
        }
        Ok(value)
    }

    /// Copies `size` bytes of `source` from `offset` into a buffer the CPU can read, and reads it.
    pub(crate) fn read_back(
        &self,
        size: u64,
        copy: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer),
    ) -> Result<Vec<u8>, GpuError> {
        self.read_later(size, None, copy)?.wait().map(<[u8]>::to_vec)
    }
}

/// What `jess.gpu` calls the API wgpu reaches an adapter through.
fn backend_name(backend: wgpu::Backend) -> &'static str {
    match backend {
        wgpu::Backend::Vulkan => "vulkan",
        wgpu::Backend::Metal => "metal",
        wgpu::Backend::Dx12 => "dx12",
        wgpu::Backend::Gl => "gl",
        wgpu::Backend::BrowserWebGpu => "webgpu",
        wgpu::Backend::Noop => "none",
    }
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    pollster::block_on(future)
}
