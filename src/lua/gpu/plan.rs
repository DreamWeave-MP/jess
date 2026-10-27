//! Plans from Luau: a render graph described as a table, its targets, buffers, and passes and what
//! each pass does, compiled for the device and a viewport, and run whole, every frame, from
//! `plan:run`.

use std::sync::Arc;

use l3i::bind::{Call, StackResults};
use l3i::convert::NewBuffer;
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata, receiver};

use super::device::LuaGpuDevice;
use super::draw_set::collection;
use super::pipeline::{
    LuaGpuBindGroup, Pipeline, clear_value, flag, gpu_error, number, options, pipeline_field, slot_name, table, workgroups,
};
use super::resources::{LuaGpuBuffer, LuaGpuRead, LuaGpuSampler, LuaGpuTexture, format_of, shown, whole};
use crate::gpu::{DrawSets, Gpu, Group, Import, Plan, PlanBinding, PlanGroup, Slot, Work};
use crate::lua::{check_options, error, kind, push_table};
use crate::render::{
    BufferSize, ClearValue, CopyAt, CopyRegion, CopySize, Extent, LoadOp, Pass, PassKind, PassMetadata, PassOutput,
    ReadUsage, RenderGraph, ResourceDesc, ResourceKind, ResourceOrigin, StoreOp, Subresource, TargetDesc, TargetSize,
};
use crate::shader::Frequency;

/// A plan compiled from Luau, and the device and draw sets it runs with.
#[derive(Debug)]
pub struct LuaGpuPlan {
    pub gpu: Arc<Gpu>,
    pub sets: Arc<DrawSets>,
    pub plan: Plan,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuPlan {
    const NAME: &'static str = "jess.gpu.Plan";
}

const FUNCTION: &str = "GpuDevice:plan";

const PLAN_KEYS: [&str; 5] = ["buffers", "name", "passes", "targets", "viewport"];

const TARGET_KEYS: [&str; 10] = [
    "format", "height", "imported", "kept", "layers", "mips", "name", "samples", "scale", "width",
];

const BUFFER_KEYS: [&str; 6] = ["image", "imported", "kept", "name", "scale", "size"];

const WRITE_KEYS: [&str; 7] = ["clear", "layer", "level", "load", "resolve", "store", "target"];

const LEVEL_KEYS: [&str; 3] = ["layer", "level", "target"];

const END_KEYS: [&str; 7] = ["buffer", "column", "layer", "level", "offset", "row", "target"];

const STORAGE_KEYS: [&str; 4] = ["buffer", "clear", "load", "store"];

const DRAW_KEYS: [&str; 10] = [
    "colors",
    "depth",
    "depthTest",
    "kind",
    "name",
    "owner",
    "pass",
    "pipeline",
    "reads",
    "tags",
];

const FULLSCREEN_KEYS: [&str; 11] = [
    "colors",
    "depth",
    "depthTest",
    "draw",
    "kind",
    "material",
    "name",
    "owner",
    "pass",
    "pipeline",
    "reads",
];

const COMPUTE_KEYS: [&str; 10] = [
    "draw",
    "kind",
    "material",
    "name",
    "owner",
    "pass",
    "pipeline",
    "reads",
    "storage",
    "workgroups",
];

const COPY_KEYS: [&str; 7] = ["from", "kind", "load", "name", "owner", "size", "to"];

const KINDS: [&str; 4] = ["draw", "fullscreen", "compute", "copy"];

/// A string field.
fn text(frame: &Frame<'_>, what: &str, table: &TableView<'_>, key: &str) -> l3i::Result<Option<String>> {
    let view = table.raw_get(frame, key)?;
    if view.is_nil() {
        return Ok(None);
    }
    if !view.is_string() {
        return Err(error(format!("{FUNCTION}: {what}'s {key} is a string, not {}", shown(view))));
    }
    Ok(Some(view.read::<&str>()?.to_owned()))
}

/// A string field that must be there.
fn required(frame: &Frame<'_>, what: &str, table: &TableView<'_>, key: &str) -> l3i::Result<String> {
    text(frame, what, table, key)?.ok_or_else(|| error(format!("{FUNCTION}: {what} needs a {key}")))
}

/// Every string of a list.
fn names(frame: &Frame<'_>, what: &str, view: ValueView<'_>) -> l3i::Result<Vec<String>> {
    let mut names = Vec::new();
    if view.is_nil() {
        return Ok(names);
    }
    table(FUNCTION, what, view)?.for_each_array(frame, |_, _, name| {
        if !name.is_string() {
            return Err(error(format!("{FUNCTION}: {what} holds strings, not {}", shown(name))));
        }
        names.push(name.read::<&str>()?.to_owned());
        Ok(())
    })?;
    Ok(names)
}

fn graph_error(problem: &crate::render::GraphError) -> l3i::Error {
    error(format!("{FUNCTION}: {problem}"))
}

/// Registers each of `targets`: `{ name, format, scale | width and height, mips, layers,
/// samples, imported }`.
fn add_targets(frame: &Frame<'_>, graph: &mut RenderGraph, targets: ValueView<'_>) -> l3i::Result<()> {
    if targets.is_nil() {
        return Ok(());
    }
    table(FUNCTION, "targets", targets)?.for_each_array(frame, |frame, index, target| {
        let what = format!("target {index}");
        let fields = options(frame, FUNCTION, &what, target, &TARGET_KEYS)?;
        let name = required(frame, &what, &fields, "name")?;
        let format = format_of(FUNCTION, &required(frame, &what, &fields, "format")?)?;
        let (width, height) = (fields.raw_get(frame, "width")?, fields.raw_get(frame, "height")?);
        let scale = fields.raw_get(frame, "scale")?;
        let size = if width.is_nil() && height.is_nil() {
            // A target's scale is a small factor; f32 holds it as the graph needs it.
            #[allow(clippy::cast_possible_truncation)]
            let scale = if scale.is_nil() {
                1.0
            } else {
                number(FUNCTION, "a scale", scale)? as f32
            };
            TargetSize::Relative { scale }
        } else if scale.is_nil() {
            TargetSize::Absolute {
                width: whole(FUNCTION, "a width", Some(width), None)?,
                height: whole(FUNCTION, "a height", Some(height), None)?,
            }
        } else {
            return Err(error(format!(
                "{FUNCTION}: {what} has a scale and a width and height: expected one or the other"
            )));
        };
        let count = |key: &str| -> l3i::Result<u32> { whole(FUNCTION, key, Some(fields.raw_get(frame, key)?), Some(1)) };
        let target = TargetDesc::new(format, size)
            .mips(count("mips")?)
            .layers(count("layers")?)
            .samples(count("samples")?);
        let origin = origin(frame, &what, &fields)?;
        graph
            .add(name, ResourceDesc::Texture(target), origin)
            .map(drop)
            .map_err(|problem| graph_error(&problem))
    })
}

/// Who owns a target or buffer: the graph, unless it is `imported` or `kept`.
fn origin(frame: &Frame<'_>, what: &str, fields: &TableView<'_>) -> l3i::Result<ResourceOrigin> {
    match (
        flag(frame, FUNCTION, fields, "imported", false)?,
        flag(frame, FUNCTION, fields, "kept", false)?,
    ) {
        (false, false) => Ok(ResourceOrigin::Transient),
        (true, false) => Ok(ResourceOrigin::Imported),
        (false, true) => Ok(ResourceOrigin::Kept),
        (true, true) => Err(error(format!(
            "{FUNCTION}: {what} is imported and kept: the run gives it, or the plan keeps it"
        ))),
    }
}

/// Registers each of `buffers`: `{ name, size | image and scale, imported, kept }`.
fn add_buffers(frame: &Frame<'_>, graph: &mut RenderGraph, buffers: ValueView<'_>) -> l3i::Result<()> {
    if buffers.is_nil() {
        return Ok(());
    }
    table(FUNCTION, "buffers", buffers)?.for_each_array(frame, |frame, index, buffer| {
        let what = format!("buffer {index}");
        let fields = options(frame, FUNCTION, &what, buffer, &BUFFER_KEYS)?;
        let name = required(frame, &what, &fields, "name")?;
        let size = fields.raw_get(frame, "size")?;
        let size = match text(frame, &what, &fields, "image")? {
            Some(format) if size.is_nil() => {
                let scale = fields.raw_get(frame, "scale")?;
                // A buffer's scale is a small factor; f32 holds it as the graph needs it.
                #[allow(clippy::cast_possible_truncation)]
                let scale = if scale.is_nil() {
                    1.0
                } else {
                    number(FUNCTION, "a scale", scale)? as f32
                };
                BufferSize::Image {
                    format: format_of(FUNCTION, &format)?,
                    scale,
                }
            }
            Some(_) => {
                return Err(error(format!(
                    "{FUNCTION}: {what} has a size and an image: expected one or the other"
                )));
            }
            None => BufferSize::Bytes(bytes(FUNCTION, "a buffer's size", size)?),
        };
        let origin = origin(frame, &what, &fields)?;
        graph
            .add(name, ResourceDesc::Buffer(size), origin)
            .map(drop)
            .map_err(|problem| graph_error(&problem))
    })
}

/// The id of the graph's resource called `name`, which pass `pass` uses.
fn resource(graph: &RenderGraph, pass: &str, name: &str) -> l3i::Result<crate::render::ResourceId> {
    graph.resource_id(name).ok_or_else(|| {
        error(format!(
            "{FUNCTION}: pass {pass} uses {name}, which the plan has no target or buffer called"
        ))
    })
}

/// The `level` and `layer` of `fields`, or `None` where it names neither.
fn level_of(frame: &Frame<'_>, fields: &TableView<'_>) -> l3i::Result<Option<Subresource>> {
    let (level, layer) = (fields.raw_get(frame, "level")?, fields.raw_get(frame, "layer")?);
    if level.is_nil() && layer.is_nil() {
        return Ok(None);
    }
    Ok(Some(Subresource {
        level: whole(FUNCTION, "a level", Some(level), Some(0))?,
        layer: whole(FUNCTION, "a layer", Some(layer), Some(0))?,
    }))
}

/// A target and maybe one level of one layer of it: a name, or `{ target, level, layer }`.
fn named_level(
    frame: &Frame<'_>,
    graph: &RenderGraph,
    pass: &str,
    what: &str,
    view: ValueView<'_>,
) -> l3i::Result<(crate::render::ResourceId, Option<Subresource>)> {
    if view.is_string() {
        return Ok((resource(graph, pass, view.read::<&str>()?)?, None));
    }
    let fields = options(frame, FUNCTION, &format!("{what}, a name or"), view, &LEVEL_KEYS)?;
    let name = required(frame, what, &fields, "target")?;
    Ok((resource(graph, pass, &name)?, level_of(frame, &fields)?))
}

/// A write: a resource's name, or `{ target | buffer, clear, load, store, level, layer, resolve
/// }`. A target clears to what `clear` gives; a buffer, to zero, when `clear` is `true`.
fn output(
    frame: &Frame<'_>,
    graph: &RenderGraph,
    pass: &str,
    view: ValueView<'_>,
    buffer: bool,
    attachment: fn(crate::render::ResourceId, LoadOp, StoreOp) -> PassOutput,
) -> l3i::Result<PassOutput> {
    let key = if buffer { "buffer" } else { "target" };
    if view.is_string() {
        let id = resource(graph, pass, view.read::<&str>()?)?;
        return Ok(attachment(id, LoadOp::DontCare, StoreOp::Store));
    }
    let what = format!("pass {pass}'s write");
    let fields = options(
        frame,
        FUNCTION,
        &format!("{what}, a name or"),
        view,
        if buffer { &STORAGE_KEYS } else { &WRITE_KEYS },
    )?;
    let name = required(frame, &what, &fields, key)?;
    let id = resource(graph, pass, &name)?;
    let clear = fields.raw_get(frame, "clear")?;
    let load = flag(frame, FUNCTION, &fields, "load", false)?;
    let load = match (clear.is_nil() || clear.is_boolean() && !clear.read::<bool>()?, load) {
        (true, false) => LoadOp::DontCare,
        (true, true) => LoadOp::Load,
        (false, true) => {
            return Err(error(format!(
                "{FUNCTION}: {what} clears {name} and loads it: expected one or the other"
            )));
        }
        (false, false) => match graph.resources()[id.index()].desc {
            ResourceDesc::Buffer { .. } if clear.is_boolean() => LoadOp::Clear(ClearValue::Zero),
            ResourceDesc::Buffer { .. } => {
                return Err(error(format!(
                    "{FUNCTION}: {what} clears the buffer {name} with {}: a buffer's clear is true, to zero",
                    shown(clear)
                )));
            }
            ResourceDesc::Texture(target) => LoadOp::Clear(clear_value(frame, FUNCTION, &what, target.format, clear)?),
        },
    };
    let store = if flag(frame, FUNCTION, &fields, "store", true)? {
        StoreOp::Store
    } else {
        StoreOp::Discard
    };
    let mut written = attachment(id, load, store);
    if buffer {
        return Ok(written);
    }
    if let Some(level) = level_of(frame, &fields)? {
        written = written.at(level.level, level.layer);
    }
    let resolve = fields.raw_get(frame, "resolve")?;
    if !resolve.is_nil() {
        let (into, level) = named_level(frame, graph, pass, &format!("{what}'s resolve"), resolve)?;
        let level = level.unwrap_or_default();
        written = written.resolving(into, level.level, level.layer);
    }
    Ok(written)
}

/// One end of a copy: a resource's name, `{ target, level, layer, column, row }`, or `{ buffer, offset
/// }`. The resource, the level of the layer it copies, and where in it the copy starts.
fn copy_end(
    frame: &Frame<'_>,
    graph: &RenderGraph,
    pass: &str,
    what: &str,
    view: ValueView<'_>,
) -> l3i::Result<(crate::render::ResourceId, Option<Subresource>, Option<CopyAt>)> {
    if view.is_string() {
        return Ok((resource(graph, pass, view.read::<&str>()?)?, None, None));
    }
    let fields = options(frame, FUNCTION, &format!("{what}, a name or"), view, &END_KEYS)?;
    let field = |key: &str| fields.raw_get(frame, key);
    if let Some(name) = text(frame, what, &fields, "buffer")? {
        for key in ["target", "level", "layer", "column", "row"] {
            if !field(key)?.is_nil() {
                return Err(error(format!("{FUNCTION}: {what} is a buffer, which has no {key}")));
            }
        }
        let offset = field("offset")?;
        let at = if offset.is_nil() {
            None
        } else {
            Some(CopyAt::Byte(bytes(FUNCTION, "an offset", offset)?))
        };
        return Ok((resource(graph, pass, &name)?, None, at));
    }
    let name = required(frame, what, &fields, "target")?;
    if !field("offset")?.is_nil() {
        return Err(error(format!(
            "{FUNCTION}: {what} is a target, which starts at a column and row, not an offset"
        )));
    }
    let (column, row) = (field("column")?, field("row")?);
    let at = if column.is_nil() && row.is_nil() {
        None
    } else {
        Some(CopyAt::Texel {
            x: whole(FUNCTION, "the column", Some(column), Some(0))?,
            y: whole(FUNCTION, "the row", Some(row), Some(0))?,
        })
    };
    Ok((resource(graph, pass, &name)?, level_of(frame, &fields)?, at))
}

/// Whole bytes: a non-negative integer.
fn bytes(function: &str, what: &str, view: ValueView<'_>) -> l3i::Result<u64> {
    let value = number(function, what, view)?;
    if value.fract() != 0.0 || value < 0.0 {
        return Err(error(format!("{function}: {what} is whole bytes, not {value}")));
    }
    // Whole and non-negative, so the cast is exact below 2^53.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as u64)
}

/// How much a copy copies: `{ width, height }` texels, or a number of bytes.
fn copy_size(frame: &Frame<'_>, what: &str, view: ValueView<'_>) -> l3i::Result<Option<CopySize>> {
    if view.is_nil() {
        return Ok(None);
    }
    if !view.is_table() {
        return Ok(Some(CopySize::Bytes(bytes(FUNCTION, &format!("{what}'s size"), view)?)));
    }
    let fields = options(
        frame,
        FUNCTION,
        &format!("{what}'s size, bytes or"),
        view,
        &["height", "width"],
    )?;
    Ok(Some(CopySize::Texels {
        width: whole(FUNCTION, "a width", Some(fields.raw_get(frame, "width")?), None)?,
        height: whole(FUNCTION, "a height", Some(fields.raw_get(frame, "height")?), None)?,
    }))
}

/// A group a pass gives: a bind group, or bindings by binding number for `group`'s layout, each a
/// resource or the name of one of the graph's targets or buffers.
fn plan_group(frame: &Frame<'_>, what: &str, group: &Group, view: ValueView<'_>) -> l3i::Result<PlanGroup> {
    if let Some(bound) = receiver::<LuaGpuBindGroup>(view) {
        return Ok(PlanGroup::Bound(bound.group.clone()));
    }
    let frequency = group.frequency;
    let bindings = table(FUNCTION, &format!("{what}, a GpuBindGroup or bindings,"), view)?;
    let mut made = Vec::new();
    bindings.for_each(frame, |_, key, value| {
        let number = whole(FUNCTION, "a binding number", Some(key), None)?;
        let Some((_, slot, name)) = group.slots.iter().find(|(declared, ..)| *declared == number) else {
            return Err(error(format!(
                "{FUNCTION}: {what}: the program binds nothing at {frequency} binding {number}"
            )));
        };
        let named = value.is_string().then(|| value.read::<&str>()).transpose()?;
        let binding = match (slot, named) {
            (Slot::Resource(id, kind), Some(target)) if !kind.is_sampler() => PlanBinding::Target(*id, target.to_owned()),
            (Slot::Storage(id, ..), Some(buffer)) => PlanBinding::Buffer(*id, buffer.to_owned()),
            (Slot::Uniform(_), None) => receiver::<LuaGpuBuffer>(value)
                .map(|buffer| PlanBinding::Uniform(buffer.buffer.clone()))
                .ok_or_else(|| expected(what, number, slot, name, "a GpuBuffer", value))?,
            (Slot::Storage(id, ..), None) => receiver::<LuaGpuBuffer>(value)
                .map(|buffer| PlanBinding::Storage(*id, buffer.buffer.clone()))
                .ok_or_else(|| expected(what, number, slot, name, "a GpuBuffer or a buffer's name", value))?,
            (Slot::Resource(id, kind), None) if kind.is_sampler() => receiver::<LuaGpuSampler>(value)
                .map(|sampler| PlanBinding::Sampler(*id, sampler.sampler.clone()))
                .ok_or_else(|| expected(what, number, slot, name, "a GpuSampler", value))?,
            (Slot::Resource(id, _), None) => receiver::<LuaGpuTexture>(value)
                .map(|texture| PlanBinding::Texture(*id, texture.texture.clone()))
                .ok_or_else(|| expected(what, number, slot, name, "a GpuTexture or a target's name", value))?,
            (_, Some(_)) => return Err(expected(what, number, slot, name, "a resource, not a name", value)),
        };
        made.push(binding);
        Ok(())
    })?;
    Ok(PlanGroup::Make(made))
}

fn expected(what: &str, number: u32, slot: &Slot, name: &str, expected: &str, value: ValueView<'_>) -> l3i::Error {
    error(format!(
        "{FUNCTION}: {what}: binding {number} is {}: expected {expected}, not {}",
        slot_name(*slot, name),
        kind(value)
    ))
}

/// The groups a fullscreen or compute pass gives by frequency, for `groups`' layouts.
fn plan_groups(
    frame: &Frame<'_>,
    pass: &str,
    fields: &TableView<'_>,
    groups: &[Group; 3],
) -> l3i::Result<[Option<PlanGroup>; 3]> {
    let mut planned = [None, None, None];
    for frequency in Frequency::ALL {
        let view = fields.raw_get(frame, frequency.name())?;
        if view.is_nil() {
            continue;
        }
        let what = format!("pass {pass}'s {frequency} group");
        planned[frequency as usize] = Some(plan_group(frame, &what, &groups[frequency as usize], view)?);
    }
    Ok(planned)
}

/// Adds what a drawing pass reads and writes to `pass`: `colors`, `depth`, `depthTest`.
fn attachments(frame: &Frame<'_>, graph: &RenderGraph, fields: &TableView<'_>, mut pass: Pass) -> l3i::Result<Pass> {
    let name = pass.name().to_owned();
    let colors = fields.raw_get(frame, "colors")?;
    if !colors.is_nil() {
        table(FUNCTION, &format!("pass {name}'s colors"), colors)?.for_each_array(frame, |frame, _, color| {
            pass.outputs
                .push(output(frame, graph, &name, color, false, PassOutput::color)?);
            Ok(())
        })?;
    }
    let depth = fields.raw_get(frame, "depth")?;
    if !depth.is_nil() {
        pass.outputs
            .push(output(frame, graph, &name, depth, false, PassOutput::depth)?);
    }
    let tested = fields.raw_get(frame, "depthTest")?;
    if !tested.is_nil() {
        let (id, level) = named_level(frame, graph, &name, &format!("pass {name}'s depth test"), tested)?;
        pass = match level {
            Some(level) => pass.read_at(id, ReadUsage::DepthTest, level.level, level.layer),
            None => pass.read(id, ReadUsage::DepthTest),
        };
    }
    Ok(pass)
}

/// A draw pass's work: its `tags`, and its `pass` group, made for `layout` from bindings.
fn draw_work(
    frame: &Frame<'_>,
    name: &str,
    fields: &TableView<'_>,
    layout: Option<Arc<crate::gpu::RenderPipeline>>,
) -> l3i::Result<Work> {
    let group = fields.raw_get(frame, "pass")?;
    let group = if group.is_nil() {
        None
    } else if let Some(bound) = receiver::<LuaGpuBindGroup>(group) {
        Some(PlanGroup::Bound(bound.group.clone()))
    } else {
        let Some(layout) = &layout else {
            return Err(error(format!(
                "{FUNCTION}: pass {name} makes its pass group from bindings, and names no pipeline to lay them \
                 out for"
            )));
        };
        let what = format!("pass {name}'s pass group");
        Some(plan_group(frame, &what, &layout.groups[Frequency::Pass as usize], group)?)
    };
    Ok(Work::Draw {
        tags: names(frame, &format!("pass {name}'s tags"), fields.raw_get(frame, "tags")?)?,
        pipeline: layout,
        pass: group,
    })
}

/// A pass and its work: `{ name, kind, owner, ... }`, the rest by kind.
fn pass_and_work(
    frame: &Frame<'_>,
    graph: &RenderGraph,
    owner: &str,
    site: &(String, u32),
    index: i64,
    view: ValueView<'_>,
) -> l3i::Result<(Pass, Work)> {
    let what = format!("pass {index}");
    let fields = table(FUNCTION, &what, view)?;
    let kind = text(frame, &what, &fields, "kind")?.unwrap_or_default();
    let (pass_kind, keys): (PassKind, &[&str]) = match kind.as_str() {
        "draw" => (PassKind::Mesh, &DRAW_KEYS),
        "fullscreen" => (PassKind::Fullscreen, &FULLSCREEN_KEYS),
        "compute" => (PassKind::Compute, &COMPUTE_KEYS),
        "copy" => (PassKind::Copy, &COPY_KEYS),
        _ => {
            return Err(error(format!(
                "{FUNCTION}: {what}'s kind is one of {}, not {}",
                KINDS.join(", "),
                shown(fields.raw_get(frame, "kind")?)
            )));
        }
    };
    check_options(frame, view, &format!("{FUNCTION} {kind} pass"), keys)?;
    let name = required(frame, &what, &fields, "name")?;
    let owner = text(frame, &what, &fields, "owner")?.unwrap_or_else(|| owner.to_owned());
    // A pass is a table, which has no place of its own: it was declared where the plan was.
    let metadata = PassMetadata::new(name.clone(), owner).with_source(site.0.clone(), site.1, 0);
    let mut pass = Pass::new(pass_kind, metadata);
    let reads = fields.raw_get(frame, "reads")?;
    if !reads.is_nil() {
        let what = format!("pass {name}'s reads");
        let mut read = Vec::new();
        table(FUNCTION, &what, reads)?.for_each_array(frame, |frame, _, view| {
            read.push(named_level(frame, graph, &name, &format!("a read of pass {name}"), view)?);
            Ok(())
        })?;
        for (id, level) in read {
            let usage = match graph.resources()[id.index()].kind() {
                ResourceKind::RenderTarget => ReadUsage::Texture,
                ResourceKind::DepthTarget => ReadUsage::DepthTexture,
                ResourceKind::Buffer => ReadUsage::Buffer,
            };
            pass = match level {
                Some(level) => pass.read_at(id, usage, level.level, level.layer),
                None => pass.read(id, usage),
            };
        }
    }
    let pipeline = || pipeline_field(frame, FUNCTION, &fields);
    let render = |pipeline: &Pipeline| match pipeline {
        Pipeline::Render(pipeline) => Ok(pipeline.clone()),
        Pipeline::Compute(_) => Err(error(format!(
            "{FUNCTION}: pass {name}'s pipeline computes, and the pass draws"
        ))),
    };
    let work = match pass_kind {
        PassKind::Mesh => {
            pass = attachments(frame, graph, &fields, pass)?;
            let layout = fields.raw_get(frame, "pipeline")?;
            let layout = if layout.is_nil() { None } else { Some(render(&*pipeline()?)?) };
            draw_work(frame, &name, &fields, layout)?
        }
        PassKind::Fullscreen => {
            pass = attachments(frame, graph, &fields, pass)?;
            let pipeline = render(&*pipeline()?)?;
            Work::Fullscreen {
                groups: plan_groups(frame, &name, &fields, &pipeline.groups)?,
                pipeline,
            }
        }
        PassKind::Compute => {
            let storage = fields.raw_get(frame, "storage")?;
            if !storage.is_nil() {
                table(FUNCTION, &format!("pass {name}'s storage"), storage)?.for_each_array(frame, |frame, _, write| {
                    pass.outputs
                        .push(output(frame, graph, &name, write, true, PassOutput::storage)?);
                    Ok(())
                })?;
            }
            let Pipeline::Compute(pipeline) = &*pipeline()? else {
                return Err(error(format!(
                    "{FUNCTION}: pass {name}'s pipeline draws, and the pass computes"
                )));
            };
            Work::Compute {
                groups: plan_groups(frame, &name, &fields, &pipeline.groups)?,
                workgroups: match fields.raw_get(frame, "workgroups")? {
                    counts if counts.is_nil() => [1; 3],
                    counts => workgroups(frame, FUNCTION, counts)?,
                },
                pipeline: pipeline.clone(),
            }
        }
        _ => {
            pass = copy_pass(frame, graph, &fields, pass)?;
            Work::Copy
        }
    };
    Ok((pass, work))
}

/// Adds what a copy pass reads and writes to `pass`: `from`, `to`, `size`, and `load`.
fn copy_pass(frame: &Frame<'_>, graph: &RenderGraph, fields: &TableView<'_>, pass: Pass) -> l3i::Result<Pass> {
    let name = pass.name().to_owned();
    let what = format!("pass {name}");
    let end = |key: &str| -> l3i::Result<_> {
        let view = fields.raw_get(frame, key)?;
        if view.is_nil() {
            return Err(error(format!("{FUNCTION}: {what} needs a {key}")));
        }
        copy_end(frame, graph, &name, &format!("{what}'s {key}"), view)
    };
    let (source, from_level, from) = end("from")?;
    let (destination, to_level, to) = end("to")?;
    let load = if flag(frame, FUNCTION, fields, "load", false)? {
        LoadOp::Load
    } else {
        LoadOp::DontCare
    };
    let amount = copy_size(frame, &what, fields.raw_get(frame, "size")?)?;
    let from_level = from_level.unwrap_or_default();
    let to_level = to_level.unwrap_or_default();
    Ok(pass
        .read_at(source, ReadUsage::Copy, from_level.level, from_level.layer)
        .write(PassOutput::copy(destination, load, StoreOp::Store).at(to_level.level, to_level.layer))
        .region(CopyRegion { from, to, size: amount }))
}

/// `device:plan { name, viewport, targets, buffers, passes }`.
fn plan(this: &LuaGpuDevice, call: &Call<'_>, desc: ValueView<'_>) -> l3i::Result<Owned<LuaGpuPlan>> {
    if !desc.is_table() {
        return Err(error(format!("{FUNCTION} takes a table, not {}", kind(desc))));
    }
    let site = crate::lua::shader::call_site(call);
    let (graph, work, viewport) = call.with_frame(|frame| {
        let fields = options(frame, FUNCTION, "the plan", desc, &PLAN_KEYS)?;
        let name = text(frame, "the plan", &fields, "name")?.unwrap_or_else(|| "plan".to_owned());
        let viewport = extent(frame, fields.raw_get(frame, "viewport")?)?;
        let mut graph = RenderGraph::new(name.clone());
        add_targets(frame, &mut graph, fields.raw_get(frame, "targets")?)?;
        add_buffers(frame, &mut graph, fields.raw_get(frame, "buffers")?)?;
        let mut work = Vec::new();
        let passes = fields.raw_get(frame, "passes")?;
        table(FUNCTION, "passes", passes)?.for_each_array(frame, |frame, index, view| {
            let (pass, done) = pass_and_work(frame, &graph, &name, &site, index, view)?;
            graph.add_pass(pass).map_err(|problem| graph_error(&problem))?;
            work.push(done);
            Ok(())
        })?;
        Ok((graph, work, viewport))
    })?;
    let plan = this
        .gpu
        .plan(&graph, work, viewport)
        .map_err(|problem| gpu_error(FUNCTION, &problem))?;
    Ok(Owned(LuaGpuPlan {
        gpu: this.gpu.clone(),
        sets: this.sets.clone(),
        plan,
    }))
}

/// A viewport: `{ width, height }`.
fn extent(frame: &Frame<'_>, view: ValueView<'_>) -> l3i::Result<Extent> {
    let fields = options(frame, FUNCTION, "the viewport", view, &["height", "width"])?;
    Ok(Extent {
        width: whole(FUNCTION, "the viewport's width", Some(fields.raw_get(frame, "width")?), None)?,
        height: whole(
            FUNCTION,
            "the viewport's height",
            Some(fields.raw_get(frame, "height")?),
            None,
        )?,
    })
}

/// A resource given to a run, held while it runs.
pub enum Given {
    /// A texture, for an imported target.
    Texture(String, Arc<crate::gpu::Texture>),
    /// A buffer, for an imported buffer.
    Buffer(String, Arc<crate::gpu::Buffer>),
}

/// The imports a table names, `{ [name] = texture | buffer }`, for `function`.
pub fn given(call: &Call<'_>, function: &str, imports: Option<ValueView<'_>>) -> l3i::Result<Vec<Given>> {
    let mut given = Vec::new();
    let Some(imports) = imports.filter(|imports| !imports.is_nil()) else {
        return Ok(given);
    };
    if !imports.is_table() {
        return Err(error(format!("{function} takes a table of imports, not {}", kind(imports))));
    }
    call.with_frame(|frame| {
        imports.as_table()?.for_each(frame, |_, key, value| {
            if !key.is_string() {
                return Err(error(format!("{function}: an import's name is a string, not {}", shown(key))));
            }
            let name = key.read::<&str>()?.to_owned();
            if let Some(texture) = receiver::<LuaGpuTexture>(value) {
                given.push(Given::Texture(name, texture.texture.clone()));
            } else if let Some(buffer) = receiver::<LuaGpuBuffer>(value) {
                given.push(Given::Buffer(name, buffer.buffer.clone()));
            } else {
                return Err(error(format!(
                    "{function}: {name} is a GpuTexture or a GpuBuffer, not {}",
                    kind(value)
                )));
            }
            Ok(())
        })
    })?;
    Ok(given)
}

/// `plan:run(imports, sets)`: every pass recorded, with `imports` by name, drawing from `sets`,
/// or the device's own.
fn run(this: &LuaGpuPlan, call: &Call<'_>, imports: Option<ValueView<'_>>, sets: Option<ValueView<'_>>) -> l3i::Result<()> {
    const RUN: &str = "GpuPlan:run";
    let sets = collection(RUN, "the draw sets", sets, &this.sets)?;
    let given = given(call, RUN, imports)?;
    let imports: Vec<Import<'_>> = given
        .iter()
        .map(|given| match given {
            Given::Texture(name, texture) => Import::Texture(name, texture),
            Given::Buffer(name, buffer) => Import::Buffer(name, buffer),
        })
        .collect();
    this.gpu
        .run(&this.plan, &sets, &imports)
        .map_err(|problem| gpu_error(RUN, &problem))
}

/// `plan:setEnabled(passes, enabled)`: a pass's name or a list of them, recorded from the next
/// run on or left out, together.
fn set_enabled(this: &LuaGpuPlan, call: &Call<'_>, passes: ValueView<'_>, enabled: ValueView<'_>) -> l3i::Result<()> {
    const FUNCTION: &str = "GpuPlan:setEnabled";
    if !enabled.is_boolean() {
        return Err(error(format!("{FUNCTION}: enabled is a boolean, not {}", shown(enabled))));
    }
    let names = if passes.is_string() {
        vec![passes.read::<&str>()?.to_owned()]
    } else {
        call.with_frame(|frame| names(frame, "the passes", passes)).map_err(|_| {
            error(format!(
                "{FUNCTION} takes a pass's name or a list of them, not {}",
                shown(passes)
            ))
        })?
    };
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    this.plan
        .set_enabled(&names, enabled.read::<bool>()?)
        .map_err(|problem| gpu_error(FUNCTION, &problem))
}

/// Where a region starts: `{ column, row }` in a target, or `{ offset }` in a buffer.
fn region_start(frame: &Frame<'_>, function: &str, what: &str, view: ValueView<'_>) -> l3i::Result<Option<CopyAt>> {
    if view.is_nil() {
        return Ok(None);
    }
    let fields = options(frame, function, what, view, &["column", "offset", "row"])?;
    let field = |key: &str| fields.raw_get(frame, key);
    let offset = field("offset")?;
    if !offset.is_nil() {
        if !field("column")?.is_nil() || !field("row")?.is_nil() {
            return Err(error(format!(
                "{function}: {what} starts at an offset or at a column and row, not both"
            )));
        }
        return Ok(Some(CopyAt::Byte(bytes(function, "an offset", offset)?)));
    }
    Ok(Some(CopyAt::Texel {
        x: whole(function, "the column", Some(field("column")?), Some(0))?,
        y: whole(function, "the row", Some(field("row")?), Some(0))?,
    }))
}

/// `plan:setRegion(pass, { from, to, size })`: what a copy pass copies from the next run on.
fn set_region(this: &LuaGpuPlan, call: &Call<'_>, pass: ValueView<'_>, region: ValueView<'_>) -> l3i::Result<()> {
    const FUNCTION: &str = "GpuPlan:setRegion";
    if !pass.is_string() {
        return Err(error(format!("{FUNCTION} takes a copy pass's name, not {}", shown(pass))));
    }
    let region = call.with_frame(|frame| {
        let fields = options(frame, FUNCTION, "the region", region, &["from", "size", "to"])?;
        Ok(CopyRegion {
            from: region_start(frame, FUNCTION, "its from", fields.raw_get(frame, "from")?)?,
            to: region_start(frame, FUNCTION, "its to", fields.raw_get(frame, "to")?)?,
            size: copy_size(frame, "the region", fields.raw_get(frame, "size")?)?,
        })
    })?;
    this.plan
        .set_region(pass.read::<&str>()?, region)
        .map_err(|problem| gpu_error(FUNCTION, &problem))
}

/// A kept resource's name, and the level and layer of it read: 0 and 0 unless named.
fn kept_args(
    function: &str,
    name: ValueView<'_>,
    level: Option<ValueView<'_>>,
    layer: Option<ValueView<'_>>,
) -> l3i::Result<(String, u32, u32)> {
    if !name.is_string() {
        return Err(error(format!("{function} takes a kept resource's name, not {}", shown(name))));
    }
    Ok((
        name.read::<&str>()?.to_owned(),
        whole(function, "the level", level, Some(0))?,
        whole(function, "the layer", layer, Some(0))?,
    ))
}

/// `plan:resized(width, height)`: the same plan for another viewport.
fn resized(this: &LuaGpuPlan, width: ValueView<'_>, height: ValueView<'_>) -> l3i::Result<Owned<LuaGpuPlan>> {
    const RESIZED: &str = "GpuPlan:resized";
    let viewport = Extent {
        width: whole(RESIZED, "the width", Some(width), None)?,
        height: whole(RESIZED, "the height", Some(height), None)?,
    };
    let plan = this
        .plan
        .resized(&this.gpu, viewport)
        .map_err(|problem| gpu_error(RESIZED, &problem))?;
    Ok(Owned(LuaGpuPlan {
        gpu: this.gpu.clone(),
        sets: this.sets.clone(),
        plan,
    }))
}

/// Declares the device's method that compiles plans.
pub fn device_methods(device: &mut UserdataBuilder<'_, LuaGpuDevice>) {
    device.method("plan", plan).untyped();
}

/// Declares the plan type.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut plan = d.userdata::<LuaGpuPlan>(LuaGpuPlan::NAME);
    plan.tag(TagPolicy::Never)
        .doc("A render graph compiled for its device and a viewport, run whole every frame.");
    plan.getter("name", |this: &LuaGpuPlan| this.plan.graph().name().to_owned())
        .untyped();
    plan.getter("order", |this: &LuaGpuPlan, call: &Call<'_>| -> l3i::Result<StackResults> {
        l3i::stack::push_list(call, this.plan.order().into_iter(), |frame, name| {
            frame.push_string(name).map(drop)
        })?;
        Ok(StackResults)
    })
    .untyped();
    plan.getter(
        "viewport",
        |this: &LuaGpuPlan, call: &Call<'_>| -> l3i::Result<StackResults> {
            let viewport = this.plan.viewport();
            push_table(call, 0, 2, |frame, table| {
                table.raw_set_value(frame, "width", &f64::from(viewport.width))?;
                table.raw_set_value(frame, "height", &f64::from(viewport.height))
            })?;
            Ok(StackResults)
        },
    )
    .untyped();
    plan.getter("allocations", |this: &LuaGpuPlan| {
        u32::try_from(this.plan.allocations()).unwrap_or(u32::MAX)
    })
    .untyped();
    plan.method("run", run).untyped();
    describe_reads(&mut plan);
    plan.method("setEnabled", set_enabled).untyped();
    plan.method("setRegion", set_region).untyped();
    plan.method("enabled", |this: &LuaGpuPlan, pass: ValueView<'_>| -> l3i::Result<bool> {
        const FUNCTION: &str = "GpuPlan:enabled";
        if !pass.is_string() {
            return Err(error(format!("{FUNCTION} takes a pass's name, not {}", shown(pass))));
        }
        this.plan
            .enabled(pass.read::<&str>()?)
            .map_err(|problem| gpu_error(FUNCTION, &problem))
    })
    .untyped();
    plan.method("timings", |this: &LuaGpuPlan, call: &Call<'_>| -> l3i::Result<StackResults> {
        let timings = this
            .gpu
            .timings(&this.plan)
            .map_err(|problem| gpu_error("GpuPlan:timings", &problem))?;
        let Some(timings) = timings else {
            call.with_frame(|frame| frame.push_nil().map(drop))?;
            return Ok(StackResults);
        };
        l3i::stack::push_list(call, timings.iter(), |frame, timing| {
            push_table(frame, 0, 2, |frame, table| {
                table.raw_set_value(frame, "pass", timing.pass.as_str())?;
                if let Some(duration) = timing.duration {
                    table.raw_set_value(frame, "ms", &(duration.as_secs_f64() * 1000.0))?;
                }
                Ok(())
            })
        })?;
        Ok(StackResults)
    })
    .untyped();
    plan.method("resized", resized).untyped();
    plan.method("dump", |this: &LuaGpuPlan| -> l3i::Result<String> {
        let graph = this.plan.graph();
        graph
            .compile()
            .map(|compiled| compiled.to_string())
            .map_err(|problem| error(format!("GpuPlan:dump: {problem}")))
    })
    .untyped();
    crate::lua::plan::metamethod(&mut plan, "__tostring", |this: &LuaGpuPlan| {
        let viewport = this.plan.viewport();
        let (running, all) = (this.plan.order().len(), this.plan.graph().passes().len());
        format!(
            "GpuPlan({}, {running} of {all} passes, {}×{})",
            this.plan.graph().name(),
            viewport.width,
            viewport.height
        )
    });
}

/// Declares the plan's methods that read what it keeps and say how big its resources are.
fn describe_reads(plan: &mut UserdataBuilder<'_, LuaGpuPlan>) {
    plan.method(
        "read",
        |this: &LuaGpuPlan,
         name: ValueView<'_>,
         level: Option<ValueView<'_>>,
         layer: Option<ValueView<'_>>|
         -> l3i::Result<NewBuffer> {
            const READ: &str = "GpuPlan:read";
            let (name, level, layer) = kept_args(READ, name, level, layer)?;
            this.gpu
                .read_kept(&this.plan, &name, level, layer)
                .map(NewBuffer)
                .map_err(|problem| gpu_error(READ, &problem))
        },
    )
    .untyped();
    plan.method(
        "readLater",
        |this: &LuaGpuPlan,
         name: ValueView<'_>,
         level: Option<ValueView<'_>>,
         layer: Option<ValueView<'_>>|
         -> l3i::Result<Owned<LuaGpuRead>> {
            const READ: &str = "GpuPlan:readLater";
            let (name, level, layer) = kept_args(READ, name, level, layer)?;
            let read = this
                .gpu
                .read_kept_later(&this.plan, &name, level, layer)
                .map_err(|problem| gpu_error(READ, &problem))?;
            Ok(Owned(LuaGpuRead {
                read: std::cell::RefCell::new(read),
            }))
        },
    )
    .untyped();
    plan.method("bytes", |this: &LuaGpuPlan, name: ValueView<'_>| -> l3i::Result<f64> {
        const BYTES: &str = "GpuPlan:bytes";
        if !name.is_string() {
            return Err(error(format!("{BYTES} takes a buffer's name, not {}", shown(name))));
        }
        let bytes = this
            .plan
            .buffer_bytes(name.read::<&str>()?)
            .map_err(|problem| gpu_error(BYTES, &problem))?;
        // A buffer's bytes stay far below 2^53.
        #[allow(clippy::cast_precision_loss)]
        Ok(bytes as f64)
    })
    .untyped();
    plan.method(
        "extent",
        |this: &LuaGpuPlan, call: &Call<'_>, name: ValueView<'_>| -> l3i::Result<StackResults> {
            const EXTENT: &str = "GpuPlan:extent";
            if !name.is_string() {
                return Err(error(format!("{EXTENT} takes a target's name, not {}", shown(name))));
            }
            let extent = this
                .plan
                .target_extent(name.read::<&str>()?)
                .map_err(|problem| gpu_error(EXTENT, &problem))?;
            push_table(call, 0, 2, |frame, table| {
                table.raw_set_value(frame, "width", &f64::from(extent.width))?;
                table.raw_set_value(frame, "height", &f64::from(extent.height))
            })?;
            Ok(StackResults)
        },
    )
    .untyped();
}
