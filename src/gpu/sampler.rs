//! Samplers: how a shader reads a texture between and outside its texels.

use super::live::{Kind, Token};
use super::{Gpu, GpuError};

/// How a sampler blends neighbouring texels, or mip levels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// The nearest one.
    #[default]
    Nearest,
    /// Linearly between the nearest.
    Linear,
}

/// What a sampler reads past a texture's edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AddressMode {
    /// The edge texel.
    #[default]
    ClampToEdge,
    /// The texture again, tiled.
    Repeat,
    /// The texture again, mirrored at each edge.
    MirrorRepeat,
}

/// How a comparison sampler compares a reference with what it reads, or a depth test a fragment's
/// depth with the target's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Compare {
    /// Never passes.
    Never,
    /// Passes when the reference is less.
    Less,
    /// Passes when they are equal.
    Equal,
    /// Passes when the reference is less or equal.
    LessEqual,
    /// Passes when the reference is greater.
    Greater,
    /// Passes when they differ.
    NotEqual,
    /// Passes when the reference is greater or equal.
    GreaterEqual,
    /// Always passes.
    Always,
}

impl Compare {
    pub(crate) fn raw(self) -> wgpu::CompareFunction {
        match self {
            Compare::Never => wgpu::CompareFunction::Never,
            Compare::Less => wgpu::CompareFunction::Less,
            Compare::Equal => wgpu::CompareFunction::Equal,
            Compare::LessEqual => wgpu::CompareFunction::LessEqual,
            Compare::Greater => wgpu::CompareFunction::Greater,
            Compare::NotEqual => wgpu::CompareFunction::NotEqual,
            Compare::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
            Compare::Always => wgpu::CompareFunction::Always,
        }
    }
}

/// What a sampler does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SamplerDesc {
    /// Between texels, when the texture is drawn larger or smaller than it is.
    pub filter: Filter,
    /// Between mip levels.
    pub mipmap_filter: Filter,
    /// Past each edge, the same on every axis.
    pub address: AddressMode,
    /// A comparison sampler's comparison; `None` for a filtering one.
    pub compare: Option<Compare>,
}

/// A way of reading textures.
#[derive(Debug)]
pub struct Sampler {
    pub(crate) raw: wgpu::Sampler,
    desc: SamplerDesc,
    _live: Token,
}

impl Sampler {
    /// What it does.
    #[must_use]
    pub fn desc(&self) -> &SamplerDesc {
        &self.desc
    }
}

impl Gpu {
    /// A sampler of `desc`.
    ///
    /// # Errors
    ///
    /// What the device refuses.
    pub fn sampler(&self, desc: &SamplerDesc) -> Result<Sampler, GpuError> {
        let filter = |filter: Filter| match filter {
            Filter::Nearest => wgpu::FilterMode::Nearest,
            Filter::Linear => wgpu::FilterMode::Linear,
        };
        let address = match desc.address {
            AddressMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
            AddressMode::Repeat => wgpu::AddressMode::Repeat,
            AddressMode::MirrorRepeat => wgpu::AddressMode::MirrorRepeat,
        };
        let raw = self.checked(|| {
            self.device.create_sampler(&wgpu::SamplerDescriptor {
                label: None,
                address_mode_u: address,
                address_mode_v: address,
                address_mode_w: address,
                mag_filter: filter(desc.filter),
                min_filter: filter(desc.filter),
                mipmap_filter: match desc.mipmap_filter {
                    Filter::Nearest => wgpu::MipmapFilterMode::Nearest,
                    Filter::Linear => wgpu::MipmapFilterMode::Linear,
                },
                compare: desc.compare.map(Compare::raw),
                ..Default::default()
            })
        })?;
        Ok(Sampler {
            raw,
            desc: *desc,
            _live: self.live.token(Kind::Sampler),
        })
    }
}
