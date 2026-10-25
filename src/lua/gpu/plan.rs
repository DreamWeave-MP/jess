//! Plans from Luau: a render graph described as a table, its targets, buffers, and passes and what
//! each pass does, compiled for the device and a viewport, and run whole, every frame, from
//! `plan:run`.

use std::rc::Rc;
use std::sync::Arc;

use l3i::bind::{Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata, receiver};

use super::device::LuaGpuDevice;
use super::pipeline::{
    LuaGpuBindGroup, Pipeline, clear_value, flag, gpu_error, number, options, pipeline_field, slot_name, table, workgroups,
};
use super::resources::{LuaGpuBuffer, LuaGpuSampler, LuaGpuTexture, format_of, shown, whole};
use crate::gpu::{DrawSets, Gpu, Group, Import, Plan, PlanBinding, PlanGroup, Slot, Work};
use crate::lua::{check_options, error, kind, push_table};
use crate::render::{
    ClearValue, Extent, LoadOp, Pass, PassKind, PassMetadata, PassOutput, ReadUsage, RenderGraph, ResourceDesc,
    ResourceKind, StoreOp, TargetSize,
};
use crate::shader::Frequency;

/// A plan compiled from Luau, and the device and draw sets it runs with.
#[derive(Debug)]
pub struct LuaGpuPlan {
    pub gpu: Arc<Gpu>,
    pub sets: Rc<DrawSets>,
    pub plan: Plan,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuPlan {
    const NAME: &'static str = "jess.gpu.Plan";
}

const FUNCTION: &str = "GpuDevice:plan";

const PLAN_KEYS: [&str; 5] = ["buffers", "name", "passes", "targets", "viewport"];

const TARGET_KEYS: [&str; 6] = ["format", "height", "imported", "name", "scale", "width"];

const BUFFER_KEYS: [&str; 3] = ["imported", "name", "size"];

const WRITE_KEYS: [&str; 4] = ["clear", "load", "store", "target"];

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

const COPY_KEYS: [&str; 6] = ["from", "kind", "load", "name", "owner", "to"];

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

/// Registers each of `targets`: `{ name, format, scale | width and height, imported }`.
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
        let imported = flag(frame, FUNCTION, &fields, "imported", false)?;
        let added = match (format.is_depth(), imported) {
            (false, false) => graph.add_render_target(name, format, size),
            (true, false) => graph.add_depth_target(name, format, size),
            (false, true) => graph.import_render_target(name, format, size),
            (true, true) => graph.import_depth_target(name, format, size),
        };
        added.map(drop).map_err(|problem| graph_error(&problem))
    })
}

/// Registers each of `buffers`: `{ name, size, imported }`.
fn add_buffers(frame: &Frame<'_>, graph: &mut RenderGraph, buffers: ValueView<'_>) -> l3i::Result<()> {
    if buffers.is_nil() {
        return Ok(());
    }
    table(FUNCTION, "buffers", buffers)?.for_each_array(frame, |frame, index, buffer| {
        let what = format!("buffer {index}");
        let fields = options(frame, FUNCTION, &what, buffer, &BUFFER_KEYS)?;
        let name = required(frame, &what, &fields, "name")?;
        let size = number(FUNCTION, "a buffer's size", fields.raw_get(frame, "size")?)?;
        if size.fract() != 0.0 || size < 0.0 {
            return Err(error(format!("{FUNCTION}: a buffer's size is whole bytes, not {size}")));
        }
        // Whole and non-negative, so the cast is exact below 2^53.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let bytes = size as u64;
        let added = if flag(frame, FUNCTION, &fields, "imported", false)? {
            graph.import_buffer(name, bytes)
        } else {
            graph.add_buffer(name, bytes)
        };
        added.map(drop).map_err(|problem| graph_error(&problem))
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

/// A write: a resource's name, or `{ target | buffer, clear, load, store }`. A target clears to
/// what `clear` gives; a buffer, to zero, when `clear` is `true`.
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
            ResourceDesc::Texture { format, .. } => LoadOp::Clear(clear_value(frame, FUNCTION, &what, format, clear)?),
        },
    };
    let store = if flag(frame, FUNCTION, &fields, "store", true)? {
        StoreOp::Store
    } else {
        StoreOp::Discard
    };
    Ok(attachment(id, load, store))
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
    if let Some(tested) = text(frame, &format!("pass {name}"), fields, "depthTest")? {
        pass = pass.read(resource(graph, &name, &tested)?, ReadUsage::DepthTest);
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
    let mut pass = Pass::new(pass_kind, PassMetadata::new(name.clone(), owner));
    for read in names(frame, &format!("pass {name}'s reads"), fields.raw_get(frame, "reads")?)? {
        let id = resource(graph, &name, &read)?;
        let usage = match graph.resources()[id.index()].kind() {
            ResourceKind::RenderTarget => ReadUsage::Texture,
            ResourceKind::DepthTarget => ReadUsage::DepthTexture,
            ResourceKind::Buffer => ReadUsage::Buffer,
        };
        pass = pass.read(id, usage);
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
            let from = required(frame, &format!("pass {name}"), &fields, "from")?;
            let to = required(frame, &format!("pass {name}"), &fields, "to")?;
            let load = if flag(frame, FUNCTION, &fields, "load", false)? {
                LoadOp::Load
            } else {
                LoadOp::DontCare
            };
            pass = pass
                .read(resource(graph, &name, &from)?, ReadUsage::Copy)
                .write(PassOutput::copy(resource(graph, &name, &to)?, load, StoreOp::Store));
            Work::Copy
        }
    };
    Ok((pass, work))
}

/// `device:plan { name, viewport, targets, buffers, passes }`.
fn plan(this: &LuaGpuDevice, call: &Call<'_>, desc: ValueView<'_>) -> l3i::Result<Owned<LuaGpuPlan>> {
    if !desc.is_table() {
        return Err(error(format!("{FUNCTION} takes a table, not {}", kind(desc))));
    }
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
            let (pass, done) = pass_and_work(frame, &graph, &name, index, view)?;
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

/// `plan:run(imports)`: every surviving pass, with `imports` by name.
fn run(this: &LuaGpuPlan, call: &Call<'_>, imports: Option<ValueView<'_>>) -> l3i::Result<()> {
    const RUN: &str = "GpuPlan:run";
    let given = given(call, RUN, imports)?;
    let imports: Vec<Import<'_>> = given
        .iter()
        .map(|given| match given {
            Given::Texture(name, texture) => Import::Texture(name, texture),
            Given::Buffer(name, buffer) => Import::Buffer(name, buffer),
        })
        .collect();
    this.gpu
        .run(&this.plan, &this.sets, &imports)
        .map_err(|problem| gpu_error(RUN, &problem))
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
