//! Textures: images on the GPU, uploaded from bytes, drawn into, and read back.

use std::ops::BitOr;
use std::sync::OnceLock;

use super::live::{Kind, Token};
use super::{Gpu, GpuError};
use crate::render::TextureFormat;

/// A texture's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextureShape {
    /// One 2D image.
    D2,
    /// Layers of 2D images of one size.
    D2Array {
        /// How many layers.
        layers: u32,
    },
    /// Six square 2D faces, in the order +x, -x, +y, -y, +z, -z.
    Cube,
    /// A volume of `depth` slices.
    D3 {
        /// How many slices.
        depth: u32,
    },
}

impl TextureShape {
    /// How many 2D layers it holds: 1, its layers, 6 faces, or 1 for a volume.
    #[must_use]
    pub fn layers(self) -> u32 {
        match self {
            TextureShape::D2 | TextureShape::D3 { .. } => 1,
            TextureShape::D2Array { layers } => layers,
            TextureShape::Cube => 6,
        }
    }

    fn depth(self) -> u32 {
        match self {
            TextureShape::D3 { depth } => depth,
            other => other.layers(),
        }
    }

    fn name(self) -> &'static str {
        match self {
            TextureShape::D2 => "2D texture",
            TextureShape::D2Array { .. } => "2D array texture",
            TextureShape::Cube => "cube texture",
            TextureShape::D3 { .. } => "3D texture",
        }
    }
}

/// What a texture is for, any number at once. Every texture can be written and read back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextureUsage(u8);

impl TextureUsage {
    /// A shader samples or loads it.
    pub const SAMPLED: TextureUsage = TextureUsage(1);
    /// A pass draws into it: a color or depth target.
    pub const TARGET: TextureUsage = TextureUsage(2);

    /// Whether this includes every usage `other` does.
    #[must_use]
    pub fn contains(self, other: TextureUsage) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for TextureUsage {
    type Output = TextureUsage;

    fn bitor(self, other: TextureUsage) -> TextureUsage {
        TextureUsage(self.0 | other.0)
    }
}

/// What a texture is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    /// Its width in texels.
    pub width: u32,
    /// Its height in texels.
    pub height: u32,
    /// One image, layers, a cube, or a volume.
    pub shape: TextureShape,
    /// The format of every texel.
    pub format: TextureFormat,
    /// How many mip levels, the first the full size and each after it half the last.
    pub mips: u32,
    /// Samples a texel: 1, or 4 for a multisampled 2D target of one level, which a pass draws
    /// into and resolves and nothing samples, copies, writes, or reads back.
    pub samples: u32,
    /// What it is for.
    pub usage: TextureUsage,
}

impl TextureDesc {
    /// A 2D texture of one mip level.
    #[must_use]
    pub fn d2(width: u32, height: u32, format: TextureFormat, usage: TextureUsage) -> TextureDesc {
        TextureDesc {
            width,
            height,
            shape: TextureShape::D2,
            format,
            mips: 1,
            samples: 1,
            usage,
        }
    }

    /// The size of mip `level`: each level half the last, never below one texel.
    fn level_size(&self, level: u32) -> (u32, u32, u32) {
        let shrink = |size: u32| (size >> level).max(1);
        let depth = match self.shape {
            TextureShape::D3 { depth } => shrink(depth),
            _ => 1,
        };
        (shrink(self.width), shrink(self.height), depth)
    }
}

/// An image on the GPU.
#[derive(Debug)]
pub struct Texture {
    pub(crate) raw: wgpu::Texture,
    /// The view of all of it, of its shape, as shaders read it.
    pub(crate) view: wgpu::TextureView,
    /// A 2D view of each level of each layer, level by level, made the first time one is asked
    /// for and kept.
    levels: Box<[OnceLock<wgpu::TextureView>]>,
    desc: TextureDesc,
    _live: Token,
}

impl Texture {
    /// What it is.
    #[must_use]
    pub fn desc(&self) -> &TextureDesc {
        &self.desc
    }

    /// A 2D view of `level` of `layer`, which it has: what a pass draws into, and what a shader
    /// reads one level of one layer through. Made once.
    pub(crate) fn level_view(&self, level: u32, layer: u32) -> &wgpu::TextureView {
        let layers = self.desc.shape.layers();
        self.levels[(level * layers + layer) as usize].get_or_init(|| {
            self.raw.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_mip_level: level,
                mip_level_count: Some(1),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            })
        })
    }

    fn levels(desc: &TextureDesc) -> Box<[OnceLock<wgpu::TextureView>]> {
        // A volume's levels aren't 2D, so it has none of these.
        let layers = match desc.shape {
            TextureShape::D3 { .. } => 0,
            shape => shape.layers(),
        };
        (0..desc.mips * layers).map(|_| OnceLock::new()).collect()
    }
}

/// A row of a copy into a buffer starts at a multiple of 256 bytes.
const ROW_ALIGNMENT: u32 = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

#[cfg(feature = "window")]
impl Texture {
    /// `raw`, a texture wgpu made elsewhere (a surface's image), described as `desc`.
    pub(crate) fn wrap(gpu: &Gpu, raw: wgpu::Texture, desc: TextureDesc) -> Texture {
        let view = raw.create_view(&wgpu::TextureViewDescriptor::default());
        Texture {
            raw,
            view,
            levels: Texture::levels(&desc),
            desc,
            _live: gpu.live.token(Kind::Texture),
        }
    }
}

impl Gpu {
    /// A texture of `desc`, zeroed.
    ///
    /// # Errors
    ///
    /// A size of 0 or past the device's limits, a cube that isn't square, more mip levels than
    /// the size halves to, a depth format on a volume, no usage, or a sample count other than 1
    /// or 4, or 4 on anything but a 2D target of one level, or of a format the device can't
    /// multisample.
    pub fn texture(&self, desc: &TextureDesc) -> Result<Texture, GpuError> {
        check_desc(self, desc)?;
        let dimension = match desc.shape {
            TextureShape::D3 { .. } => wgpu::TextureDimension::D3,
            _ => wgpu::TextureDimension::D2,
        };
        let mut usage = wgpu::TextureUsages::empty();
        // A multisampled texture is only drawn into and resolved.
        if desc.samples == 1 {
            usage |= wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::COPY_SRC;
        }
        if desc.usage.contains(TextureUsage::SAMPLED) {
            usage |= wgpu::TextureUsages::TEXTURE_BINDING;
        }
        if desc.usage.contains(TextureUsage::TARGET) {
            usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;
        }
        let view_dimension = match desc.shape {
            TextureShape::D2 => wgpu::TextureViewDimension::D2,
            TextureShape::D2Array { .. } => wgpu::TextureViewDimension::D2Array,
            TextureShape::Cube => wgpu::TextureViewDimension::Cube,
            TextureShape::D3 { .. } => wgpu::TextureViewDimension::D3,
        };
        let (raw, view) = self.checked(|| {
            let raw = self.device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: desc.width,
                    height: desc.height,
                    depth_or_array_layers: desc.shape.depth(),
                },
                mip_level_count: desc.mips,
                sample_count: desc.samples,
                dimension,
                format: raw_format(desc.format),
                usage,
                view_formats: &[],
            });
            let view = raw.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(view_dimension),
                ..Default::default()
            });
            (raw, view)
        })?;
        Ok(Texture {
            raw,
            view,
            levels: Texture::levels(desc),
            desc: *desc,
            _live: self.live.token(Kind::Texture),
        })
    }

    /// Writes all of mip `level` of `layer` (a cube's face, an array's layer; 0 for a 2D texture
    /// or a volume, whose level is written whole) from `bytes`, rows top to bottom, tightly
    /// packed.
    ///
    /// # Errors
    ///
    /// A level or layer the texture doesn't have, bytes that aren't exactly the level's, or a
    /// depth format, which only a pass writes.
    pub fn write_texture(&self, texture: &Texture, level: u32, layer: u32, bytes: &[u8]) -> Result<(), GpuError> {
        let desc = &texture.desc;
        single_sample(desc)?;
        let texel = writable(desc)?;
        let (width, height, depth) = check_level(desc, level, layer)?;
        let expected = u64::from(width) * u64::from(height) * u64::from(depth) * u64::from(texel);
        if bytes.len() as u64 != expected {
            return Err(GpuError::Invalid(format!(
                "{} bytes don't fill level {level} of a {width}×{height} {}: expected {expected}",
                bytes.len(),
                desc.format
            )));
        }
        self.checked(|| {
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture.raw,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * texel),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: depth,
                },
            );
        })
    }

    /// Reads all of mip `level` of `layer`, as [`write_texture`](Gpu::write_texture) writes it,
    /// once everything submitted so far is done.
    ///
    /// # Errors
    ///
    /// A level or layer the texture doesn't have, or `Depth24Plus`, whose layout is the
    /// backend's.
    pub fn read_texture(&self, texture: &Texture, level: u32, layer: u32) -> Result<Vec<u8>, GpuError> {
        let desc = &texture.desc;
        single_sample(desc)?;
        let Some(texel) = desc.format.texel_bytes() else {
            return Err(GpuError::Invalid(format!(
                "a {} texture can't be read back: its layout is the GPU's own",
                desc.format
            )));
        };
        let (width, height, depth) = check_level(desc, level, layer)?;
        let row = width * texel;
        let padded = row.next_multiple_of(ROW_ALIGNMENT);
        let size = u64::from(padded) * u64::from(height) * u64::from(depth);
        let aspect = if desc.format.is_depth() {
            wgpu::TextureAspect::DepthOnly
        } else {
            wgpu::TextureAspect::All
        };
        let bytes = self.read_back(size, |encoder, staging| {
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture.raw,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(padded),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: depth,
                },
            );
        })?;
        Ok(bytes
            .chunks(padded as usize)
            .flat_map(|line| &line[..row as usize])
            .copied()
            .collect())
    }
}

fn check_desc(gpu: &Gpu, desc: &TextureDesc) -> Result<(), GpuError> {
    let limit = gpu.info.max_texture_size;
    let shape = desc.shape.name();
    let problem = if desc.width == 0 || desc.height == 0 || desc.width > limit || desc.height > limit {
        Some(format!("each side from 1 to {limit}"))
    } else if desc.shape == TextureShape::Cube && desc.width != desc.height {
        Some("square faces".to_owned())
    } else if desc.shape.depth() == 0 || desc.shape.layers() > gpu.info.max_texture_layers {
        Some(format!("from 1 to {} layers", gpu.info.max_texture_layers))
    } else if desc.mips == 0 || desc.mips > 32 - desc.width.max(desc.height).leading_zeros() {
        Some("from 1 mip level to as many as its size halves to".to_owned())
    } else if desc.format.is_depth() && matches!(desc.shape, TextureShape::D3 { .. }) {
        Some("a color format: depth isn't a volume".to_owned())
    } else if desc.usage.contains(TextureUsage::TARGET) && matches!(desc.shape, TextureShape::D3 { .. }) {
        Some("to be sampled only: a pass draws into 2D layers, not a volume".to_owned())
    } else if desc.usage == TextureUsage::default() {
        Some("a usage: sampled, a target, or both".to_owned())
    } else if !matches!(desc.samples, 1 | 4) {
        Some(format!("1 or 4 samples a texel, not {}", desc.samples))
    } else if desc.samples > 1 && (desc.shape != TextureShape::D2 || desc.mips > 1 || desc.usage != TextureUsage::TARGET) {
        Some("one 2D level, made only to be a target, being multisampled".to_owned())
    } else if desc.samples > 1 && !gpu.multisamples(desc.format) {
        Some(format!("a format the device multisamples, not {}", desc.format))
    } else {
        None
    };
    let Some(expected) = problem else {
        return Ok(());
    };
    Err(GpuError::Invalid(format!(
        "a {}×{} {} {shape} is invalid: expected {expected}",
        desc.width, desc.height, desc.format
    )))
}

impl Gpu {
    /// Whether the device draws into `format` four samples a texel.
    pub(crate) fn multisamples(&self, format: TextureFormat) -> bool {
        raw_format(format)
            .guaranteed_format_features(self.device.features())
            .flags
            .sample_count_supported(4)
    }
}

/// Whether the CPU can write or read `desc`'s texels: not a multisampled texture's.
fn single_sample(desc: &TextureDesc) -> Result<(), GpuError> {
    if desc.samples == 1 {
        return Ok(());
    }
    Err(GpuError::Invalid(format!(
        "a multisampled {} texture can't be written or read back: resolve it into one that can",
        desc.format
    )))
}

/// The size of `level` of `desc`, if it has that level and `layer`.
fn check_level(desc: &TextureDesc, level: u32, layer: u32) -> Result<(u32, u32, u32), GpuError> {
    if level >= desc.mips || layer >= desc.shape.layers() {
        return Err(GpuError::Invalid(format!(
            "level {level} of layer {layer} is invalid in a {} of {} levels and {} layers",
            desc.shape.name(),
            desc.mips,
            desc.shape.layers()
        )));
    }
    Ok(desc.level_size(level))
}

/// The bytes a texel of a texture the CPU may write takes.
fn writable(desc: &TextureDesc) -> Result<u32, GpuError> {
    match desc.format.texel_bytes() {
        Some(texel) if !desc.format.is_depth() => Ok(texel),
        _ => Err(GpuError::Invalid(format!(
            "a {} texture can't be written from the CPU: only a pass writes depth",
            desc.format
        ))),
    }
}

/// The wgpu format of `format`.
pub fn raw_format(format: TextureFormat) -> wgpu::TextureFormat {
    match format {
        TextureFormat::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        TextureFormat::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        TextureFormat::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        TextureFormat::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
        TextureFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        TextureFormat::R8Unorm => wgpu::TextureFormat::R8Unorm,
        TextureFormat::Rg8Unorm => wgpu::TextureFormat::Rg8Unorm,
        TextureFormat::R16Float => wgpu::TextureFormat::R16Float,
        TextureFormat::Rg16Float => wgpu::TextureFormat::Rg16Float,
        TextureFormat::R32Float => wgpu::TextureFormat::R32Float,
        TextureFormat::Rg32Float => wgpu::TextureFormat::Rg32Float,
        TextureFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        TextureFormat::R32Uint => wgpu::TextureFormat::R32Uint,
        TextureFormat::Rgba8Uint => wgpu::TextureFormat::Rgba8Uint,
        TextureFormat::Rgba32Uint => wgpu::TextureFormat::Rgba32Uint,
        TextureFormat::R32Sint => wgpu::TextureFormat::R32Sint,
        TextureFormat::Rgba32Sint => wgpu::TextureFormat::Rgba32Sint,
        TextureFormat::Depth24Plus => wgpu::TextureFormat::Depth24Plus,
        TextureFormat::Depth32Float => wgpu::TextureFormat::Depth32Float,
    }
}
