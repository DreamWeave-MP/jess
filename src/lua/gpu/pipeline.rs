//! Pipelines, bind groups, passes, and dispatches from Luau: a shader program made ready to draw
//! or to compute, what each frequency binds for it by the binding numbers `program.layout`
//! reports, and the work itself.

use std::rc::Rc;
use std::sync::Arc;

use l3i::bind::Call;
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Frame, Scope, TableView, ValueView};
use l3i::userdata::{Owned, Userdata, receiver};

use super::device::LuaGpuDevice;
use super::resources::{LuaGpuBuffer, LuaGpuSampler, LuaGpuTexture, choice, compare_of, format_of, shown, whole};
use crate::gpu::{
    BindGroup, Binding, Blend, Buffer, ColorState, Compare, ComputePipeline, Cull, DepthState, Draw, Gpu, GpuError, Group,
    IndexFormat, PipelineState, RenderPipeline, Resolve, Sampler, Slot, Target, Texture, Topology,
};
use crate::lua::shader::LuaShaderProgram;
use crate::lua::{check_options, error, kind};
use crate::render::{ClearValue, LoadOp, StoreOp};
use crate::shader::Frequency;

/// A program made ready to draw or to compute.
#[derive(Debug)]
pub enum Pipeline {
    /// A vertex and fragment program and its fixed-function state.
    Render(Arc<RenderPipeline>),
    /// A compute program.
    Compute(Arc<ComputePipeline>),
}

impl Pipeline {
    fn groups(&self) -> &[Group; 3] {
        match self {
            Pipeline::Render(pipeline) => &pipeline.groups,
            Pipeline::Compute(pipeline) => &pipeline.groups,
        }
    }
}

/// A program ready to draw or to compute.
#[derive(Debug)]
pub struct LuaGpuPipeline {
    gpu: Arc<Gpu>,
    pipeline: Rc<Pipeline>,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuPipeline {
    const NAME: &'static str = "jess.gpu.Pipeline";
}

/// What one frequency binds for a pipeline.
#[derive(Debug)]
pub struct LuaGpuBindGroup {
    pub group: Arc<BindGroup>,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuBindGroup {
    const NAME: &'static str = "jess.gpu.BindGroup";
}

const STATE_KEYS: [&str; 6] = ["clockwise", "colors", "cull", "depth", "multisampled", "topology"];

const COLOR_KEYS: [&str; 2] = ["blend", "format"];

const DEPTH_KEYS: [&str; 3] = ["compare", "format", "write"];

const TARGET_KEYS: [&str; 6] = ["clear", "layer", "level", "resolve", "store", "texture"];

const RESOLVE_KEYS: [&str; 3] = ["layer", "level", "texture"];

const RENDER_KEYS: [&str; 3] = ["colors", "depth", "draws"];

const DRAW_KEYS: [&str; 12] = [
    "count",
    "draw",
    "indexFormat",
    "indices",
    "indirect",
    "indirectOffset",
    "instanceCount",
    "instances",
    "material",
    "pass",
    "pipeline",
    "vertices",
];

const DISPATCH_KEYS: [&str; 5] = ["draw", "material", "pass", "pipeline", "workgroups"];

const COMPARES: [&str; 8] = [
    "never",
    "less",
    "equal",
    "lessEqual",
    "greater",
    "notEqual",
    "greaterEqual",
    "always",
];

pub fn gpu_error(function: &str, problem: &GpuError) -> l3i::Error {
    error(format!("{function}: {problem}"))
}

/// Commits what the script changed, before `function` uses the GPU and waits on it: a script
/// sees its own changes.
pub fn committed(gpu: &Gpu, function: &str) -> l3i::Result<()> {
    gpu.commit().map(drop).map_err(|problem| gpu_error(function, &problem))
}

/// `view` as a table, or the error naming `what` it should have been.
pub fn table<'v>(function: &str, what: &str, view: ValueView<'v>) -> l3i::Result<TableView<'v>> {
    if !view.is_table() {
        return Err(error(format!("{function}: {what} is a table, not {}", shown(view))));
    }
    view.as_table()
}

/// [`table`], taking only `keys`.
pub fn options<'v>(
    frame: &Frame<'_>,
    function: &str,
    what: &str,
    view: ValueView<'v>,
    keys: &[&str],
) -> l3i::Result<TableView<'v>> {
    let options = table(function, what, view)?;
    check_options(frame, view, function, keys)?;
    Ok(options)
}

/// A boolean field, or `default` where it is nil.
pub fn flag(frame: &Frame<'_>, function: &str, table: &TableView<'_>, key: &str, default: bool) -> l3i::Result<bool> {
    let view = table.raw_get(frame, key)?;
    if view.is_nil() {
        return Ok(default);
    }
    if !view.is_boolean() {
        return Err(error(format!("{function}: {key} is a boolean, not {}", shown(view))));
    }
    view.read::<bool>()
}

/// A finite number.
pub fn number(function: &str, what: &str, view: ValueView<'_>) -> l3i::Result<f64> {
    let value = (view.is_number() || view.is_integer())
        .then(|| view.read::<f64>().ok())
        .flatten()
        .filter(|value| value.is_finite());
    value.ok_or_else(|| error(format!("{function}: {what} is a number, not {}", shown(view))))
}

/// `device:pipeline(program, state)`.
fn pipeline(
    this: &LuaGpuDevice,
    call: &Call<'_>,
    program: ValueView<'_>,
    state: Option<ValueView<'_>>,
) -> l3i::Result<Owned<LuaGpuPipeline>> {
    const FUNCTION: &str = "GpuDevice:pipeline";
    let Some(program) = receiver::<LuaShaderProgram>(program) else {
        return Err(error(format!("{FUNCTION} takes a shader program, not {}", kind(program))));
    };
    let program = program.program();
    let state = state.filter(|state| !state.is_nil());
    let pipeline = if program.workgroup_size().is_some() {
        if let Some(state) = state {
            return Err(error(format!(
                "{FUNCTION}: a compute program has no drawing state, and was given {}",
                shown(state)
            )));
        }
        Pipeline::Compute(
            this.gpu
                .cached_compute_pipeline(program)
                .map_err(|problem| gpu_error(FUNCTION, &problem))?,
        )
    } else {
        let Some(state) = state else {
            return Err(error(format!(
                "{FUNCTION}: a vertex and fragment program needs a state naming its color targets"
            )));
        };
        let state = call.with_frame(|frame| pipeline_state(frame, FUNCTION, state))?;
        Pipeline::Render(
            this.gpu
                .cached_render_pipeline(program, &state)
                .map_err(|problem| gpu_error(FUNCTION, &problem))?,
        )
    };
    Ok(Owned(LuaGpuPipeline {
        gpu: this.gpu.clone(),
        pipeline: Rc::new(pipeline),
    }))
}

/// A render pipeline's state: `{ colors, depth, topology, cull, clockwise }`.
fn pipeline_state(frame: &Frame<'_>, function: &str, state: ValueView<'_>) -> l3i::Result<PipelineState> {
    let state = options(frame, function, "the state", state, &STATE_KEYS)?;
    let topology = match choice(
        frame,
        function,
        &state,
        "topology",
        &["triangles", "triangleStrip", "lines", "lineStrip", "points"],
        "triangles",
    )? {
        "triangleStrip" => Topology::TriangleStrip,
        "lines" => Topology::Lines,
        "lineStrip" => Topology::LineStrip,
        "points" => Topology::Points,
        _ => Topology::Triangles,
    };
    let cull = match choice(frame, function, &state, "cull", &["none", "front", "back"], "none")? {
        "front" => Cull::Front,
        "back" => Cull::Back,
        _ => Cull::None,
    };
    let mut colors = Vec::new();
    let listed = state.raw_get(frame, "colors")?;
    if !listed.is_nil() {
        table(function, "colors", listed)?.for_each_array(frame, |frame, _, color| {
            colors.push(color_state(frame, function, color)?);
            Ok(())
        })?;
    }
    let depth = state.raw_get(frame, "depth")?;
    let depth = if depth.is_nil() {
        None
    } else {
        Some(depth_state(frame, function, depth)?)
    };
    Ok(PipelineState {
        topology,
        cull,
        clockwise: flag(frame, function, &state, "clockwise", false)?,
        colors,
        depth,
        multisampled: flag(frame, function, &state, "multisampled", false)?,
    })
}

/// A color target: a format's name, or `{ format, blend }`.
fn color_state(frame: &Frame<'_>, function: &str, color: ValueView<'_>) -> l3i::Result<ColorState> {
    if color.is_string() {
        return Ok(ColorState::new(format_of(function, color.read::<&str>()?)?));
    }
    let color = options(frame, function, "a color target", color, &COLOR_KEYS)?;
    let blend = match choice(
        frame,
        function,
        &color,
        "blend",
        &["replace", "alpha", "premultiplied", "additive"],
        "replace",
    )? {
        "alpha" => Blend::Alpha,
        "premultiplied" => Blend::Premultiplied,
        "additive" => Blend::Additive,
        _ => Blend::Replace,
    };
    Ok(ColorState {
        format: named_format(frame, function, &color)?,
        blend,
    })
}

/// The depth target: a format's name, or `{ format, compare, write }`.
fn depth_state(frame: &Frame<'_>, function: &str, depth: ValueView<'_>) -> l3i::Result<DepthState> {
    if depth.is_string() {
        return Ok(DepthState {
            format: format_of(function, depth.read::<&str>()?)?,
            compare: Compare::Less,
            write: true,
        });
    }
    let depth = options(frame, function, "the depth target", depth, &DEPTH_KEYS)?;
    Ok(DepthState {
        format: named_format(frame, function, &depth)?,
        compare: compare_of(choice(frame, function, &depth, "compare", &COMPARES, "less")?),
        write: flag(frame, function, &depth, "write", true)?,
    })
}

fn named_format(frame: &Frame<'_>, function: &str, table: &TableView<'_>) -> l3i::Result<crate::render::TextureFormat> {
    let format = table.raw_get(frame, "format")?;
    if !format.is_string() {
        return Err(error(format!(
            "{function}: format is a format's name, such as \"rgba8Unorm\", not {}",
            shown(format)
        )));
    }
    format_of(function, format.read::<&str>()?)
}

/// The frequency named `name`.
pub fn frequency_of(function: &str, view: ValueView<'_>) -> l3i::Result<Frequency> {
    let found = view
        .is_string()
        .then(|| view.read::<&str>().ok())
        .flatten()
        .and_then(|name| Frequency::ALL.into_iter().find(|frequency| frequency.name() == name));
    found.ok_or_else(|| {
        error(format!(
            "{function}: a frequency is pass, material, or draw, not {}",
            shown(view)
        ))
    })
}

/// A resource a bind group is made from, held while it is made.
enum Held {
    Buffer(Arc<Buffer>),
    Texture(Arc<Texture>),
    Sampler(Arc<Sampler>),
}

/// What `slot`, named `name`, is, as errors name it.
pub fn slot_name(slot: Slot, name: &str) -> String {
    match slot {
        Slot::Uniform(_) => "the uniform block".to_owned(),
        Slot::Resource(_, kind) if kind.is_sampler() => format!("{name}'s sampler"),
        Slot::Resource(..) => format!("the texture {name}"),
        Slot::Storage(..) => format!("the storage buffer {name}"),
    }
}

/// `pipeline:bind(frequency, bindings)`: `bindings` by binding number.
fn bind(
    this: &LuaGpuPipeline,
    call: &Call<'_>,
    frequency: ValueView<'_>,
    bindings: ValueView<'_>,
) -> l3i::Result<Owned<LuaGpuBindGroup>> {
    const FUNCTION: &str = "GpuPipeline:bind";
    let frequency = frequency_of(FUNCTION, frequency)?;
    let group = &this.pipeline.groups()[frequency as usize];
    let bindings = table(FUNCTION, "the bindings", bindings)?;
    let mut held: Vec<(Slot, Held)> = Vec::new();
    call.with_frame(|frame| {
        bindings.for_each(frame, |_, key, value| {
            let number = whole(FUNCTION, "a binding number", Some(key), None)?;
            let Some((_, slot, name)) = group.slots.iter().find(|(declared, ..)| *declared == number) else {
                let declared: Vec<String> = group
                    .slots
                    .iter()
                    .map(|(number, slot, name)| format!("{number} ({})", slot_name(*slot, name)))
                    .collect();
                let declared = if declared.is_empty() {
                    "nothing".to_owned()
                } else {
                    declared.join(", ")
                };
                return Err(error(format!(
                    "{FUNCTION}: the program binds nothing at {frequency} binding {number}: it binds {declared}"
                )));
            };
            let what = slot_name(*slot, name);
            let resource = match slot {
                Slot::Uniform(_) | Slot::Storage(..) => {
                    receiver::<LuaGpuBuffer>(value).map(|buffer| Held::Buffer(buffer.buffer.clone()))
                }
                Slot::Resource(_, kind) if kind.is_sampler() => {
                    receiver::<LuaGpuSampler>(value).map(|sampler| Held::Sampler(sampler.sampler.clone()))
                }
                Slot::Resource(..) => receiver::<LuaGpuTexture>(value).map(|texture| Held::Texture(texture.texture.clone())),
            };
            let Some(resource) = resource else {
                let expected = match slot {
                    Slot::Uniform(_) | Slot::Storage(..) => "a GpuBuffer",
                    Slot::Resource(_, kind) if kind.is_sampler() => "a GpuSampler",
                    Slot::Resource(..) => "a GpuTexture",
                };
                return Err(error(format!(
                    "{FUNCTION}: {frequency} binding {number} is {what}: expected {expected}, not {}",
                    kind(value)
                )));
            };
            held.push((*slot, resource));
            Ok(())
        })
    })?;
    let bindings: Vec<Binding<'_>> = held
        .iter()
        .map(|(slot, resource)| match (slot, resource) {
            (Slot::Storage(id, ..), Held::Buffer(buffer)) => Binding::Storage(*id, buffer),
            (_, Held::Buffer(buffer)) => Binding::Uniform(buffer),
            (Slot::Resource(id, _), Held::Texture(texture)) => Binding::Texture(*id, texture),
            (Slot::Resource(id, _), Held::Sampler(sampler)) => Binding::Sampler(*id, sampler),
            _ => unreachable!("a texture or sampler is held only for a resource slot"),
        })
        .collect();
    let group = match &*this.pipeline {
        Pipeline::Render(pipeline) => this.gpu.bind_group(pipeline, frequency, &bindings),
        Pipeline::Compute(pipeline) => this.gpu.compute_bind_group(pipeline, frequency, &bindings),
    }
    .map_err(|problem| gpu_error(FUNCTION, &problem))?;
    Ok(Owned(LuaGpuBindGroup { group: Arc::new(group) }))
}

/// A pass target, held while the pass is drawn.
struct HeldTarget {
    texture: Arc<Texture>,
    level: u32,
    layer: u32,
    load: LoadOp,
    store: StoreOp,
    /// The texture its samples resolve into, and its level and layer.
    resolve: Option<(Arc<Texture>, u32, u32)>,
}

impl HeldTarget {
    fn target(&self) -> Target<'_> {
        Target {
            texture: &self.texture,
            level: self.level,
            layer: self.layer,
            load: self.load,
            store: self.store,
            resolve: self.resolve.as_ref().map(|(texture, level, layer)| Resolve {
                texture,
                level: *level,
                layer: *layer,
            }),
        }
    }
}

/// A draw, held while the pass is drawn.
struct HeldDraw {
    pipeline: Rc<Pipeline>,
    groups: [Option<Arc<BindGroup>>; 3],
    vertices: Option<Arc<Buffer>>,
    instances: Option<Arc<Buffer>>,
    indices: Option<(Arc<Buffer>, IndexFormat)>,
    count: u32,
    instance_count: u32,
    indirect: Option<(Arc<Buffer>, u64)>,
}

/// A draw's `indirect` buffer and its `indirectOffset`, if it draws what the GPU counted.
pub fn indirect_field(frame: &Frame<'_>, function: &str, table: &TableView<'_>) -> l3i::Result<Option<(Arc<Buffer>, u64)>> {
    let Some(buffer) = buffer_field(frame, function, table, "indirect")? else {
        return Ok(None);
    };
    let offset = whole(
        function,
        "the indirect offset",
        Some(table.raw_get(frame, "indirectOffset")?),
        Some(0),
    )?;
    Ok(Some((buffer, u64::from(offset))))
}

/// A draw's `count`, which an indirect draw leaves to the GPU.
pub fn count_field(frame: &Frame<'_>, function: &str, table: &TableView<'_>, indirect: bool) -> l3i::Result<u32> {
    whole(
        function,
        "the count",
        Some(table.raw_get(frame, "count")?),
        indirect.then_some(0),
    )
}

/// A target: a texture, loaded and stored, or `{ texture, clear, level, layer, store }`.
fn target(frame: &Frame<'_>, function: &str, what: &str, view: ValueView<'_>) -> l3i::Result<HeldTarget> {
    if let Some(texture) = receiver::<LuaGpuTexture>(view) {
        return Ok(HeldTarget {
            texture: texture.texture.clone(),
            level: 0,
            layer: 0,
            load: LoadOp::Load,
            store: StoreOp::Store,
            resolve: None,
        });
    }
    let table = options(frame, function, &format!("{what}, a GpuTexture or"), view, &TARGET_KEYS)?;
    let texture = table.raw_get(frame, "texture")?;
    let Some(texture) = receiver::<LuaGpuTexture>(texture) else {
        return Err(error(format!(
            "{function}: {what}'s texture is a GpuTexture, not {}",
            kind(texture)
        )));
    };
    let clear = table.raw_get(frame, "clear")?;
    let load = if clear.is_nil() {
        LoadOp::Load
    } else {
        LoadOp::Clear(clear_value(frame, function, what, texture.texture.desc().format, clear)?)
    };
    Ok(HeldTarget {
        texture: texture.texture.clone(),
        level: whole(function, "the level", Some(table.raw_get(frame, "level")?), Some(0))?,
        layer: whole(function, "the layer", Some(table.raw_get(frame, "layer")?), Some(0))?,
        load,
        store: if flag(frame, function, &table, "store", true)? {
            StoreOp::Store
        } else {
            StoreOp::Discard
        },
        resolve: resolve_target(frame, function, what, table.raw_get(frame, "resolve")?)?,
    })
}

/// Where a target resolves: nothing, a texture's level 0 of layer 0, or `{ texture, level,
/// layer }`.
fn resolve_target(
    frame: &Frame<'_>,
    function: &str,
    what: &str,
    view: ValueView<'_>,
) -> l3i::Result<Option<(Arc<Texture>, u32, u32)>> {
    if view.is_nil() {
        return Ok(None);
    }
    if let Some(texture) = receiver::<LuaGpuTexture>(view) {
        return Ok(Some((texture.texture.clone(), 0, 0)));
    }
    let what = format!("{what}'s resolve");
    let table = options(frame, function, &format!("{what}, a GpuTexture or"), view, &RESOLVE_KEYS)?;
    let texture = table.raw_get(frame, "texture")?;
    let Some(texture) = receiver::<LuaGpuTexture>(texture) else {
        return Err(error(format!(
            "{function}: {what}'s texture is a GpuTexture, not {}",
            kind(texture)
        )));
    };
    Ok(Some((
        texture.texture.clone(),
        whole(function, "the level", Some(table.raw_get(frame, "level")?), Some(0))?,
        whole(function, "the layer", Some(table.raw_get(frame, "layer")?), Some(0))?,
    )))
}

/// What a target of `format` is cleared to: a depth, or four channels of its kind.
pub fn clear_value(
    frame: &Frame<'_>,
    function: &str,
    what: &str,
    format: crate::render::TextureFormat,
    clear: ValueView<'_>,
) -> l3i::Result<ClearValue> {
    if format.is_depth() {
        // Depth is cleared within 0..=1; the pass checks the range.
        #[allow(clippy::cast_possible_truncation)]
        return Ok(ClearValue::Depth(number(function, "a depth clear", clear)? as f32));
    }
    let channels = table(function, &format!("{what}'s clear, four channels,"), clear)?;
    let mut values = [0.0; 4];
    let mut count = 0;
    channels.for_each_array(frame, |_, index, value| {
        if let Some(slot) = usize::try_from(index - 1).ok().and_then(|index| values.get_mut(index)) {
            *slot = number(function, "a clear channel", value)?;
        }
        count = index;
        Ok(())
    })?;
    if count != 4 {
        return Err(error(format!("{function}: {what}'s clear is four channels, not {count}")));
    }
    let integers = |low: f64, high: f64| -> l3i::Result<[f64; 4]> {
        if values
            .iter()
            .all(|value| value.fract() == 0.0 && (low..=high).contains(value))
        {
            return Ok(values);
        }
        Err(error(format!(
            "{function}: a {format} target clears to whole numbers from {low} to {high}, not {values:?}"
        )))
    };
    // Each channel is checked to be whole and in its type's range first.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(if format.is_uint() {
        ClearValue::Uint(integers(0.0, f64::from(u32::MAX))?.map(|value| value as u32))
    } else if format.is_sint() {
        ClearValue::Sint(integers(f64::from(i32::MIN), f64::from(i32::MAX))?.map(|value| value as i32))
    } else {
        ClearValue::Color(values.map(|value| value as f32))
    })
}

/// The bind groups a draw or dispatch gives, by frequency.
pub fn groups(frame: &Frame<'_>, function: &str, table: &TableView<'_>) -> l3i::Result<[Option<Arc<BindGroup>>; 3]> {
    let mut groups = [None, None, None];
    for frequency in Frequency::ALL {
        let view = table.raw_get(frame, frequency.name())?;
        if view.is_nil() {
            continue;
        }
        let Some(group) = receiver::<LuaGpuBindGroup>(view) else {
            return Err(error(format!(
                "{function}: {frequency} is a GpuBindGroup, not {}",
                kind(view)
            )));
        };
        groups[frequency as usize] = Some(group.group.clone());
    }
    Ok(groups)
}

/// A buffer field, if it is given.
pub fn buffer_field(
    frame: &Frame<'_>,
    function: &str,
    table: &TableView<'_>,
    key: &str,
) -> l3i::Result<Option<Arc<Buffer>>> {
    let view = table.raw_get(frame, key)?;
    if view.is_nil() {
        return Ok(None);
    }
    receiver::<LuaGpuBuffer>(view)
        .map(|buffer| Some(buffer.buffer.clone()))
        .ok_or_else(|| error(format!("{function}: {key} is a GpuBuffer, not {}", kind(view))))
}

/// The pipeline a draw or dispatch runs.
pub fn pipeline_field(frame: &Frame<'_>, function: &str, table: &TableView<'_>) -> l3i::Result<Rc<Pipeline>> {
    let view = table.raw_get(frame, "pipeline")?;
    receiver::<LuaGpuPipeline>(view)
        .map(|pipeline| pipeline.pipeline.clone())
        .ok_or_else(|| error(format!("{function}: pipeline is a GpuPipeline, not {}", kind(view))))
}

/// One draw: `{ pipeline, pass, material, draw, vertices, instances, indices, indexFormat, count,
/// instanceCount }`.
fn draw(frame: &Frame<'_>, function: &str, view: ValueView<'_>) -> l3i::Result<HeldDraw> {
    let table = options(frame, function, "a draw", view, &DRAW_KEYS)?;
    let indices = buffer_field(frame, function, &table, "indices")?;
    let index_format = match choice(frame, function, &table, "indexFormat", &["u16", "u32"], "u32")? {
        "u16" => IndexFormat::U16,
        _ => IndexFormat::U32,
    };
    let indirect = indirect_field(frame, function, &table)?;
    Ok(HeldDraw {
        pipeline: pipeline_field(frame, function, &table)?,
        groups: groups(frame, function, &table)?,
        vertices: buffer_field(frame, function, &table, "vertices")?,
        instances: buffer_field(frame, function, &table, "instances")?,
        indices: indices.map(|buffer| (buffer, index_format)),
        count: count_field(frame, function, &table, indirect.is_some())?,
        indirect,
        instance_count: whole(
            function,
            "the instance count",
            Some(table.raw_get(frame, "instanceCount")?),
            Some(1),
        )?,
    })
}

/// `device:render { colors, depth, draws }`.
fn render(this: &LuaGpuDevice, call: &Call<'_>, pass: ValueView<'_>) -> l3i::Result<()> {
    const FUNCTION: &str = "GpuDevice:render";
    if !pass.is_table() {
        return Err(error(format!("{FUNCTION} takes a table, not {}", kind(pass))));
    }
    let (colors, depth, draws) = call.with_frame(|frame| {
        check_options(frame, pass, FUNCTION, &RENDER_KEYS)?;
        let pass = pass.as_table()?;
        let mut colors = Vec::new();
        let listed = pass.raw_get(frame, "colors")?;
        if !listed.is_nil() {
            table(FUNCTION, "colors", listed)?.for_each_array(frame, |frame, index, color| {
                colors.push(target(frame, FUNCTION, &format!("color target {}", index - 1), color)?);
                Ok(())
            })?;
        }
        let depth = pass.raw_get(frame, "depth")?;
        let depth = if depth.is_nil() {
            None
        } else {
            Some(target(frame, FUNCTION, "the depth target", depth)?)
        };
        let mut draws = Vec::new();
        let listed = pass.raw_get(frame, "draws")?;
        if !listed.is_nil() {
            table(FUNCTION, "draws", listed)?.for_each_array(frame, |frame, _, one| {
                draws.push(draw(frame, FUNCTION, one)?);
                Ok(())
            })?;
        }
        Ok((colors, depth, draws))
    })?;
    let color_targets: Vec<Target<'_>> = colors.iter().map(HeldTarget::target).collect();
    let depth_target = depth.as_ref().map(HeldTarget::target);
    let mut prepared = Vec::with_capacity(draws.len());
    for (index, held) in draws.iter().enumerate() {
        let Pipeline::Render(pipeline) = &*held.pipeline else {
            return Err(error(format!(
                "{FUNCTION}: draw {index}'s pipeline computes: dispatch it with GpuDevice:dispatch"
            )));
        };
        prepared.push(Draw {
            pipeline,
            groups: [0, 1, 2].map(|frequency| held.groups[frequency].as_deref()),
            vertices: held.vertices.as_deref(),
            instances: held.instances.as_deref(),
            indices: held.indices.as_ref().map(|(buffer, format)| (&**buffer, *format)),
            count: held.count,
            instance_count: held.instance_count,
            indirect: held.indirect.as_ref().map(|(buffer, offset)| (&**buffer, *offset)),
        });
    }
    committed(&this.gpu, FUNCTION)?;
    this.gpu
        .render(&color_targets, depth_target.as_ref(), &prepared)
        .map_err(|problem| gpu_error(FUNCTION, &problem))
}

/// `device:dispatch { pipeline, pass, material, draw, workgroups }`.
fn dispatch(this: &LuaGpuDevice, call: &Call<'_>, work: ValueView<'_>) -> l3i::Result<()> {
    const FUNCTION: &str = "GpuDevice:dispatch";
    if !work.is_table() {
        return Err(error(format!("{FUNCTION} takes a table, not {}", kind(work))));
    }
    let (pipeline, groups, workgroups) = call.with_frame(|frame| {
        check_options(frame, work, FUNCTION, &DISPATCH_KEYS)?;
        let work = work.as_table()?;
        Ok((
            pipeline_field(frame, FUNCTION, &work)?,
            groups(frame, FUNCTION, &work)?,
            workgroups(frame, FUNCTION, work.raw_get(frame, "workgroups")?)?,
        ))
    })?;
    let Pipeline::Compute(compute) = &*pipeline else {
        return Err(error(format!(
            "{FUNCTION}: the pipeline draws: draw it with GpuDevice:render"
        )));
    };
    committed(&this.gpu, FUNCTION)?;
    this.gpu
        .dispatch(compute, [0, 1, 2].map(|frequency| groups[frequency].as_deref()), workgroups)
        .map_err(|problem| gpu_error(FUNCTION, &problem))
}

/// How many workgroups a dispatch runs on each axis: a count, or one to three of them, the rest 1.
pub fn workgroups(frame: &Frame<'_>, function: &str, counts: ValueView<'_>) -> l3i::Result<[u32; 3]> {
    let mut workgroups = [1; 3];
    if !counts.is_table() {
        workgroups[0] = whole(function, "workgroups", Some(counts), None)?;
        return Ok(workgroups);
    }
    let mut count = 0;
    counts.as_table()?.for_each_array(frame, |_, index, value| {
        if let Some(slot) = usize::try_from(index - 1).ok().and_then(|index| workgroups.get_mut(index)) {
            *slot = whole(function, "a workgroup count", Some(value), None)?;
        }
        count = index;
        Ok(())
    })?;
    if !(1..=3).contains(&count) {
        return Err(error(format!(
            "{function}: workgroups is a count, or one to three of them, not {count}"
        )));
    }
    Ok(workgroups)
}

/// Declares the device's methods that make pipelines and run them.
pub fn device_methods(device: &mut UserdataBuilder<'_, LuaGpuDevice>) {
    device.method("pipeline", pipeline).untyped();
    device.method("render", render).untyped();
    device.method("dispatch", dispatch).untyped();
}

/// Declares the pipeline and bind group types.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut pipeline = d.userdata::<LuaGpuPipeline>(LuaGpuPipeline::NAME);
    pipeline
        .tag(TagPolicy::Never)
        .doc("A shader program made ready to draw or to compute.");
    pipeline
        .getter("kind", |this: &LuaGpuPipeline| match &*this.pipeline {
            Pipeline::Render(_) => "render",
            Pipeline::Compute(_) => "compute",
        })
        .untyped();
    pipeline.method("bind", bind).untyped();
    crate::lua::plan::metamethod(&mut pipeline, "__tostring", |this: &LuaGpuPipeline| match &*this.pipeline {
        Pipeline::Render(pipeline) => {
            let state = pipeline.state();
            let depth = if state.depth.is_some() { ", depth" } else { "" };
            let targets = state.colors.len();
            let plural = if targets == 1 { "" } else { "s" };
            format!("GpuPipeline(render, {targets} color target{plural}{depth})")
        }
        Pipeline::Compute(pipeline) => {
            let [x, y, z] = pipeline.workgroup_size();
            format!("GpuPipeline(compute {x}×{y}×{z})")
        }
    });

    let mut group = d.userdata::<LuaGpuBindGroup>(LuaGpuBindGroup::NAME);
    group
        .tag(TagPolicy::Never)
        .doc("What one frequency binds for a pipeline: its uniform block, textures, samplers, and storage buffers.");
    group
        .getter("frequency", |this: &LuaGpuBindGroup| this.group.frequency().name())
        .untyped();
    crate::lua::plan::metamethod(&mut group, "__tostring", |this: &LuaGpuBindGroup| {
        format!("GpuBindGroup({})", this.group.frequency())
    });
}
