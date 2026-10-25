//! Render passes: draws into color and depth targets, each checked against its pipeline.

use super::{BindGroup, Buffer, BufferUsage, Gpu, GpuError, RenderPipeline, Texture, TextureUsage};
use crate::render::{ClearValue, LoadOp, StoreOp};
use crate::shader::Frequency;

/// A level of a layer of a texture a pass draws into, and what it starts and ends with.
#[derive(Clone, Copy, Debug)]
pub struct Target<'a> {
    /// The texture, made to be a target.
    pub texture: &'a Texture,
    /// The mip level drawn into.
    pub level: u32,
    /// The layer drawn into: an array's layer or a cube's face; 0 otherwise.
    pub layer: u32,
    /// What it holds when the pass starts.
    pub load: LoadOp,
    /// Whether what the pass drew is kept.
    pub store: StoreOp,
}

impl<'a> Target<'a> {
    /// Level 0 of layer 0 of `texture`, cleared to `clear` and stored.
    #[must_use]
    pub fn cleared(texture: &'a Texture, clear: ClearValue) -> Target<'a> {
        Target {
            texture,
            level: 0,
            layer: 0,
            load: LoadOp::Clear(clear),
            store: StoreOp::Store,
        }
    }
}

/// How wide each index of an index buffer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IndexFormat {
    /// Two bytes: at most 65 535 vertices.
    U16,
    /// Four bytes.
    U32,
}

/// One draw: a pipeline, what each frequency binds for it, its buffers, and how much to draw.
#[derive(Clone, Copy, Debug)]
pub struct Draw<'a> {
    /// The pipeline.
    pub pipeline: &'a RenderPipeline,
    /// The pass's, the material's, and the draw's bind groups, for each its program binds
    /// anything for.
    pub groups: [Option<&'a BindGroup>; 3],
    /// The per-vertex attributes, if the program declares any.
    pub vertices: Option<&'a Buffer>,
    /// The per-instance attributes, if the program declares any.
    pub instances: Option<&'a Buffer>,
    /// The indices of the vertices to draw, if the draw is indexed.
    pub indices: Option<(&'a Buffer, IndexFormat)>,
    /// How many vertices, or indices, to draw.
    pub count: u32,
    /// How many instances.
    pub instance_count: u32,
    /// Where the counts come from instead, written by the GPU: the buffer, and the offset of
    /// four `u32`s in it (vertex count, instance count, first vertex, first instance) or, for an
    /// indexed draw, five (index count, instance count, first index, base vertex, first instance).
    /// `count` and `instance_count` are then ignored, and nothing checks the counts against the
    /// buffers: the GPU keeps a draw past their ends inside them.
    pub indirect: Option<(&'a Buffer, u64)>,
}

impl<'a> Draw<'a> {
    /// `count` vertices of `pipeline`, once, with nothing bound.
    #[must_use]
    pub fn new(pipeline: &'a RenderPipeline, count: u32) -> Draw<'a> {
        Draw {
            pipeline,
            groups: [None; 3],
            vertices: None,
            instances: None,
            indices: None,
            count,
            instance_count: 1,
            indirect: None,
        }
    }
}

impl Gpu {
    /// Draws `draws`, in order, into `colors` (by location) and `depth`, and submits the pass.
    ///
    /// # Errors
    ///
    /// No target, a target that isn't one or hasn't the level or layer, a clear value of the
    /// wrong kind, or a draw whose pipeline's targets aren't the pass's, whose bind groups or
    /// buffers are missing or laid out for another program, or whose count runs past its
    /// vertices.
    pub fn render(&self, colors: &[Target<'_>], depth: Option<&Target<'_>>, draws: &[Draw<'_>]) -> Result<(), GpuError> {
        check_pass(colors, depth, false, draws)?;
        self.checked(|| {
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            encode(&mut encoder, colors, depth, false, draws, None);
            self.queue.submit([encoder.finish()]);
        })
    }
}

/// Whether a pass of `draws` into `colors` and `depth` can be drawn, as
/// [`render`](Gpu::render) checks it. A pass that only tests against `depth` (`read_only`) draws
/// nothing that writes it.
pub fn check_pass(
    colors: &[Target<'_>],
    depth: Option<&Target<'_>>,
    read_only: bool,
    draws: &[Draw<'_>],
) -> Result<(), GpuError> {
    if colors.is_empty() && depth.is_none() {
        return Err(GpuError::Invalid(
            "a pass draws into nothing: expected a color or depth target".to_owned(),
        ));
    }
    for (position, target) in colors.iter().chain(depth).enumerate() {
        check_target(target, position == colors.len())?;
    }
    for (index, draw) in draws.iter().enumerate() {
        check_draw(index, draw, colors, depth)?;
        if read_only && draw.pipeline.state().depth.is_some_and(|depth| depth.write) {
            return Err(GpuError::Invalid(format!(
                "draw {index} is invalid: its pipeline writes depth, and the pass only tests against its depth target"
            )));
        }
    }
    Ok(())
}

/// Records a pass [`check_pass`] accepted into `encoder`, with its start and end timestamps at
/// `timed`'s query and the next, if it is timed.
pub fn encode(
    encoder: &mut wgpu::CommandEncoder,
    colors: &[Target<'_>],
    depth: Option<&Target<'_>>,
    read_only: bool,
    draws: &[Draw<'_>],
    timed: Option<(&wgpu::QuerySet, u32)>,
) {
    let views: Vec<wgpu::TextureView> = colors.iter().chain(depth).map(target_view).collect();
    let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = colors
        .iter()
        .zip(&views)
        .map(|(target, view)| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match target.load {
                        LoadOp::Clear(clear) => wgpu::LoadOp::Clear(clear_color(clear)),
                        LoadOp::Load => wgpu::LoadOp::Load,
                        LoadOp::DontCare => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    },
                    store: store(target.store),
                },
            })
        })
        .collect();
    let depth_attachment = depth.map(|target| wgpu::RenderPassDepthStencilAttachment {
        view: &views[colors.len()],
        depth_ops: (!read_only).then(|| wgpu::Operations {
            load: match target.load {
                LoadOp::Clear(ClearValue::Depth(depth)) => wgpu::LoadOp::Clear(depth),
                LoadOp::Load => wgpu::LoadOp::Load,
                _ => wgpu::LoadOp::Clear(1.0),
            },
            store: store(target.store),
        }),
        stencil_ops: None,
    });
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: None,
        color_attachments: &attachments,
        depth_stencil_attachment: depth_attachment,
        timestamp_writes: timed.map(|(query_set, first)| wgpu::RenderPassTimestampWrites {
            query_set,
            beginning_of_pass_write_index: Some(first),
            end_of_pass_write_index: Some(first + 1),
        }),
        occlusion_query_set: None,
        multiview_mask: None,
    });
    for draw in draws {
        pass.set_pipeline(&draw.pipeline.raw);
        for (group, bound) in (0..).zip(draw.groups) {
            if let Some(bound) = bound {
                pass.set_bind_group(group, &bound.raw, &[]);
            }
        }
        for (slot, buffer) in (0..).zip([draw.vertices, draw.instances]) {
            if let Some(buffer) = buffer {
                pass.set_vertex_buffer(slot, buffer.raw.slice(..));
            }
        }
        match draw.indices {
            Some((buffer, format)) => {
                let format = match format {
                    IndexFormat::U16 => wgpu::IndexFormat::Uint16,
                    IndexFormat::U32 => wgpu::IndexFormat::Uint32,
                };
                pass.set_index_buffer(buffer.raw.slice(..), format);
                match draw.indirect {
                    Some((arguments, offset)) => pass.draw_indexed_indirect(&arguments.raw, offset),
                    None => pass.draw_indexed(0..draw.count, 0, 0..draw.instance_count),
                }
            }
            None => match draw.indirect {
                Some((arguments, offset)) => pass.draw_indirect(&arguments.raw, offset),
                None => pass.draw(0..draw.count, 0..draw.instance_count),
            },
        }
    }
}

/// Whether `target` can be drawn into: a target texture with the level and layer, and a clear
/// value of its format's kind.
fn check_target(target: &Target<'_>, is_depth: bool) -> Result<(), GpuError> {
    let desc = target.texture.desc();
    let format = desc.format;
    let problem = if !desc.usage.contains(TextureUsage::TARGET) {
        Some("a texture made to be a target".to_owned())
    } else if format.is_depth() != is_depth {
        Some(if is_depth { "a depth format" } else { "a color format" }.to_owned())
    } else if target.level >= desc.mips || target.layer >= desc.shape.layers() {
        Some(format!("one of its {} levels and {} layers", desc.mips, desc.shape.layers()))
    } else if let LoadOp::Clear(clear) = target.load
        && !(clear.fits(format) && clear.is_valid())
    {
        Some(format!("a clear value for {format}, not {clear:?}"))
    } else {
        None
    };
    let Some(expected) = problem else {
        return Ok(());
    };
    let what = if is_depth { "depth target" } else { "color target" };
    Err(GpuError::Invalid(format!(
        "a {what} of {format}, level {} of layer {}, is invalid: expected {expected}",
        target.level, target.layer
    )))
}

fn check_draw(index: usize, draw: &Draw<'_>, colors: &[Target<'_>], depth: Option<&Target<'_>>) -> Result<(), GpuError> {
    let pipeline = draw.pipeline;
    let state = pipeline.state();
    let formats: Vec<_> = colors.iter().map(|target| target.texture.desc().format).collect();
    let wanted: Vec<_> = state.colors.iter().map(|color| color.format).collect();
    let problem = if formats != wanted {
        Some(format!("its pipeline draws into {wanted:?}, and the pass has {formats:?}"))
    } else if state.depth.map(|depth| depth.format) != depth.map(|target| target.texture.desc().format) {
        Some("its pipeline's depth target and the pass's differ".to_owned())
    } else if draw.instance_count == 0 && draw.indirect.is_none() {
        Some("at least one instance".to_owned())
    } else {
        group_problem(draw, &Frequency::ALL).or_else(|| buffer_problem(draw))
    };
    let Some(problem) = problem else {
        return Ok(());
    };
    Err(GpuError::Invalid(format!("draw {index} is invalid: {problem}")))
}

/// What is wrong with a draw's bind groups for `frequencies`, if anything.
pub fn group_problem(draw: &Draw<'_>, frequencies: &[Frequency]) -> Option<String> {
    for &frequency in frequencies {
        let (group, bound) = (&draw.pipeline.groups[frequency as usize], draw.groups[frequency as usize]);
        match (&group.layout, bound) {
            (Some(_), None) => return Some(format!("its program binds a {frequency} group, and the draw gives none")),
            (None, Some(_)) => {
                return Some(format!(
                    "its program binds nothing per {frequency}, and the draw gives a group"
                ));
            }
            (Some(layout), Some(bound)) if !bound.fits(frequency, layout) => {
                return Some(format!("its {frequency} bind group was made for another frequency or layout"));
            }
            _ => {}
        }
    }
    None
}

/// What is wrong with a draw's buffers, if anything. An indirect draw's counts are the GPU's, so
/// only its buffers' usages and its argument buffer's size are checked.
pub fn buffer_problem(draw: &Draw<'_>) -> Option<String> {
    let counted = draw.indirect.is_none();
    if let Some((buffer, offset)) = draw.indirect {
        let needed = if draw.indices.is_some() { 20 } else { 16 };
        if !buffer.usage().contains(BufferUsage::INDIRECT) {
            return Some("its indirect buffer isn't one".to_owned());
        }
        if !offset.is_multiple_of(4) || buffer.size() < offset + needed {
            return Some(format!(
                "its indirect arguments take {needed} bytes from {offset}, a multiple of 4, in its {}-byte \
                 buffer",
                buffer.size()
            ));
        }
    }
    let [vertex_stride, instance_stride] = draw.pipeline.strides;
    for (name, plural, buffer, stride, count) in [
        (
            "vertex",
            "vertices",
            draw.vertices,
            vertex_stride,
            if draw.indices.is_some() { 0 } else { draw.count },
        ),
        ("instance", "instances", draw.instances, instance_stride, draw.instance_count),
    ] {
        match buffer {
            None if stride > 0 => {
                return Some(format!(
                    "its program has {name} attributes, and the draw gives no {name} buffer"
                ));
            }
            Some(_) if stride == 0 => {
                return Some(format!(
                    "its program has no {name} attributes, and the draw gives a {name} buffer"
                ));
            }
            Some(buffer) if !buffer.usage().contains(BufferUsage::VERTEX) => {
                return Some(format!("its {name} buffer isn't a vertex buffer"));
            }
            Some(buffer) if counted && buffer.size() < u64::from(stride) * u64::from(count) => {
                return Some(format!(
                    "{count} {plural} of {stride} bytes run past its {}-byte {name} buffer",
                    buffer.size()
                ));
            }
            _ => {}
        }
    }
    let (buffer, format) = draw.indices?;
    let width = match format {
        IndexFormat::U16 => 2,
        IndexFormat::U32 => 4,
    };
    if !buffer.usage().contains(BufferUsage::INDEX) {
        return Some("its index buffer isn't one".to_owned());
    }
    (counted && buffer.size() < width * u64::from(draw.count)).then(|| {
        format!(
            "{} indices of {width} bytes run past its {}-byte index buffer",
            draw.count,
            buffer.size()
        )
    })
}

/// A view of exactly the level and layer a target draws into.
fn target_view(target: &Target<'_>) -> wgpu::TextureView {
    target.texture.raw.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_mip_level: target.level,
        mip_level_count: Some(1),
        base_array_layer: target.layer,
        array_layer_count: Some(1),
        ..Default::default()
    })
}

fn clear_color(clear: ClearValue) -> wgpu::Color {
    let color = |[r, g, b, a]: [f64; 4]| wgpu::Color { r, g, b, a };
    match clear {
        ClearValue::Color(values) => color(values.map(f64::from)),
        ClearValue::Uint(values) => color(values.map(f64::from)),
        ClearValue::Sint(values) => color(values.map(f64::from)),
        ClearValue::Depth(depth) => color([f64::from(depth); 4]),
        ClearValue::Zero => wgpu::Color::TRANSPARENT,
    }
}

fn store(op: StoreOp) -> wgpu::StoreOp {
    match op {
        StoreOp::Store => wgpu::StoreOp::Store,
        StoreOp::Discard => wgpu::StoreOp::Discard,
    }
}
