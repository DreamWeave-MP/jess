//! Buffers, textures, and samplers from Luau: made on the device, written from a Luau `buffer` or
//! string without a copy on the way in, and read back as a new `buffer`.

use std::sync::Arc;

use l3i::bind::{Call, StackResults};
use l3i::convert::{BytesView, NewBuffer};
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Frame, Scope, ValueView};
use l3i::userdata::{Owned, Userdata};

use super::device::LuaGpuDevice;
use crate::gpu::{
    AddressMode, Buffer, BufferUsage, Compare, Filter, Gpu, Sampler, SamplerDesc, Texture, TextureDesc, TextureShape,
    TextureUsage,
};
use crate::lua::{check_options, error, kind};
use crate::render::TextureFormat;

/// Bytes on the GPU.
#[derive(Debug)]
pub struct LuaGpuBuffer {
    pub(crate) gpu: Arc<Gpu>,
    pub(crate) buffer: Arc<Buffer>,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuBuffer {
    const NAME: &'static str = "jess.gpu.Buffer";
}

/// An image on the GPU.
#[derive(Debug)]
pub struct LuaGpuTexture {
    pub(crate) gpu: Arc<Gpu>,
    pub(crate) texture: Arc<Texture>,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuTexture {
    const NAME: &'static str = "jess.gpu.Texture";
}

/// A way of reading textures.
#[derive(Debug)]
pub struct LuaGpuSampler {
    pub(crate) sampler: Arc<Sampler>,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuSampler {
    const NAME: &'static str = "jess.gpu.Sampler";
}

const BUFFER_USAGES: [(&str, BufferUsage); 4] = [
    ("vertex", BufferUsage::VERTEX),
    ("index", BufferUsage::INDEX),
    ("uniform", BufferUsage::UNIFORM),
    ("storage", BufferUsage::STORAGE),
];

const TEXTURE_USAGES: [(&str, TextureUsage); 2] = [("sampled", TextureUsage::SAMPLED), ("target", TextureUsage::TARGET)];

const TEXTURE_KEYS: [&str; 8] = ["depth", "format", "height", "layers", "mips", "shape", "usage", "width"];

const SAMPLER_KEYS: [&str; 4] = ["address", "compare", "filter", "mipmapFilter"];

/// A format's name in Luau: its name with the first letter lowered, `rgba8Unorm`.
pub fn format_name(format: TextureFormat) -> String {
    let name = format.name();
    let mut first = name.chars();
    first
        .next()
        .map(|letter| letter.to_ascii_lowercase().to_string() + first.as_str())
        .unwrap_or_default()
}

/// The format named `name`, or the error listing the names.
pub fn format_of(function: &str, name: &str) -> l3i::Result<TextureFormat> {
    TextureFormat::ALL
        .into_iter()
        .find(|format| format_name(*format) == name)
        .ok_or_else(|| {
            let names: Vec<String> = TextureFormat::ALL.into_iter().map(format_name).collect();
            error(format!(
                "{function}: no format {name:?}: expected one of {}",
                names.join(", ")
            ))
        })
}

/// A string, or a list of them, each one of `known`, combined.
fn usages<T: Copy + std::ops::BitOr<Output = T> + Default>(
    frame: &Frame<'_>,
    function: &str,
    view: ValueView<'_>,
    known: &[(&str, T)],
) -> l3i::Result<T> {
    let names: Vec<&str> = known.iter().map(|(name, _)| *name).collect();
    let one = |view: ValueView<'_>| -> l3i::Result<T> {
        let found = view
            .is_string()
            .then(|| view.read::<&str>().ok())
            .flatten()
            .and_then(|name| known.iter().find(|(known, _)| *known == name));
        found.map(|(_, usage)| *usage).ok_or_else(|| {
            error(format!(
                "{function}: a usage is one of {}, not {}",
                names.join(", "),
                shown(view)
            ))
        })
    };
    if !view.is_table() {
        return one(view);
    }
    let mut usage = T::default();
    view.as_table()?.for_each(frame, |_, _, value| {
        usage = usage | one(value)?;
        Ok(())
    })?;
    Ok(usage)
}

/// What a value is, as errors name it: a string in quotes, a number as Luau prints it, else its
/// kind.
pub fn shown(view: ValueView<'_>) -> String {
    if view.is_string() {
        return view
            .read::<&str>()
            .map_or_else(|_| "a string".to_owned(), |text| format!("{text:?}"));
    }
    if view.is_number() || view.is_integer() {
        return view
            .read::<f64>()
            .map_or_else(|_| "a number".to_owned(), crate::lua::lua::number_text);
    }
    kind(view).to_owned()
}

/// A whole number from 0 to `u32::MAX`, or `default` where `view` is missing or nil.
pub fn whole(function: &str, what: &str, view: Option<ValueView<'_>>, default: Option<u32>) -> l3i::Result<u32> {
    let view = view.filter(|view| !view.is_nil());
    if view.is_none()
        && let Some(default) = default
    {
        return Ok(default);
    }
    let number = view
        .filter(|view| view.is_number() || view.is_integer())
        .and_then(|view| view.read::<f64>().ok())
        .filter(|number| number.fract() == 0.0 && (0.0..=f64::from(u32::MAX)).contains(number));
    // Whole and in range, checked above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    number.map(|number| number as u32).ok_or_else(|| {
        error(format!(
            "{function}: {what} is a whole number from 0 to {}, not {}",
            u32::MAX,
            view.map_or_else(|| "nil".to_owned(), shown)
        ))
    })
}

/// A string field of `table`, one of `names`, or `default` where it is nil.
pub fn choice<'a>(
    frame: &Frame<'_>,
    function: &str,
    table: &l3i::stack::TableView<'_>,
    key: &str,
    names: &[&'a str],
    default: &'a str,
) -> l3i::Result<&'a str> {
    let view = table.raw_get(frame, key)?;
    if view.is_nil() {
        return Ok(default);
    }
    let found = view
        .is_string()
        .then(|| view.read::<&str>().ok())
        .flatten()
        .and_then(|name| names.iter().find(|known| **known == name));
    found.copied().ok_or_else(|| {
        error(format!(
            "{function}: {key} is one of {}, not {}",
            names.join(", "),
            shown(view)
        ))
    })
}

fn gpu_error(function: &str, problem: &crate::gpu::GpuError) -> l3i::Error {
    error(format!("{function}: {problem}"))
}

/// `device:buffer(size, usage)`.
fn buffer(
    this: &LuaGpuDevice,
    call: &Call<'_>,
    size: ValueView<'_>,
    usage: ValueView<'_>,
) -> l3i::Result<Owned<LuaGpuBuffer>> {
    const FUNCTION: &str = "GpuDevice:buffer";
    let size = whole(FUNCTION, "the size", Some(size), None)?;
    let usage = call.with_frame(|frame| usages(frame, FUNCTION, usage, &BUFFER_USAGES))?;
    let buffer = this
        .gpu
        .buffer(u64::from(size), usage)
        .map_err(|problem| gpu_error(FUNCTION, &problem))?;
    Ok(Owned(LuaGpuBuffer {
        gpu: this.gpu.clone(),
        buffer: Arc::new(buffer),
    }))
}

/// `device:texture { width, height, format, usage, shape, layers, depth, mips }`.
fn texture(this: &LuaGpuDevice, call: &Call<'_>, desc: ValueView<'_>) -> l3i::Result<Owned<LuaGpuTexture>> {
    const FUNCTION: &str = "GpuDevice:texture";
    if !desc.is_table() {
        return Err(error(format!("{FUNCTION} takes a table, not {}", kind(desc))));
    }
    let desc = call.with_frame(|frame| {
        check_options(frame, desc, FUNCTION, &TEXTURE_KEYS)?;
        let table = desc.as_table()?;
        let get = |key: &str| table.raw_get(frame, key);
        let format = get("format")?;
        if !format.is_string() {
            return Err(error(format!(
                "{FUNCTION}: format is a format's name, such as \"rgba8Unorm\", not {}",
                shown(format)
            )));
        }
        let shape = match choice(frame, FUNCTION, &table, "shape", &["2d", "2dArray", "cube", "3d"], "2d")? {
            "2dArray" => TextureShape::D2Array {
                layers: whole(FUNCTION, "layers", Some(get("layers")?), None)?,
            },
            "cube" => TextureShape::Cube,
            "3d" => TextureShape::D3 {
                depth: whole(FUNCTION, "depth", Some(get("depth")?), None)?,
            },
            _ => TextureShape::D2,
        };
        Ok(TextureDesc {
            width: whole(FUNCTION, "the width", Some(get("width")?), None)?,
            height: whole(FUNCTION, "the height", Some(get("height")?), None)?,
            shape,
            format: format_of(FUNCTION, format.read::<&str>()?)?,
            mips: whole(FUNCTION, "mips", Some(get("mips")?), Some(1))?,
            usage: usages(frame, FUNCTION, get("usage")?, &TEXTURE_USAGES)?,
        })
    })?;
    let texture = this.gpu.texture(&desc).map_err(|problem| gpu_error(FUNCTION, &problem))?;
    Ok(Owned(LuaGpuTexture {
        gpu: this.gpu.clone(),
        texture: Arc::new(texture),
    }))
}

/// `device:sampler { filter, mipmapFilter, address, compare }`.
fn sampler(this: &LuaGpuDevice, call: &Call<'_>, desc: Option<ValueView<'_>>) -> l3i::Result<Owned<LuaGpuSampler>> {
    const FUNCTION: &str = "GpuDevice:sampler";
    let desc = match desc.filter(|desc| !desc.is_nil()) {
        None => SamplerDesc::default(),
        Some(desc) if !desc.is_table() => {
            return Err(error(format!("{FUNCTION} takes a table, not {}", kind(desc))));
        }
        Some(desc) => call.with_frame(|frame| {
            check_options(frame, desc, FUNCTION, &SAMPLER_KEYS)?;
            let table = desc.as_table()?;
            let filter = |key: &str| -> l3i::Result<Filter> {
                Ok(
                    match choice(frame, FUNCTION, &table, key, &["nearest", "linear"], "nearest")? {
                        "linear" => Filter::Linear,
                        _ => Filter::Nearest,
                    },
                )
            };
            let address = match choice(frame, FUNCTION, &table, "address", &["clamp", "repeat", "mirror"], "clamp")? {
                "repeat" => AddressMode::Repeat,
                "mirror" => AddressMode::MirrorRepeat,
                _ => AddressMode::ClampToEdge,
            };
            let compares = [
                "never",
                "less",
                "equal",
                "lessEqual",
                "greater",
                "notEqual",
                "greaterEqual",
                "always",
            ];
            let compare = if table.raw_get(frame, "compare")?.is_nil() {
                None
            } else {
                Some(compare_of(choice(frame, FUNCTION, &table, "compare", &compares, "less")?))
            };
            Ok(SamplerDesc {
                filter: filter("filter")?,
                mipmap_filter: filter("mipmapFilter")?,
                address,
                compare,
            })
        })?,
    };
    let sampler = this.gpu.sampler(&desc).map_err(|problem| gpu_error(FUNCTION, &problem))?;
    Ok(Owned(LuaGpuSampler {
        sampler: Arc::new(sampler),
    }))
}

/// The comparison named `name`, one `choice` already checked.
pub fn compare_of(name: &str) -> Compare {
    match name {
        "never" => Compare::Never,
        "equal" => Compare::Equal,
        "lessEqual" => Compare::LessEqual,
        "greater" => Compare::Greater,
        "notEqual" => Compare::NotEqual,
        "greaterEqual" => Compare::GreaterEqual,
        "always" => Compare::Always,
        _ => Compare::Less,
    }
}

/// Bytes from a Luau `buffer` or string, borrowed, from `from` for `length` (to the end by
/// default).
fn with_bytes<R>(
    function: &str,
    bytes: ValueView<'_>,
    from: Option<ValueView<'_>>,
    length: Option<ValueView<'_>>,
    work: impl FnOnce(&[u8]) -> l3i::Result<R>,
) -> l3i::Result<R> {
    if !(bytes.is_buffer() || bytes.is_string()) {
        return Err(error(format!("{function}: expected a buffer or string, not {}", kind(bytes))));
    }
    let bytes = bytes.read::<BytesView<'_>>()?;
    // SAFETY: the slice lives only through `work`, which hands it to the GPU's queue and never
    // calls back into Luau, so nothing writes the buffer while it is borrowed.
    let all = unsafe { bytes.bytes_unchecked() };
    let size = all.len();
    let start = whole(function, "the bytes' start", from, Some(0))? as usize;
    let rest = u32::try_from(size.saturating_sub(start)).unwrap_or(u32::MAX);
    let count = whole(function, "the bytes' length", length, Some(rest))? as usize;
    let Some(chosen) = all.get(start..start.saturating_add(count)) else {
        return Err(error(format!(
            "{function}: {count} bytes from {start} run past the {size} given"
        )));
    };
    work(chosen)
}

/// Declares the device's methods that make resources.
pub fn device_methods(device: &mut UserdataBuilder<'_, LuaGpuDevice>) {
    device.method("buffer", buffer).untyped();
    device.method("texture", texture).untyped();
    device.method("sampler", sampler).untyped();
}

/// Declares the resource types.
#[allow(clippy::too_many_lines)] // one registration per method
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut buffer = d.userdata::<LuaGpuBuffer>(LuaGpuBuffer::NAME);
    buffer
        .tag(TagPolicy::Never)
        .doc("Bytes on the GPU: vertices, indices, a uniform block, or a storage buffer.");
    // Buffer sizes are far below 2^53: a Luau number holds them exactly.
    #[allow(clippy::cast_precision_loss)]
    buffer
        .getter("size", |this: &LuaGpuBuffer| this.buffer.size() as f64)
        .untyped();
    buffer
        .getter("usage", |this: &LuaGpuBuffer, call: &Call<'_>| -> l3i::Result<StackResults> {
            let names: Vec<&str> = BUFFER_USAGES
                .iter()
                .filter(|(_, usage)| this.buffer.usage().contains(*usage))
                .map(|(name, _)| *name)
                .collect();
            l3i::stack::push_list(call, names.iter(), |frame, name| frame.push_string(name).map(|_| ()))?;
            Ok(StackResults)
        })
        .untyped();
    buffer
        .method(
            "write",
            |this: &LuaGpuBuffer,
             offset: ValueView<'_>,
             bytes: ValueView<'_>,
             from: Option<ValueView<'_>>,
             length: Option<ValueView<'_>>|
             -> l3i::Result<()> {
                const FUNCTION: &str = "GpuBuffer:write";
                let offset = whole(FUNCTION, "the offset", Some(offset), None)?;
                with_bytes(FUNCTION, bytes, from, length, |bytes| {
                    this.gpu
                        .write_buffer(&this.buffer, u64::from(offset), bytes)
                        .map_err(|problem| gpu_error(FUNCTION, &problem))
                })
            },
        )
        .untyped();
    buffer
        .method(
            "read",
            |this: &LuaGpuBuffer, offset: Option<ValueView<'_>>, length: Option<ValueView<'_>>| -> l3i::Result<NewBuffer> {
                const FUNCTION: &str = "GpuBuffer:read";
                let offset = u64::from(whole(FUNCTION, "the offset", offset, Some(0))?);
                let rest = this.buffer.size().saturating_sub(offset);
                let length = whole(FUNCTION, "the length", length, Some(u32::try_from(rest).unwrap_or(u32::MAX)))?;
                this.gpu
                    .read_buffer(&this.buffer, offset, u64::from(length))
                    .map(NewBuffer)
                    .map_err(|problem| gpu_error(FUNCTION, &problem))
            },
        )
        .untyped();
    crate::lua::plan::metamethod(&mut buffer, "__tostring", |this: &LuaGpuBuffer| {
        format!("GpuBuffer({} bytes, {})", this.buffer.size(), this.buffer.usage())
    });

    let mut texture = d.userdata::<LuaGpuTexture>(LuaGpuTexture::NAME);
    texture
        .tag(TagPolicy::Never)
        .doc("An image on the GPU: sampled by shaders, drawn into, or both.");
    texture
        .getter("width", |this: &LuaGpuTexture| this.texture.desc().width)
        .untyped();
    texture
        .getter("height", |this: &LuaGpuTexture| this.texture.desc().height)
        .untyped();
    texture
        .getter("format", |this: &LuaGpuTexture| format_name(this.texture.desc().format))
        .untyped();
    texture
        .getter("mips", |this: &LuaGpuTexture| this.texture.desc().mips)
        .untyped();
    texture
        .getter("layers", |this: &LuaGpuTexture| this.texture.desc().shape.layers())
        .untyped();
    texture
        .getter("shape", |this: &LuaGpuTexture| shape_name(this.texture.desc().shape))
        .untyped();
    texture
        .method(
            "write",
            |this: &LuaGpuTexture,
             bytes: ValueView<'_>,
             level: Option<ValueView<'_>>,
             layer: Option<ValueView<'_>>|
             -> l3i::Result<()> {
                const FUNCTION: &str = "GpuTexture:write";
                let level = whole(FUNCTION, "the level", level, Some(0))?;
                let layer = whole(FUNCTION, "the layer", layer, Some(0))?;
                with_bytes(FUNCTION, bytes, None, None, |bytes| {
                    this.gpu
                        .write_texture(&this.texture, level, layer, bytes)
                        .map_err(|problem| gpu_error(FUNCTION, &problem))
                })
            },
        )
        .untyped();
    texture
        .method(
            "read",
            |this: &LuaGpuTexture, level: Option<ValueView<'_>>, layer: Option<ValueView<'_>>| -> l3i::Result<NewBuffer> {
                const FUNCTION: &str = "GpuTexture:read";
                let level = whole(FUNCTION, "the level", level, Some(0))?;
                let layer = whole(FUNCTION, "the layer", layer, Some(0))?;
                this.gpu
                    .read_texture(&this.texture, level, layer)
                    .map(NewBuffer)
                    .map_err(|problem| gpu_error(FUNCTION, &problem))
            },
        )
        .untyped();
    crate::lua::plan::metamethod(&mut texture, "__tostring", |this: &LuaGpuTexture| {
        let desc = this.texture.desc();
        format!(
            "GpuTexture({}×{} {}, {})",
            desc.width,
            desc.height,
            format_name(desc.format),
            shape_name(desc.shape)
        )
    });

    let mut sampler = d.userdata::<LuaGpuSampler>(LuaGpuSampler::NAME);
    sampler
        .tag(TagPolicy::Never)
        .doc("How shaders read a texture between and past its texels.");
    crate::lua::plan::metamethod(&mut sampler, "__tostring", |this: &LuaGpuSampler| {
        let desc = this.sampler.desc();
        let filter = |filter: Filter| match filter {
            Filter::Nearest => "nearest",
            Filter::Linear => "linear",
        };
        let compare = if desc.compare.is_some() { ", comparing" } else { "" };
        format!("GpuSampler({}{compare})", filter(desc.filter))
    });
}

fn shape_name(shape: TextureShape) -> &'static str {
    match shape {
        TextureShape::D2 => "2d",
        TextureShape::D2Array { .. } => "2dArray",
        TextureShape::Cube => "cube",
        TextureShape::D3 { .. } => "3d",
    }
}
