//! Draw sets from Luau: draws kept on the device, which every plan's draw passes take by their
//! tags, frame after frame, until they are removed.

use std::rc::Rc;
use std::sync::Arc;

use l3i::bind::{Call, StackResults};
use l3i::extension::{ExtensionDescriptor, TagPolicy, UserdataBuilder};
use l3i::stack::{Scope, ValueView};
use l3i::userdata::{Owned, Userdata};

use super::device::LuaGpuDevice;
use super::pipeline::{Pipeline, buffer_field, gpu_error, groups, options, pipeline_field, table};
use super::resources::{choice, shown, whole};
use crate::gpu::{DrawSet, DrawSetDesc, DrawSets, IndexFormat};
use crate::lua::{error, kind};

/// A draw kept on a device, and the device's draw sets it is drawn from.
#[derive(Debug)]
pub struct LuaGpuDrawSet {
    set: Arc<DrawSet>,
    sets: Rc<DrawSets>,
}

// SAFETY: plain Rust state; the destructor never touches the Lua API.
unsafe impl Userdata for LuaGpuDrawSet {
    const NAME: &'static str = "jess.gpu.DrawSet";
}

const DRAW_SET_KEYS: [&str; 10] = [
    "count",
    "draw",
    "indexFormat",
    "indices",
    "instanceCount",
    "instances",
    "material",
    "pipeline",
    "tags",
    "vertices",
];

/// `device:drawSet { pipeline, material, draw, vertices, instances, indices, indexFormat, count,
/// instanceCount, tags }`: a draw set, drawn from the next frame on.
fn draw_set(this: &LuaGpuDevice, call: &Call<'_>, desc: ValueView<'_>) -> l3i::Result<Owned<LuaGpuDrawSet>> {
    const FUNCTION: &str = "GpuDevice:drawSet";
    if !desc.is_table() {
        return Err(error(format!("{FUNCTION} takes a table, not {}", kind(desc))));
    }
    let desc = call.with_frame(|frame| {
        let fields = options(frame, FUNCTION, "the draw set", desc, &DRAW_SET_KEYS)?;
        let pipeline = pipeline_field(frame, FUNCTION, &fields)?;
        let Pipeline::Render(pipeline) = &*pipeline else {
            return Err(error(format!(
                "{FUNCTION}: the pipeline computes: a draw set draws, so it needs a render pipeline"
            )));
        };
        let [_, material, draw] = groups(frame, FUNCTION, &fields)?;
        let indices = buffer_field(frame, FUNCTION, &fields, "indices")?;
        let index_format = match choice(frame, FUNCTION, &fields, "indexFormat", &["u16", "u32"], "u32")? {
            "u16" => IndexFormat::U16,
            _ => IndexFormat::U32,
        };
        let mut tags = Vec::new();
        let listed = fields.raw_get(frame, "tags")?;
        if !listed.is_nil() {
            table(FUNCTION, "tags", listed)?.for_each_array(frame, |_, _, tag| {
                if !tag.is_string() {
                    return Err(error(format!("{FUNCTION}: a tag is a string, not {}", shown(tag))));
                }
                tags.push(tag.read::<&str>()?.to_owned());
                Ok(())
            })?;
        }
        Ok(DrawSetDesc {
            pipeline: pipeline.clone(),
            material,
            draw,
            vertices: buffer_field(frame, FUNCTION, &fields, "vertices")?,
            instances: buffer_field(frame, FUNCTION, &fields, "instances")?,
            indices: indices.map(|buffer| (buffer, index_format)),
            count: whole(FUNCTION, "the count", Some(fields.raw_get(frame, "count")?), None)?,
            instance_count: whole(
                FUNCTION,
                "the instance count",
                Some(fields.raw_get(frame, "instanceCount")?),
                Some(1),
            )?,
            tags,
        })
    })?;
    let set = Arc::new(DrawSet::new(desc).map_err(|problem| gpu_error(FUNCTION, &problem))?);
    this.sets.add(&set);
    Ok(Owned(LuaGpuDrawSet {
        set,
        sets: this.sets.clone(),
    }))
}

/// Declares the device's method that makes draw sets.
pub fn device_methods(device: &mut UserdataBuilder<'_, LuaGpuDevice>) {
    device.method("drawSet", draw_set).untyped();
}

/// Declares the draw set type.
pub fn describe(d: &mut ExtensionDescriptor) {
    let mut set = d.userdata::<LuaGpuDrawSet>(LuaGpuDrawSet::NAME);
    set.tag(TagPolicy::Never)
        .doc("A draw kept on its device, which plans' draw passes take by its tags every frame.");
    set.property(
        "count",
        |this: &LuaGpuDrawSet| this.set.count(),
        |this: &LuaGpuDrawSet, count: ValueView<'_>| -> l3i::Result<()> {
            const FUNCTION: &str = "GpuDrawSet.count";
            let count = whole(FUNCTION, "the count", Some(count), None)?;
            this.set.set_count(count).map_err(|problem| gpu_error(FUNCTION, &problem))
        },
    )
    .untyped();
    set.property(
        "instanceCount",
        |this: &LuaGpuDrawSet| this.set.instance_count(),
        |this: &LuaGpuDrawSet, count: ValueView<'_>| -> l3i::Result<()> {
            const FUNCTION: &str = "GpuDrawSet.instanceCount";
            let count = whole(FUNCTION, "the instance count", Some(count), None)?;
            this.set
                .set_instance_count(count)
                .map_err(|problem| gpu_error(FUNCTION, &problem))
        },
    )
    .untyped();
    set.property(
        "visible",
        |this: &LuaGpuDrawSet| this.set.visible(),
        |this: &LuaGpuDrawSet, visible: ValueView<'_>| -> l3i::Result<()> {
            if !visible.is_boolean() {
                return Err(error(format!(
                    "GpuDrawSet.visible: visible is a boolean, not {}",
                    shown(visible)
                )));
            }
            this.set.set_visible(visible.read::<bool>()?);
            Ok(())
        },
    )
    .untyped();
    set.getter("tags", |this: &LuaGpuDrawSet, call: &Call<'_>| -> l3i::Result<StackResults> {
        l3i::stack::push_list(call, this.set.tags().iter(), |frame, tag| frame.push_string(tag).map(drop))?;
        Ok(StackResults)
    })
    .untyped();
    set.getter("added", |this: &LuaGpuDrawSet| this.sets.contains(&this.set))
        .untyped();
    set.method("add", |this: &LuaGpuDrawSet| this.sets.add(&this.set)).untyped();
    set.method("remove", |this: &LuaGpuDrawSet| this.sets.remove(&this.set))
        .untyped();
    crate::lua::plan::metamethod(&mut set, "__tostring", |this: &LuaGpuDrawSet| {
        let set = &this.set;
        let noun = if set.count() == 1 { "vertex" } else { "vertices" };
        let instances = match set.instance_count() {
            1 => String::new(),
            count => format!(" ×{count}"),
        };
        let tags = if set.tags().is_empty() {
            String::new()
        } else {
            format!(", {}", set.tags().join(" "))
        };
        let hidden = if set.visible() { "" } else { ", hidden" };
        format!("GpuDrawSet({} {noun}{instances}{tags}{hidden})", set.count())
    });
}
