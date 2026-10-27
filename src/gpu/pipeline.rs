//! Render pipelines: a shader program and its fixed-function state, with the bind group and vertex
//! buffer layouts its declarations need, every one from the program's own layout.

use std::num::NonZeroU64;
use std::sync::{Arc, PoisonError};

use super::live::{Kind, Token};
use super::texture::raw_format;
use super::{Compare, Gpu, GpuError};
use crate::render::{SampleKind, TextureFormat};
use crate::shader::layout::{GroupLayout, VertexBufferLayout};
use crate::shader::wgsl::{self, FRAGMENT_ENTRY_POINT, VERTEX_ENTRY_POINT, WgslModule};
use crate::shader::{Access, Frequency, Program, ResourceId, ResourceKind, Scalar, StorageId, Type};

/// How a pipeline assembles vertices into primitives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Topology {
    /// Each three vertices a triangle.
    #[default]
    Triangles,
    /// Each vertex and the two before it a triangle.
    TriangleStrip,
    /// Each two vertices a line.
    Lines,
    /// Each vertex and the one before it a line.
    LineStrip,
    /// Each vertex a point.
    Points,
}

/// Which triangles a pipeline throws away by the way they face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Cull {
    /// None.
    #[default]
    None,
    /// The ones facing the camera.
    Front,
    /// The ones facing away.
    Back,
}

/// How a color output combines with what its target holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Blend {
    /// It replaces it.
    #[default]
    Replace,
    /// Over it, by the output's alpha.
    Alpha,
    /// Over it, by alpha, the output's color already multiplied by it.
    Premultiplied,
    /// Added to it.
    Additive,
}

/// A color target a pipeline draws into, by location.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorState {
    /// The target's format.
    pub format: TextureFormat,
    /// How the output combines with what the target holds.
    pub blend: Blend,
}

impl ColorState {
    /// A target of `format` the output replaces.
    #[must_use]
    pub fn new(format: TextureFormat) -> ColorState {
        ColorState {
            format,
            blend: Blend::Replace,
        }
    }
}

/// The depth target a pipeline tests against, and whether it writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DepthState {
    /// The target's depth format.
    pub format: TextureFormat,
    /// When a fragment passes: compared with what the target holds.
    pub compare: Compare,
    /// Whether a passing fragment writes its depth.
    pub write: bool,
}

/// Everything about a pipeline besides its program.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PipelineState {
    /// How vertices assemble into primitives.
    pub topology: Topology,
    /// Which triangles are thrown away.
    pub cull: Cull,
    /// Whether a triangle whose vertices run clockwise faces the camera; by default
    /// counter-clockwise ones do.
    pub clockwise: bool,
    /// The color targets, one per fragment output, by location.
    pub colors: Vec<ColorState>,
    /// The depth target, if there is one.
    pub depth: Option<DepthState>,
    /// Whether it draws into multisampled targets, 4 samples a texel.
    pub multisampled: bool,
}

impl PipelineState {
    /// Samples a texel of the targets it draws into.
    #[must_use]
    pub fn samples(&self) -> u32 {
        if self.multisampled { 4 } else { 1 }
    }
}

/// One binding of a group, as a program declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The uniform block, of so many bytes.
    Uniform(u64),
    /// A texture or sampler.
    Resource(ResourceId, ResourceKind),
    /// A storage buffer, so many bytes an element.
    Storage(StorageId, Access, u32),
}

/// What one frequency's group binds, and its layout, if it binds anything.
#[derive(Debug)]
pub struct Group {
    pub(crate) frequency: Frequency,
    /// Each binding number, what is declared there, and its name.
    pub(crate) slots: Vec<(u32, Slot, String)>,
    /// The layout, shared with every group on the device laid out alike.
    pub(crate) layout: Option<Arc<Layout>>,
}

impl Group {
    /// The layout's wgpu object, for a pipeline layout.
    pub(crate) fn raw_layout(&self) -> Option<&wgpu::BindGroupLayout> {
        self.layout.as_deref().map(|layout| &layout.raw)
    }
}

/// A bind group layout. A device makes one per distinct layout, so groups laid out alike share it
/// and a bind group made for one binds for all of them.
#[derive(Debug)]
pub struct Layout {
    pub(crate) raw: wgpu::BindGroupLayout,
}

/// A program ready to draw, with its fixed-function state.
#[derive(Debug)]
pub struct RenderPipeline {
    pub(crate) raw: wgpu::RenderPipeline,
    pub(crate) groups: [Group; 3],
    state: PipelineState,
    /// The bytes per vertex and per instance; 0 for a rate without attributes.
    pub(crate) strides: [u32; 2],
    _live: Token,
}

impl RenderPipeline {
    /// Its fixed-function state.
    #[must_use]
    pub fn state(&self) -> &PipelineState {
        &self.state
    }
}

impl Gpu {
    /// The pipeline that draws with `program` and `state`. The program's layout gives every
    /// binding and vertex attribute; `state` must have a color target per fragment output, each
    /// of a format the output fits.
    ///
    /// # Errors
    ///
    /// A compute program, a program that doesn't validate, color targets that don't fit the
    /// outputs, or a depth state without a depth format.
    pub fn render_pipeline(&self, program: &Program, state: &PipelineState) -> Result<RenderPipeline, GpuError> {
        if program.workgroup_size().is_some() {
            return Err(GpuError::Invalid(
                "a compute program draws nothing: make it a compute pipeline".to_owned(),
            ));
        }
        let module = wgsl::lower(program).map_err(|errors| invalid_program(&errors))?;
        self.render_pipeline_of(program, &module, state)
    }

    /// The pipeline of `program`, lowered to `module`, drawing with `state`.
    pub(crate) fn render_pipeline_of(
        &self,
        program: &Program,
        module: &WgslModule,
        state: &PipelineState,
    ) -> Result<RenderPipeline, GpuError> {
        check_targets(&module.outputs, state)?;
        let groups = self.groups(program, &module.layout.groups, wgpu::ShaderStages::VERTEX_FRAGMENT)?;
        let attributes: Vec<Vec<wgpu::VertexAttribute>> =
            module.layout.vertex_buffers.iter().map(vertex_attributes).collect();
        let buffers: Vec<Option<wgpu::VertexBufferLayout<'_>>> = module
            .layout
            .vertex_buffers
            .iter()
            .zip([wgpu::VertexStepMode::Vertex, wgpu::VertexStepMode::Instance])
            .zip(&attributes)
            .map(|((buffer, step_mode), attributes)| {
                (!attributes.is_empty()).then_some(wgpu::VertexBufferLayout {
                    array_stride: u64::from(buffer.stride),
                    step_mode,
                    attributes,
                })
            })
            .collect();
        let targets: Vec<Option<wgpu::ColorTargetState>> = state
            .colors
            .iter()
            .map(|color| {
                Some(wgpu::ColorTargetState {
                    format: raw_format(color.format),
                    blend: blend(color.blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })
            })
            .collect();
        let raw = self.checked(|| {
            let shader = self.device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(module.source.as_str().into()),
            });
            let layouts: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(Group::raw_layout).collect();
            let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: None,
                bind_group_layouts: &layouts,
                immediate_size: 0,
            });
            self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(VERTEX_ENTRY_POINT),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &buffers,
                },
                primitive: primitive(state),
                depth_stencil: state.depth.map(|depth| wgpu::DepthStencilState {
                    format: raw_format(depth.format),
                    depth_write_enabled: Some(depth.write),
                    depth_compare: Some(depth.compare.raw()),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: state.samples(),
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(FRAGMENT_ENTRY_POINT),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        })?;
        Ok(RenderPipeline {
            raw,
            groups,
            state: state.clone(),
            strides: module.layout.vertex_buffers.each_ref().map(|buffer| buffer.stride),
            _live: self.live.token(Kind::Pipeline),
        })
    }

    /// Each frequency's group: what the program binds there, and the layout of it, visible to
    /// `stages`.
    pub(crate) fn groups(
        &self,
        program: &Program,
        layouts: &[GroupLayout; 3],
        stages: wgpu::ShaderStages,
    ) -> Result<[Group; 3], GpuError> {
        let mut groups = Vec::with_capacity(3);
        for layout in layouts {
            let mut slots = Vec::new();
            if let Some(block) = &layout.uniform {
                slots.push((0, Slot::Uniform(u64::from(block.size)), String::new()));
            }
            for binding in &layout.resources {
                let resource = &program.resources()[binding.resource.index()];
                let slot = Slot::Resource(binding.resource, resource.kind);
                slots.push((binding.binding, slot, resource.name.clone()));
            }
            for binding in &layout.storage {
                let buffer = &program.storage_buffers()[binding.storage.index()];
                let slot = Slot::Storage(binding.storage, buffer.access, binding.stride);
                slots.push((binding.binding, slot, buffer.name.clone()));
            }
            let entries: Vec<wgpu::BindGroupLayoutEntry> = slots
                .iter()
                .map(|(binding, slot, _)| layout_entry(*binding, *slot, stages))
                .collect();
            let shared = if entries.is_empty() {
                None
            } else {
                Some(self.layout(entries)?)
            };
            groups.push(Group {
                frequency: layout.frequency,
                slots,
                layout: shared,
            });
        }
        Ok(groups.try_into().unwrap_or_else(|_| unreachable!("a group per frequency")))
    }

    /// The device's layout of `entries`, made the first time it is asked for.
    fn layout(&self, entries: Vec<wgpu::BindGroupLayoutEntry>) -> Result<Arc<Layout>, GpuError> {
        let mut layouts = self.layouts.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(layout) = layouts.get(&entries) {
            return Ok(layout.clone());
        }
        let raw = self.checked(|| {
            self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: None,
                entries: &entries,
            })
        })?;
        let layout = Arc::new(Layout { raw });
        layouts.insert(entries, layout.clone());
        drop(layouts);
        Ok(layout)
    }
}

/// A program that didn't validate, as one error listing every problem.
pub fn invalid_program(errors: &[crate::shader::ValidationError]) -> GpuError {
    let problems: Vec<String> = errors.iter().map(ToString::to_string).collect();
    GpuError::Invalid(format!("the shader program is invalid: {}", problems.join("; ")))
}

/// Whether each color target fits the output at its location, and the depth state is depth.
fn check_targets(outputs: &[Type], state: &PipelineState) -> Result<(), GpuError> {
    if outputs.len() != state.colors.len() {
        return Err(GpuError::Invalid(format!(
            "the program writes {} color outputs, and the pipeline has {} color targets: expected one per output",
            outputs.len(),
            state.colors.len()
        )));
    }
    for (location, (output, color)) in outputs.iter().zip(&state.colors).enumerate() {
        let format = color.format;
        let fits = match format.sample_kind() {
            SampleKind::Float | SampleKind::UnfilterableFloat => output.scalar() == Scalar::Float,
            SampleKind::Uint => output.scalar() == Scalar::UInt,
            SampleKind::Sint => output.scalar() == Scalar::Int,
            SampleKind::Depth => false,
        };
        if !fits || output.components() < format.channels() {
            return Err(GpuError::Invalid(format!(
                "color output {location} is a {output}, which a {format} target can't hold: expected {} of {} \
                 channels or more",
                match format.sample_kind() {
                    SampleKind::Uint => "a UInt or UInt vector",
                    SampleKind::Sint => "an Int or Int vector",
                    SampleKind::Depth => "no depth format",
                    _ => "a Float or float vector",
                },
                format.channels()
            )));
        }
        if color.blend != Blend::Replace && (format.is_uint() || format.is_sint()) {
            return Err(GpuError::Invalid(format!(
                "color target {location} blends, and a {format} target can't: integers only replace"
            )));
        }
    }
    if let Some(depth) = state.depth
        && !depth.format.is_depth()
    {
        return Err(GpuError::Invalid(format!(
            "the depth target is {}: expected a depth format",
            depth.format
        )));
    }
    Ok(())
}

fn layout_entry(binding: u32, slot: Slot, stages: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    let (ty, visibility) = match slot {
        Slot::Uniform(size) => (
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: NonZeroU64::new(size),
            },
            stages,
        ),
        Slot::Resource(_, kind) if kind.is_sampler() => {
            let ty = if kind == ResourceKind::ComparisonSampler {
                wgpu::SamplerBindingType::Comparison
            } else {
                wgpu::SamplerBindingType::Filtering
            };
            (wgpu::BindingType::Sampler(ty), stages)
        }
        Slot::Resource(_, kind) => (
            wgpu::BindingType::Texture {
                sample_type: match kind {
                    ResourceKind::Texture2Di => wgpu::TextureSampleType::Sint,
                    ResourceKind::Texture2Du => wgpu::TextureSampleType::Uint,
                    _ if kind.is_depth() => wgpu::TextureSampleType::Depth,
                    _ => wgpu::TextureSampleType::Float { filterable: true },
                },
                view_dimension: view_dimension(kind),
                multisampled: false,
            },
            stages,
        ),
        // The vertex stage may not see a buffer it could write.
        Slot::Storage(_, access, _) => (
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage {
                    read_only: access == Access::Read,
                },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            if access.writes() {
                stages - wgpu::ShaderStages::VERTEX
            } else {
                stages
            },
        ),
    };
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty,
        count: None,
    }
}

/// The view a texture of `kind` is bound through.
pub fn view_dimension(kind: ResourceKind) -> wgpu::TextureViewDimension {
    match kind {
        ResourceKind::Texture2DArray | ResourceKind::DepthTexture2DArray => wgpu::TextureViewDimension::D2Array,
        ResourceKind::TextureCube | ResourceKind::DepthTextureCube => wgpu::TextureViewDimension::Cube,
        ResourceKind::Texture3D => wgpu::TextureViewDimension::D3,
        _ => wgpu::TextureViewDimension::D2,
    }
}

/// A buffer's attributes as wgpu takes them: a value at its location, or a matrix as one column
/// per location.
fn vertex_attributes(buffer: &VertexBufferLayout) -> Vec<wgpu::VertexAttribute> {
    let mut attributes = Vec::new();
    for attribute in &buffer.attributes {
        let (format, columns, column_bytes) = match attribute.ty {
            Type::Float => (wgpu::VertexFormat::Float32, 1, 4),
            Type::Vec2f => (wgpu::VertexFormat::Float32x2, 1, 8),
            Type::Vec3f => (wgpu::VertexFormat::Float32x3, 1, 12),
            Type::Vec4f => (wgpu::VertexFormat::Float32x4, 1, 16),
            Type::Int => (wgpu::VertexFormat::Sint32, 1, 4),
            Type::Vec2i => (wgpu::VertexFormat::Sint32x2, 1, 8),
            Type::Vec3i => (wgpu::VertexFormat::Sint32x3, 1, 12),
            Type::Vec4i => (wgpu::VertexFormat::Sint32x4, 1, 16),
            Type::UInt | Type::Bool => (wgpu::VertexFormat::Uint32, 1, 4),
            Type::Vec2u => (wgpu::VertexFormat::Uint32x2, 1, 8),
            Type::Vec3u => (wgpu::VertexFormat::Uint32x3, 1, 12),
            Type::Vec4u => (wgpu::VertexFormat::Uint32x4, 1, 16),
            Type::Mat3f => (wgpu::VertexFormat::Float32x3, 3, 12),
            Type::Mat4f => (wgpu::VertexFormat::Float32x4, 4, 16),
        };
        for column in 0..columns {
            attributes.push(wgpu::VertexAttribute {
                format,
                offset: u64::from(attribute.offset + column * column_bytes),
                shader_location: attribute.location + column,
            });
        }
    }
    attributes
}

fn primitive(state: &PipelineState) -> wgpu::PrimitiveState {
    wgpu::PrimitiveState {
        topology: match state.topology {
            Topology::Triangles => wgpu::PrimitiveTopology::TriangleList,
            Topology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
            Topology::Lines => wgpu::PrimitiveTopology::LineList,
            Topology::LineStrip => wgpu::PrimitiveTopology::LineStrip,
            Topology::Points => wgpu::PrimitiveTopology::PointList,
        },
        front_face: if state.clockwise {
            wgpu::FrontFace::Cw
        } else {
            wgpu::FrontFace::Ccw
        },
        cull_mode: match state.cull {
            Cull::None => None,
            Cull::Front => Some(wgpu::Face::Front),
            Cull::Back => Some(wgpu::Face::Back),
        },
        ..Default::default()
    }
}

fn blend(blend: Blend) -> Option<wgpu::BlendState> {
    match blend {
        Blend::Replace => None,
        Blend::Alpha => Some(wgpu::BlendState::ALPHA_BLENDING),
        Blend::Premultiplied => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        Blend::Additive => {
            let add = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            };
            Some(wgpu::BlendState { color: add, alpha: add })
        }
    }
}
