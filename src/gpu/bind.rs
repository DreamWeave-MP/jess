//! Bind groups: what one frequency binds for a pipeline, checked against what its program
//! declares there.

use std::sync::Arc;

use super::pipeline::{Group, Layout, Slot};
use super::{Buffer, BufferUsage, Gpu, GpuError, Sampler, Texture, TextureShape, TextureUsage};
use crate::render::SampleKind;
use crate::shader::{Frequency, ResourceId, ResourceKind, StorageId};

/// One thing a group binds.
#[derive(Clone, Copy, Debug)]
pub enum Binding<'a> {
    /// The group's uniform block.
    Uniform(&'a Buffer),
    /// A declared texture.
    Texture(ResourceId, &'a Texture),
    /// A declared sampler, its own or a texture's.
    Sampler(ResourceId, &'a Sampler),
    /// A declared storage buffer.
    Storage(StorageId, &'a Buffer),
}

/// What one frequency binds, for every pipeline whose program lays that frequency out the same.
#[derive(Debug)]
pub struct BindGroup {
    pub(crate) raw: wgpu::BindGroup,
    pub(crate) frequency: Frequency,
    layout: Arc<Layout>,
}

impl BindGroup {
    /// The frequency whose group it is.
    #[must_use]
    pub fn frequency(&self) -> Frequency {
        self.frequency
    }

    /// Whether it binds for `frequency` laid out as `layout`.
    pub(crate) fn fits(&self, frequency: Frequency, layout: &Arc<Layout>) -> bool {
        self.frequency == frequency && Arc::ptr_eq(&self.layout, layout)
    }
}

impl Gpu {
    /// What `frequency` binds for `pipeline`: exactly one binding for each of the uniform block,
    /// textures, samplers, and storage buffers its program declares there, each of the kind
    /// declared.
    ///
    /// # Errors
    ///
    /// A frequency the program binds nothing for, a declaration left unbound or bound twice, a
    /// binding the program doesn't declare there, or one of the wrong kind: a buffer without the
    /// usage or too small, a texture of another shape or sample kind, a filtering sampler for a
    /// comparison one.
    pub fn bind_group(
        &self,
        pipeline: &super::RenderPipeline,
        frequency: Frequency,
        bindings: &[Binding<'_>],
    ) -> Result<BindGroup, GpuError> {
        self.bind_group_of(&pipeline.groups[frequency as usize], bindings)
    }

    pub(crate) fn bind_group_of(&self, group: &Group, bindings: &[Binding<'_>]) -> Result<BindGroup, GpuError> {
        let frequency = group.frequency;
        let Some(layout) = &group.layout else {
            return Err(GpuError::Invalid(format!(
                "the program binds nothing per {frequency}: it needs no {frequency} bind group"
            )));
        };
        let mut entries = Vec::with_capacity(group.slots.len());
        for (number, slot, name) in &group.slots {
            let what = slot_name(*slot, name);
            let mut matching = bindings.iter().filter(|binding| answers(**binding, *slot));
            let (Some(binding), None) = (matching.next(), matching.next()) else {
                return Err(GpuError::Invalid(format!(
                    "the {frequency} bind group needs exactly one {what}"
                )));
            };
            check_binding(*binding, *slot).map_err(|expected| {
                GpuError::Invalid(format!("the {frequency} bind group's {what} is invalid: expected {expected}"))
            })?;
            entries.push(wgpu::BindGroupEntry {
                binding: *number,
                resource: resource(*binding),
            });
        }
        if let Some(extra) = bindings
            .iter()
            .find(|binding| !group.slots.iter().any(|(_, slot, _)| answers(**binding, *slot)))
        {
            return Err(GpuError::Invalid(format!(
                "the program declares no {} per {frequency}",
                binding_name(*extra)
            )));
        }
        let raw = self.checked(|| {
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &layout.raw,
                entries: &entries,
            })
        })?;
        Ok(BindGroup {
            raw,
            frequency,
            layout: layout.clone(),
        })
    }
}

/// Whether `binding` is the one for `slot`: the same declaration, ignoring kind.
fn answers(binding: Binding<'_>, slot: Slot) -> bool {
    match (binding, slot) {
        (Binding::Uniform(_), Slot::Uniform(_)) => true,
        (Binding::Texture(id, _) | Binding::Sampler(id, _), Slot::Resource(declared, _)) => id == declared,
        (Binding::Storage(id, _), Slot::Storage(declared, ..)) => id == declared,
        _ => false,
    }
}

/// Whether `binding` fits `slot`, or what was expected instead.
fn check_binding(binding: Binding<'_>, slot: Slot) -> Result<(), String> {
    let problem = match (binding, slot) {
        (Binding::Uniform(buffer), Slot::Uniform(size)) => {
            (!buffer.usage().contains(BufferUsage::UNIFORM) || buffer.size() < size).then(|| {
                format!(
                    "a uniform buffer of at least {size} bytes, not a {}-byte {} buffer",
                    buffer.size(),
                    buffer.usage()
                )
            })
        }
        (Binding::Storage(_, buffer), Slot::Storage(_, _, stride)) => {
            (!buffer.usage().contains(BufferUsage::STORAGE) || buffer.size() < u64::from(stride)).then(|| {
                format!(
                    "a storage buffer of at least one {stride}-byte element, not a {}-byte {} buffer",
                    buffer.size(),
                    buffer.usage()
                )
            })
        }
        (Binding::Texture(_, texture), Slot::Resource(_, kind)) => texture_problem(texture, kind),
        (Binding::Sampler(_, sampler), Slot::Resource(_, kind)) => {
            let comparing = sampler.desc().compare.is_some();
            match kind {
                ResourceKind::Sampler if comparing => Some("a filtering sampler, not a comparison one".to_owned()),
                ResourceKind::ComparisonSampler if !comparing => {
                    Some("a comparison sampler, not a filtering one".to_owned())
                }
                ResourceKind::Sampler | ResourceKind::ComparisonSampler => None,
                _ => Some(format!("a {} texture, not a sampler", kind.name())),
            }
        }
        _ => unreachable!("answers() paired the binding with a slot of its kind"),
    };
    problem.map_or(Ok(()), Err)
}

/// What is wrong with binding `texture` where a texture of `kind` is declared, if anything.
fn texture_problem(texture: &Texture, kind: ResourceKind) -> Option<String> {
    if kind.is_sampler() {
        return Some("a sampler, not a texture".to_owned());
    }
    let desc = texture.desc();
    let shape = match desc.shape {
        TextureShape::D2 => {
            matches!(
                kind,
                ResourceKind::Texture2D | ResourceKind::Texture2Di | ResourceKind::Texture2Du
            ) || kind == ResourceKind::DepthTexture2D
        }
        TextureShape::D2Array { .. } => matches!(kind, ResourceKind::Texture2DArray | ResourceKind::DepthTexture2DArray),
        TextureShape::Cube => matches!(kind, ResourceKind::TextureCube | ResourceKind::DepthTextureCube),
        TextureShape::D3 { .. } => kind == ResourceKind::Texture3D,
    };
    let samples = match desc.format.sample_kind() {
        SampleKind::Float => !kind.is_integer() && !kind.is_depth(),
        SampleKind::UnfilterableFloat => false,
        SampleKind::Uint => kind == ResourceKind::Texture2Du,
        SampleKind::Sint => kind == ResourceKind::Texture2Di,
        SampleKind::Depth => kind.is_depth(),
    };
    let wanted = format!("a {} texture", kind.name());
    if !shape {
        return Some(format!("{wanted}, not a {:?}", desc.shape));
    }
    if !samples {
        return Some(format!("{wanted} of a format it reads, not {}", desc.format));
    }
    if !desc.usage.contains(TextureUsage::SAMPLED) {
        return Some(format!("{wanted} made to be sampled"));
    }
    None
}

fn resource(binding: Binding<'_>) -> wgpu::BindingResource<'_> {
    match binding {
        Binding::Uniform(buffer) | Binding::Storage(_, buffer) => buffer.raw.as_entire_binding(),
        Binding::Sampler(_, sampler) => wgpu::BindingResource::Sampler(&sampler.raw),
        // A texture's own view is of its shape, which check_binding matched to the declaration.
        Binding::Texture(_, texture) => wgpu::BindingResource::TextureView(&texture.view),
    }
}

fn slot_name(slot: Slot, name: &str) -> String {
    match slot {
        Slot::Uniform(_) => "uniform block".to_owned(),
        Slot::Resource(_, kind) => format!("{} {name:?}", kind.name()),
        Slot::Storage(..) => format!("storage buffer {name:?}"),
    }
}

fn binding_name(binding: Binding<'_>) -> String {
    match binding {
        Binding::Uniform(_) => "uniform block".to_owned(),
        Binding::Texture(id, _) => format!("texture {}", id.index()),
        Binding::Sampler(id, _) => format!("sampler {}", id.index()),
        Binding::Storage(id, _) => format!("storage buffer {}", id.index()),
    }
}
