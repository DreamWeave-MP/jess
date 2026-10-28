//! `jess.gpu` from Luau: the device a script opens under `gpu.device`, what it reports, and the
//! buffers, textures, and samplers it makes.

use super::common;

/// Without `gpu.device`, asking for the device is denied, naming the function and the capability.
#[test]
fn the_device_needs_its_capability() {
    let runtime = common::runtime();
    let error = runtime.exec("jess.gpu.device()").unwrap_err().to_string();
    assert!(error.contains("jess.gpu.device") && error.contains("gpu.device"), "{error}");
}

/// A runtime holding `gpu.device`.
fn device_runtime() -> l3i::Runtime {
    jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &["gpu.device".to_owned()]).unwrap()
}

/// A jess built without the `gpu` feature has the member, and says it has no GPU.
#[cfg(not(feature = "gpu"))]
#[test]
fn a_jess_without_the_gpu_feature_has_no_device() {
    let error = device_runtime().exec("jess.gpu.device()").unwrap_err().to_string();
    assert!(error.contains("built without the gpu feature"), "{error}");
}

/// A runtime holding `gpu.device` on a machine with a GPU, or `None` after saying `test` is
/// skipped.
#[cfg(feature = "gpu")]
fn gpu_runtime(test: &str) -> Option<l3i::Runtime> {
    crate::gpu::gpu(test)?;
    Some(device_runtime())
}

/// The device is the runtime's, opened once, describing its adapter.
#[cfg(feature = "gpu")]
#[test]
fn the_device_describes_its_adapter() {
    let Some(runtime) = gpu_runtime("the_device_describes_its_adapter") else {
        return;
    };
    runtime
        .exec(
            r"
            local device = jess.gpu.device()
            assert(device.name ~= '' and device.backend ~= '', tostring(device))
            assert(table.find({ 'discrete', 'integrated', 'virtual', 'cpu', 'other' }, device.kind), device.kind)
            assert(device.limits.textureSize >= 2048 and device.limits.workgroups >= 1)
            assert(string.find(tostring(device), 'GpuDevice(', 1, true) == 1)
            assert(jess.gpu.device().name == device.name)
            assert(require('@jess/gpu') == jess.gpu)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A buffer takes bytes from a Luau `buffer` or a string, whole or a range of them, and reads back
/// what it holds, whole or a range.
#[cfg(feature = "gpu")]
#[test]
fn a_buffer_round_trips_bytes() {
    let Some(runtime) = gpu_runtime("a_buffer_round_trips_bytes") else {
        return;
    };
    runtime
        .exec(
            r"
            local device = jess.gpu.device()
            local gpuBuffer = device:buffer(16, { 'vertex', 'storage' })
            assert(gpuBuffer.size == 16)
            assert(gpuBuffer.usage[1] == 'vertex' and gpuBuffer.usage[2] == 'storage' and #gpuBuffer.usage == 2)
            assert(tostring(gpuBuffer) == 'GpuBuffer(16 bytes, vertex, storage)', tostring(gpuBuffer))

            local bytes = buffer.create(16)
            for index = 0, 15 do
                buffer.writeu8(bytes, index, index * 3)
            end
            gpuBuffer:write(0, bytes)
            assert(buffer.tostring(gpuBuffer:read()) == buffer.tostring(bytes))

            gpuBuffer:write(4, 'abcdefgh', 2, 4)
            assert(buffer.readstring(gpuBuffer:read(4, 4), 0, 4) == 'cdef')
            assert(buffer.len(gpuBuffer:read(8)) == 8)
            assert(device:buffer(4, 'uniform').usage[1] == 'uniform')

            -- A read later has what its copy saw, whatever is written after it is asked for.
            local later = gpuBuffer:readLater(4, 4)
            gpuBuffer:write(4, 'wxyz')
            local polls = 0
            local found = later:poll()
            while not found do
                polls += 1
                assert(polls < 10000000, 'the GPU never finished a copy of four bytes')
                found = later:poll()
            end
            assert(later.ready and tostring(later) == 'GpuRead(ready)', tostring(later))
            assert(buffer.tostring(found) == 'cdef' and buffer.tostring(later:wait()) == 'cdef')
            local texture = device:texture({ width = 2, height = 1, format = 'rgba8Unorm', usage = 'sampled' })
            texture:write('abcdefgh')
            assert(buffer.tostring(texture:readLater():wait()) == 'abcdefgh')
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A texture takes a level of a layer's texels and reads them back; its getters say what it is.
#[cfg(feature = "gpu")]
#[test]
fn a_texture_round_trips_texels() {
    let Some(runtime) = gpu_runtime("a_texture_round_trips_texels") else {
        return;
    };
    runtime
        .exec(
            r"
            local device = jess.gpu.device()
            local texture = device:texture({
              width = 2,
              height = 2,
              format = 'rgba8Unorm',
              usage = { 'sampled', 'target' },
              shape = '2dArray',
              layers = 3,
              mips = 2,
            })
            assert(texture.width == 2 and texture.height == 2 and texture.format == 'rgba8Unorm')
            assert(texture.shape == '2dArray' and texture.layers == 3 and texture.mips == 2)
            assert(tostring(texture) == 'GpuTexture(2×2 rgba8Unorm, 2dArray)', tostring(texture))

            local texels = buffer.create(16)
            for index = 0, 15 do
                buffer.writeu8(texels, index, 255 - index)
            end
            texture:write(texels, 0, 2)
            assert(buffer.tostring(texture:read(0, 2)) == buffer.tostring(texels))
            texture:write('wxyz', 1, 1)
            assert(buffer.tostring(texture:read(1, 1)) == 'wxyz')

            local plain = device:texture({ width = 4, height = 1, format = 'r32Float', usage = 'sampled' })
            assert(plain.shape == '2d' and plain.layers == 1 and plain.mips == 1)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A sampler is made from names, every one optional.
#[cfg(feature = "gpu")]
#[test]
fn a_sampler_is_made_from_names() {
    let Some(runtime) = gpu_runtime("a_sampler_is_made_from_names") else {
        return;
    };
    runtime
        .exec(
            r"
            local device = jess.gpu.device()
            assert(tostring(device:sampler()) == 'GpuSampler(nearest)')
            local sampler = device:sampler({ filter = 'linear', mipmapFilter = 'linear', address = 'repeat' })
            assert(tostring(sampler) == 'GpuSampler(linear)', tostring(sampler))
            assert(tostring(device:sampler({ compare = 'lessEqual' })) == 'GpuSampler(nearest, comparing)')
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// What a script gets wrong is named: the function, the value, and what was expected.
#[cfg(feature = "gpu")]
#[test]
fn resource_errors_name_the_mistake() {
    let Some(runtime) = gpu_runtime("resource_errors_name_the_mistake") else {
        return;
    };
    for (script, expected) in [
        (
            "device:buffer(16, 'vertexx')",
            "GpuDevice:buffer: a usage is one of vertex, index, uniform, storage, indirect, not \"vertexx\"",
        ),
        (
            "device:buffer(1.5, 'vertex')",
            "GpuDevice:buffer: the size is a whole number from 0 to 4294967295, not 1.5",
        ),
        (
            "device:buffer(6, 'vertex')",
            "GpuDevice:buffer: a buffer of 6 bytes is invalid: expected a multiple of 4",
        ),
        ("device:buffer(8, 'vertex'):write(0, 'abc')", "GpuBuffer:write: "),
        (
            "device:buffer(8, 'vertex'):write(0, 'abcd', 2, 4)",
            "GpuBuffer:write: 4 bytes from 2 run past the 4 given",
        ),
        (
            "device:buffer(8, 'vertex'):write(0, {})",
            "GpuBuffer:write: expected a buffer or string, not table",
        ),
        (
            "device:texture({ width = 1, height = 1, format = 'rgba8', usage = 'sampled' })",
            "GpuDevice:texture: no format \"rgba8\": expected one of rgba8Unorm, rgba8UnormSrgb, ",
        ),
        (
            "device:texture({ width = 1, height = 1, format = 'rgba8Unorm', usage = 'sampled', layer = 2 })",
            "unknown GpuDevice:texture option 'layer': expected depth, format, height, layers, mips, shape, usage, or width",
        ),
        (
            "device:texture({ width = 1, height = 1, usage = 'sampled' })",
            "GpuDevice:texture: format is a format's name",
        ),
        ("device:texture('big')", "GpuDevice:texture takes a table, not string"),
        (
            "device:sampler({ address = 'wrap' })",
            "GpuDevice:sampler: address is one of clamp, repeat, mirror, not \"wrap\"",
        ),
    ] {
        let error = runtime
            .exec(&format!("local device = jess.gpu.device()\n{script}"))
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{script}: {error}");
    }
}

/// A tinted, textured triangle drawn from Luau over a cleared target: a render pipeline, a
/// material bind group by the binding numbers the program's layout reports, a vertex buffer, a
/// depth target, and the pixels read back.
#[cfg(feature = "gpu")]
#[test]
fn a_script_draws_a_triangle() {
    let Some(runtime) = gpu_runtime("a_script_draws_a_triangle") else {
        return;
    };
    runtime
        .exec(
            r"
            local shader = require('@jess/shader')
            local program = shader.program({
              attributes = { position = shader.vec2 },
              material = { tint = shader.vec4, diffuse = shader.texture2d },
              vertex = function(input)
                return { position = shader.vec4(input.attributes.position, 0.5, 1) }
              end,
              fragment = function(input)
                return shader.sample(input.material.diffuse, shader.vec2(0.5, 0.5)) * input.material.tint
              end,
            })

            local device = jess.gpu.device()
            local pipeline = device:pipeline(program, { colors = { 'rgba8Unorm' }, depth = 'depth32Float' })
            assert(pipeline.kind == 'render')
            assert(tostring(pipeline) == 'GpuPipeline(render, 1 color target, depth)', tostring(pipeline))

            local tint = device:buffer(program.layout.material.size, 'uniform')
            local tintBytes = buffer.create(16)
            buffer.writef32(tintBytes, 0, 1)
            buffer.writef32(tintBytes, 12, 1)
            tint:write(0, tintBytes)
            local white = device:texture({ width = 1, height = 1, format = 'rgba8Unorm', usage = 'sampled' })
            white:write('\255\255\255\255')

            local bindings = { [0] = tint }
            for _, binding in program.layout.material.bindings do
              bindings[binding.binding] = if binding.kind == 'Sampler' then device:sampler() else white
            end
            local material = pipeline:bind('material', bindings)
            assert(material.frequency == 'material' and tostring(material) == 'GpuBindGroup(material)')

            local corners = buffer.create(24)
            for index, value in { -1, -1, 3, -1, -1, 3 } do
              buffer.writef32(corners, (index - 1) * 4, value)
            end
            local vertices = device:buffer(24, 'vertex')
            vertices:write(0, corners)

            local color = device:texture({ width = 4, height = 4, format = 'rgba8Unorm', usage = 'target' })
            local depth = device:texture({ width = 4, height = 4, format = 'depth32Float', usage = 'target' })
            device:render({
              colors = { { texture = color, clear = { 0, 0, 1, 1 } } },
              depth = { texture = depth, clear = 1 },
              draws = { { pipeline = pipeline, material = material, vertices = vertices, count = 3 } },
            })
            assert(buffer.tostring(color:read()) == string.rep('\255\0\0\255', 16))

            device:render({ colors = { { texture = color, clear = { 0, 0, 1, 1 } } } })
            assert(buffer.tostring(color:read()) == string.rep('\0\0\255\255', 16))
            device:render({
              colors = { color },
              depth = { texture = depth, clear = 0.25 },
              draws = { { pipeline = pipeline, material = material, vertices = vertices, count = 3 } },
            })
            assert(buffer.tostring(color:read()) == string.rep('\0\0\255\255', 16), 'the depth test passed')

            -- A pass reads its draws one at a time, so ten thousand hold no more of the stack than one.
            local one = { pipeline = pipeline, material = material, vertices = vertices, count = 3 }
            device:render({
              colors = { { texture = color, clear = { 0, 0, 0, 0 } } },
              depth = { texture = depth, clear = 1 },
              draws = table.create(10000, one),
            })
            assert(buffer.tostring(color:read()) == string.rep('\255\0\0\255', 16))
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A compute program run from Luau over storage buffers it reads and writes.
#[cfg(feature = "gpu")]
#[test]
fn a_script_dispatches_a_compute_program() {
    let Some(runtime) = gpu_runtime("a_script_dispatches_a_compute_program") else {
        return;
    };
    runtime
        .exec(
            r"
            local shader = require('@jess/shader')
            local integrate = shader.compute({
              workgroupSize = { 64 },
              pass = {
                positions = shader.storage(shader.vec4, 'readWrite'),
                step = shader.float,
                velocities = shader.storage(shader.vec4),
              },
              compute = function(input)
                local index = input.builtin.globalInvocationId.x
                local pass = input.pass
                shader.when(shader.less(index, shader.arrayLength(pass.positions)), function()
                  shader.write(pass.positions, index, pass.positions[index] + pass.velocities[index] * pass.step)
                end)
              end,
            })

            local device = jess.gpu.device()
            local pipeline = device:pipeline(integrate)
            assert(pipeline.kind == 'compute' and tostring(pipeline) == 'GpuPipeline(compute 64×1×1)')

            local bytes = buffer.create(64)
            for index = 0, 15 do
              buffer.writef32(bytes, index * 4, index)
            end
            local positions, velocities = device:buffer(64, 'storage'), device:buffer(64, 'storage')
            positions:write(0, bytes)
            velocities:write(0, bytes)
            local step = device:buffer(16, 'uniform')
            local stepBytes = buffer.create(16)
            buffer.writef32(stepBytes, 0, 0.5)
            step:write(0, stepBytes)

            local pass = pipeline:bind('pass', { [0] = step, [1] = positions, [2] = velocities })
            device:dispatch({ pipeline = pipeline, pass = pass, workgroups = { 1, 1, 1 } })
            device:dispatch({ pipeline = pipeline, pass = pass, workgroups = 1 })
            local moved = positions:read()
            for index = 0, 15 do
              assert(buffer.readf32(moved, index * 4) == index * 2, `{index}: {buffer.readf32(moved, index * 4)}`)
            end
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// Each mistake [`pipeline_errors_name_the_mistake`] makes, and what its error says.
#[cfg(feature = "gpu")]
const PIPELINE_MISTAKES: [(&str, &str); 20] = [
    (
        "device:pipeline('shader')",
        "GpuDevice:pipeline takes a shader program, not string",
    ),
    (
        "device:pipeline(draw)",
        "GpuDevice:pipeline: a vertex and fragment program needs a state naming its color targets",
    ),
    (
        "device:pipeline(fill, {})",
        "GpuDevice:pipeline: a compute program has no drawing state, and was given table",
    ),
    (
        "device:pipeline(draw, { color = { 'rgba8Unorm' } })",
        "unknown GpuDevice:pipeline option 'color'",
    ),
    (
        "device:pipeline(draw, { colors = { { format = 'rgba8Unorm', blend = 'over' } } })",
        "blend is one of replace, alpha, premultiplied, additive, not \"over\"",
    ),
    (
        "device:pipeline(draw, { colors = {} })",
        "the program writes 1 color outputs, and the pipeline has 0 color targets",
    ),
    (
        "drawing:bind('frame', {})",
        "GpuPipeline:bind: a frequency is pass, material, or draw, not \"frame\"",
    ),
    (
        "drawing:bind('material', { [5] = target })",
        "the program binds nothing at material binding 5: it binds 0 (the uniform block), 1 (the texture diffuse), 2 (diffuse's sampler)",
    ),
    (
        "drawing:bind('material', { [1] = device:sampler() })",
        "material binding 1 is the texture diffuse: expected a GpuTexture, not userdata",
    ),
    (
        "drawing:bind('pass', {})",
        "GpuPipeline:bind: the program binds nothing per pass",
    ),
    (
        "drawing:bind('material', { [1] = target })",
        "GpuPipeline:bind: the material bind group needs exactly one uniform",
    ),
    (
        "device:render({ colors = { { texture = target, clear = { 0, 0, 0 } } } })",
        "GpuDevice:render: color target 0's clear is four channels, not 3",
    ),
    (
        "device:render({ colors = { { texture = target, clear = 1 } } })",
        "GpuDevice:render: color target 0's clear, four channels, is a table, not 1",
    ),
    (
        "device:render({ colors = { 'target' } })",
        "GpuDevice:render: color target 0, a GpuTexture or is a table, not \"target\"",
    ),
    (
        "device:render({ colors = { target }, draws = { { pipeline = computing, count = 3 } } })",
        "GpuDevice:render: draw 0's pipeline computes: dispatch it with GpuDevice:dispatch",
    ),
    (
        "device:render({ colors = { target }, draws = { { pipeline = drawing } } })",
        "GpuDevice:render: the count is a whole number",
    ),
    (
        "device:render({ colors = { target }, draws = { { pipeline = drawing, count = 3 } } })",
        "draw 0 is invalid: its program binds a material group, and the draw gives none",
    ),
    (
        "device:dispatch({ pipeline = drawing, workgroups = 1 })",
        "GpuDevice:dispatch: the pipeline draws: draw it with GpuDevice:render",
    ),
    (
        "device:dispatch({ pipeline = computing, workgroups = 1 })",
        "the dispatch is invalid: its program binds a pass group, and the dispatch gives none",
    ),
    (
        "device:dispatch({ pipeline = computing, workgroups = { 1, 1, 1, 1 } })",
        "GpuDevice:dispatch: workgroups is a count, or one to three of them, not 4",
    ),
];

/// What a script gets wrong about pipelines, bind groups, passes, and dispatches is named.
#[cfg(feature = "gpu")]
#[test]
fn pipeline_errors_name_the_mistake() {
    let Some(runtime) = gpu_runtime("pipeline_errors_name_the_mistake") else {
        return;
    };
    // What every case starts from: a device, a render pipeline whose material binds a uniform
    // block and a texture, a compute pipeline, and a target.
    let prelude = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local draw = shader.program({
              material = { tint = shader.vec4, diffuse = shader.texture2d },
              vertex = function(input) return { position = shader.vec4(0, 0, 0, 1) } end,
              fragment = function(input) return input.material.tint end,
            })
            local fill = shader.compute({
              workgroupSize = 1,
              pass = { values = shader.storage(shader.uint, 'readWrite') },
              compute = function(input) shader.write(input.pass.values, 0, shader.uint(7)) end,
            })
            local drawing = device:pipeline(draw, { colors = { 'rgba8Unorm' } })
            local computing = device:pipeline(fill)
            local target = device:texture({ width = 1, height = 1, format = 'rgba8Unorm', usage = 'target' })
        ";
    for (script, expected) in PIPELINE_MISTAKES {
        let error = runtime.exec(&format!("{prelude}\n{script}")).unwrap_err().to_string();
        assert!(error.contains(expected), "{script}: {error}");
    }
}

/// What `a_script_keeps_draw_sets` runs first.
#[cfg(feature = "gpu")]
const DRAW_SETS: &str = r"
            local shader = require('@jess/shader')
            local program = shader.program({
              attributes = { position = shader.vec2 },
              pass = { tint = shader.vec4 },
              material = { color = shader.vec4 },
              vertex = function(input)
                return { position = shader.vec4(input.attributes.position, 0.5, 1) }
              end,
              fragment = function(input) return input.material.color * input.pass.tint end,
            })
            local device = jess.gpu.device()
            local pipeline = device:pipeline(program, { colors = { 'rgba8Unorm' } })
            local color = device:buffer(16, 'uniform')
            local material = pipeline:bind('material', { [0] = color })
            local vertices = device:buffer(48, 'vertex')

            local set = device:drawSet({
              pipeline = pipeline,
              material = material,
              vertices = vertices,
              count = 6,
              tags = { 'opaque', 'shadow' },
            })
            -- What a script changes is what the next commit applies, all of it at once.
            assert(not set.added and device:commit() and set.added and not device:commit())
            assert(set.count == 6 and set.instanceCount == 1 and set.visible)
            assert(table.concat(set.tags, ' ') == 'opaque shadow')
            assert(tostring(set) == 'GpuDrawSet(6 vertices, opaque shadow)', tostring(set))

            set.count = 3
            set.visible = false
            assert(set.count == 6 and set.visible, 'not yet committed')
            assert(device.room.changes == 65536 - 2, tostring(device.room.changes))
            device:commit()
            assert(set.count == 3 and not set.visible)
            assert(tostring(set) == 'GpuDrawSet(3 vertices, opaque shadow, hidden)', tostring(set))

            assert(set:remove() and set.added)
            device:commit()
            assert(not set.added and not set:remove())
            set:add()
            set:add()
            device:commit()
            assert(set.added)

            -- A material change writes a buffer; it compiles nothing, and the same program and
            -- state made again is the pipeline the device already has.
            local before = device.pipelines
            color:write(0, buffer.create(16))
            assert(device:pipeline(program, { colors = { 'rgba8Unorm' } }) ~= nil)
            local after = device.pipelines
            assert(after.compiled == before.compiled and after.reused == before.reused + 1)
";

/// A draw set made from Luau is kept on its device until it is removed; its count, instance
/// count, and visibility change in place, checked as a pass checks a draw.
#[cfg(feature = "gpu")]
#[test]
fn a_script_keeps_draw_sets() {
    let Some(runtime) = gpu_runtime("a_script_keeps_draw_sets") else {
        return;
    };
    runtime.exec(DRAW_SETS).unwrap_or_else(|error| panic!("{error}"));

    let prelude = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local program = shader.program({
              attributes = { position = shader.vec2 },
              material = { color = shader.vec4 },
              vertex = function(input) return { position = shader.vec4(input.attributes.position, 0, 1) } end,
              fragment = function(input) return input.material.color end,
            })
            local pipeline = device:pipeline(program, { colors = { 'rgba8Unorm' } })
            local material = pipeline:bind('material', { [0] = device:buffer(16, 'uniform') })
            local vertices = device:buffer(48, 'vertex')
            local set = device:drawSet({ pipeline = pipeline, material = material, vertices = vertices, count = 6 })
        ";
    for (script, expected) in [
        ("device:drawSet(1)", "GpuDevice:drawSet takes a table, not number"),
        (
            "device:drawSet({ pipeline = pipeline, pass = material, count = 1 })",
            "unknown GpuDevice:drawSet option 'pass'",
        ),
        (
            "device:drawSet({ pipeline = pipeline, vertices = vertices, count = 6 })",
            "GpuDevice:drawSet: the draw set is invalid: its program binds a material group, and the draw gives none",
        ),
        (
            "device:drawSet({ pipeline = device:pipeline(shader.compute({ workgroupSize = 1, \
             compute = function() end })), count = 1 })",
            "GpuDevice:drawSet: the pipeline computes: a draw set draws, so it needs a render pipeline",
        ),
        (
            "device:drawSet({ pipeline = pipeline, material = material, vertices = vertices, count = 6, tags = { 1 } })",
            "GpuDevice:drawSet: a tag is a string, not 1",
        ),
        (
            "set.count = 7",
            "GpuDrawSet.count: the draw set is invalid: 7 vertices of 8 bytes run past its 48-byte vertex buffer",
        ),
        (
            "set.instanceCount = 0",
            "GpuDrawSet.instanceCount: the draw set is invalid: at least one instance",
        ),
        ("set.visible = 1", "GpuDrawSet.visible: visible is a boolean, not 1"),
    ] {
        let error = runtime.exec(&format!("{prelude}\n{script}")).unwrap_err().to_string();
        assert!(error.contains(expected), "{script}: {error}");
    }
}

/// A plan described in Luau: opaque draw sets with depth, an overlay depth tested read-only, a
/// fullscreen pass sampling the result into an imported target, and a copy of that into an
/// imported buffer, run again after a set changes, and resized.
#[cfg(feature = "gpu")]
#[test]
fn a_script_plans_a_frame_and_runs_it() {
    let Some(runtime) = gpu_runtime("a_script_plans_a_frame_and_runs_it") else {
        return;
    };
    runtime.exec(FRAME_PLAN).unwrap_or_else(|error| panic!("{error}"));
}

/// What `a_script_plans_a_frame_and_runs_it` runs.
#[cfg(feature = "gpu")]
const FRAME_PLAN: &str = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()

            local lit = shader.program({
              attributes = { position = shader.vec3 },
              pass = { tint = shader.vec4 },
              material = { color = shader.vec4 },
              vertex = function(input) return { position = shader.vec4(input.attributes.position, 1) } end,
              fragment = function(input) return input.material.color * input.pass.tint end,
            })
            local solid = device:pipeline(lit, { colors = { 'rgba8Unorm' }, depth = 'depth32Float' })
            local glaze = device:pipeline(lit, {
              colors = { 'rgba8Unorm' },
              depth = { format = 'depth32Float', write = false },
            })

            local function uniform(values: { number })
              local bytes = buffer.create(16)
              for index, value in values do
                buffer.writef32(bytes, (index - 1) * 4, value)
              end
              local block = device:buffer(16, 'uniform')
              block:write(0, bytes)
              return block
            end

            local function rectangle(pipeline, left, right, depth, color, tag)
              local corners = { left, -1, right, -1, left, 1, right, -1, right, 1, left, 1 }
              local bytes = buffer.create(72)
              for vertex = 0, 5 do
                buffer.writef32(bytes, vertex * 12, corners[vertex * 2 + 1])
                buffer.writef32(bytes, vertex * 12 + 4, corners[vertex * 2 + 2])
                buffer.writef32(bytes, vertex * 12 + 8, depth)
              end
              local vertices = device:buffer(72, 'vertex')
              vertices:write(0, bytes)
              return device:drawSet({
                pipeline = pipeline,
                material = pipeline:bind('material', { [0] = uniform(color) }),
                vertices = vertices,
                count = 6,
                tags = { tag },
              })
            end

            local green = rectangle(solid, -1, 0, 0.25, { 0, 1, 0, 1 }, 'opaque')
            local red = rectangle(solid, -1, 1, 0.5, { 1, 0, 0, 1 }, 'opaque')
            local overlay = rectangle(glaze, -1, 1, 0.4, { 0, 0, 1, 1 }, 'overlay')
            local tint = solid:bind('pass', { [0] = uniform({ 0.6, 0.6, 0.6, 1 }) })

            local swap = shader.program({
              material = { scene = shader.texture2d },
              vertex = function(input)
                local index = input.builtin.vertexIndex
                local corner = shader.vec2(
                  shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))),
                  shader.float(shader.bitAnd(index, shader.uint(2)))
                )
                return { position = shader.vec4(corner * 2 - 1, 0, 1) }
              end,
              fragment = function(input)
                local texel = shader.load(input.material.scene, shader.vec2u(input.builtin.position.xy), 0)
                return texel.zyxw
              end,
            })
            local present = device:pipeline(swap, { colors = { 'rgba8Unorm' } })
            local material = {}
            for _, binding in swap.layout.material.bindings do
              material[binding.binding] = if binding.kind == 'Sampler' then device:sampler() else 'color'
            end

            local plan = device:plan({
              name = 'scene',
              viewport = { width = 16, height = 16 },
              targets = {
                { name = 'color', format = 'rgba8Unorm' },
                { name = 'depth', format = 'depth32Float' },
                { name = 'debug', format = 'rgba8Unorm', scale = 0.5 },
                { name = 'output', format = 'rgba8Unorm', imported = true },
              },
              buffers = { { name = 'pixels', size = 256 * 16, imported = true } },
              passes = {
                {
                  name = 'opaque',
                  kind = 'draw',
                  tags = { 'opaque' },
                  pass = tint,
                  colors = { { target = 'color', clear = { 0, 0, 0, 1 } } },
                  depth = { target = 'depth', clear = 1 },
                },
                { name = 'debug', kind = 'draw', tags = { 'debug' }, colors = { 'debug' } },
                {
                  name = 'overlay',
                  kind = 'draw',
                  tags = { 'overlay' },
                  pass = tint,
                  depthTest = 'depth',
                  colors = { { target = 'color', load = true } },
                },
                {
                  name = 'present',
                  kind = 'fullscreen',
                  pipeline = present,
                  material = material,
                  reads = { 'color' },
                  colors = { 'output' },
                },
                { name = 'readBack', kind = 'copy', from = 'output', to = 'pixels' },
              },
            })
            assert(table.concat(plan.order, ' ') == 'opaque overlay present readBack', table.concat(plan.order, ' '))
            assert(plan.allocations == 2 and plan.viewport.width == 16 and plan.name == 'scene')
            assert(tostring(plan) == 'GpuPlan(scene, 4 of 5 passes, 16×16)', tostring(plan))
            assert(string.find(plan:dump(), 'culled\n    debug', 1, true), plan:dump())
            assert(string.find(plan:dump(), 'opaque\n    from:   exec:', 1, true), plan:dump())

            local output = device:texture({ width = 16, height = 16, format = 'rgba8Unorm', usage = 'target' })
            local pixels = device:buffer(256 * 16, {})
            local function texel(x, y)
              return buffer.readu32(pixels:read(y * 256 + x * 4, 4), 0)
            end
            plan:run({ output = output, pixels = pixels })
            assert(texel(2, 8) == 0xff009900, string.format('%08x', texel(2, 8)))
            assert(texel(14, 8) == 0xff000099, string.format('%08x', texel(14, 8)))

            overlay.visible = false
            plan:run({ output = output, pixels = pixels })
            assert(texel(14, 8) == 0xff990000, string.format('%08x', texel(14, 8)))

            local timings = plan:timings()
            if device.timestamps then
              assert(timings and #timings == 4 and timings[1].pass == 'opaque' and timings[1].ms >= 0)
              assert(timings[4].pass == 'readBack' and timings[4].ms == nil, 'a copy is not timed')
            else
              assert(timings == nil)
            end

            local small = plan:resized(8, 8)
            assert(small.viewport.width == 8 and plan.viewport.width == 16)

            -- A collection of its own: the red rectangle alone, red swapped to blue.
            local scene = device:drawSets()
            scene:add(red)
            device:commit()
            assert(scene:has(red) and not scene:has(green) and scene.count == 1, tostring(scene))
            assert(tostring(scene) == 'GpuDrawSets(1 set)' and device.sets:has(red))
            overlay.visible = true
            plan:run({ output = output, pixels = pixels }, scene)
            assert(texel(2, 8) == 0xff990000, string.format('%08x', texel(2, 8)))

            -- The copy left out, the pixels keep what they held; put back, one texel of them.
            pixels:write(0, buffer.create(256 * 16))
            plan:setEnabled('readBack', false)
            plan:run({ output = output, pixels = pixels })
            assert(texel(2, 8) == 0 and not plan:enabled('readBack'))
            plan:setEnabled({ 'readBack' }, true)
            plan:setRegion('readBack', { from = { column = 14, row = 8 }, size = { width = 1, height = 1 } })
            plan:run({ output = output, pixels = pixels })
            assert(texel(0, 0) == 0xff000099, string.format('%08x', texel(0, 0)))

            -- A run with nothing new to draw makes nothing new.
            local made = device.made.bindGroups
            plan:run({ output = output, pixels = pixels })
            plan:run({ output = output, pixels = pixels })
            assert(device.made.bindGroups == made, `{device.made.bindGroups} bind groups, not {made}`)
";

/// A plan described in Luau over levels, layers, and samples: a mip chain drawn level by level
/// inside one target, a corner of its last level copied into a buffer, and a multisampled target
/// resolved into the host's.
#[cfg(feature = "gpu")]
#[test]
fn a_script_plans_levels_and_samples() {
    let Some(runtime) = gpu_runtime("a_script_plans_levels_and_samples") else {
        return;
    };
    runtime.exec(LEVELS_PLAN).unwrap_or_else(|error| panic!("{error}"));
}

/// What `a_script_plans_levels_and_samples` runs.
#[cfg(feature = "gpu")]
const LEVELS_PLAN: &str = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local function covering(input)
              local index = input.builtin.vertexIndex
              local corner = shader.vec2(
                shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))),
                shader.float(shader.bitAnd(index, shader.uint(2)))
              )
              return { position = shader.vec4(corner * 2 - 1, 0, 1) }
            end
            local flat = shader.program({
              material = { color = shader.vec4 },
              vertex = covering,
              fragment = function(input) return input.material.color end,
            })
            local halve = shader.program({
              material = { above = shader.texture2d },
              vertex = covering,
              fragment = function(input)
                local at = shader.vec2u(input.builtin.position.xy * 2)
                return shader.load(input.material.above, at, 0) * 0.5
              end,
            })
            local fill = device:pipeline(flat, { colors = { 'rgba8Unorm' } })
            local fillSamples = device:pipeline(flat, { colors = { 'rgba8Unorm' }, multisampled = true })
            local down = device:pipeline(halve, { colors = { 'rgba8Unorm' } })
            local function color(values: { number })
              local bytes = buffer.create(16)
              for index, value in values do
                buffer.writef32(bytes, (index - 1) * 4, value)
              end
              local block = device:buffer(16, 'uniform')
              block:write(0, bytes)
              return { [0] = block }
            end
            local above = {}
            for _, binding in halve.layout.material.bindings do
              above[binding.binding] = if binding.kind == 'Sampler' then device:sampler() else 'chain'
            end

            local plan = device:plan({
              name = 'levels',
              viewport = { width = 8, height = 8 },
              targets = {
                { name = 'chain', format = 'rgba8Unorm', width = 16, height = 16, mips = 3 },
                { name = 'samples', format = 'rgba8Unorm', samples = 4 },
                { name = 'screen', format = 'rgba8Unorm', imported = true },
              },
              buffers = { { name = 'pixels', size = 256 * 4, imported = true } },
              passes = {
                { name = 'fill', kind = 'fullscreen', pipeline = fill, material = color({ 1, 0, 0, 1 }), colors = { 'chain' } },
                {
                  name = 'down1',
                  kind = 'fullscreen',
                  pipeline = down,
                  material = above,
                  reads = { { target = 'chain', level = 0 } },
                  colors = { { target = 'chain', level = 1 } },
                },
                {
                  name = 'down2',
                  kind = 'fullscreen',
                  pipeline = down,
                  material = above,
                  reads = { { target = 'chain', level = 1 } },
                  colors = { { target = 'chain', level = 2 } },
                },
                {
                  name = 'corner',
                  kind = 'copy',
                  from = { target = 'chain', level = 2, column = 1, row = 1 },
                  to = { buffer = 'pixels', offset = 256 },
                  size = { width = 2, height = 2 },
                },
                {
                  name = 'smooth',
                  kind = 'fullscreen',
                  pipeline = fillSamples,
                  material = color({ 0, 1, 0, 1 }),
                  colors = { { target = 'samples', store = false, resolve = 'screen' } },
                },
              },
            })
            local dump = plan:dump()
            assert(string.find(dump, 'reads:  chain[level 1]\n    writes: chain[level 2]', 1, true), dump)
            assert(string.find(dump, 'writes: samples -> screen', 1, true), dump)
            assert(string.find(dump, '16x16, 3 levels', 1, true), dump)

            local screen = device:texture({ width = 8, height = 8, format = 'rgba8Unorm', usage = 'target' })
            local pixels = device:buffer(256 * 4, {})
            plan:run({ screen = screen, pixels = pixels })
            local corner = pixels:read(256, 8)
            local red = buffer.readu8(corner, 0)
            assert(red >= 63 and red <= 65, `a quarter of 255, not {red}`)
            assert(buffer.readu32(pixels:read(0, 4), 0) == 0, 'the first row is untouched')
            assert(buffer.readu32(screen:read(), 0) == 0xff00ff00, string.format('%08x', buffer.readu32(screen:read(), 0)))
";

/// A plan keeps a frame in a buffer sized by its viewport, for the script to read from the plan.
#[cfg(feature = "gpu")]
#[test]
fn a_script_reads_what_a_plan_kept() {
    let Some(runtime) = gpu_runtime("a_script_reads_what_a_plan_kept") else {
        return;
    };
    runtime
        .exec(
            r"
            local device = jess.gpu.device()
            local plan = device:plan({
              viewport = { width = 4, height = 2 },
              targets = { { name = 'color', format = 'rgba8Unorm' } },
              buffers = { { name = 'pixels', image = 'rgba8Unorm', kept = true } },
              passes = {
                { name = 'clear', kind = 'draw', colors = { { target = 'color', clear = { 0, 1, 0, 1 } } } },
                { name = 'readBack', kind = 'copy', from = 'color', to = 'pixels' },
              },
            })
            assert(plan:bytes('pixels') == 512, tostring(plan:bytes('pixels')))
            local extent = plan:extent('color')
            assert(extent.width == 4 and extent.height == 2)
            plan:run()
            assert(buffer.readu32(plan:read('pixels'), 256) == 0xff00ff00)
            assert(buffer.len(plan:readLater('pixels'):wait()) == 512)
            assert(plan:resized(8, 8):bytes('pixels') == 256 * 8)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// A plan described in Luau whose compute passes write a transient buffer, which a fullscreen pass
/// reads and a copy keeps.
#[cfg(feature = "gpu")]
#[test]
fn a_script_plans_compute_for_later_passes() {
    let Some(runtime) = gpu_runtime("a_script_plans_compute_for_later_passes") else {
        return;
    };
    runtime
        .exec(
            r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local output = device:texture({ width = 16, height = 16, format = 'rgba8Unorm', usage = 'target' })
            local brighten = shader.compute({
              workgroupSize = 4,
              pass = { colors = shader.storage(shader.vec4, 'readWrite') },
              compute = function(input)
                local index = input.builtin.globalInvocationId.x
                shader.write(input.pass.colors, index, input.pass.colors[index] + 0.2)
              end,
            })
            local paint = shader.program({
              material = { colors = shader.storage(shader.vec4) },
              vertex = function(input)
                local index = input.builtin.vertexIndex
                local corner = shader.vec2(
                  shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))),
                  shader.float(shader.bitAnd(index, shader.uint(2)))
                )
                return { position = shader.vec4(corner * 2 - 1, 0, 1) }
              end,
              fragment = function(input) return input.material.colors[shader.uint(0)] end,
            })
            local computing = device:pipeline(brighten)
            local painting = device:pipeline(paint, { colors = { 'rgba8Unorm' } })
            local storage = brighten.layout.pass.bindings[1].binding
            local read = paint.layout.material.bindings[1].binding
            local computed = device:plan({
              viewport = { width = 16, height = 16 },
              targets = { { name = 'output', format = 'rgba8Unorm', imported = true } },
              buffers = { { name = 'colors', size = 64 }, { name = 'kept', size = 64, imported = true } },
              passes = {
                {
                  name = 'fill',
                  kind = 'compute',
                  pipeline = computing,
                  pass = { [storage] = 'colors' },
                  storage = { { buffer = 'colors', clear = true } },
                },
                {
                  name = 'again',
                  kind = 'compute',
                  pipeline = computing,
                  pass = { [storage] = 'colors' },
                  storage = { { buffer = 'colors', load = true } },
                  workgroups = { 1 },
                },
                {
                  name = 'paint',
                  kind = 'fullscreen',
                  pipeline = painting,
                  material = { [read] = 'colors' },
                  reads = { 'colors' },
                  colors = { 'output' },
                },
                { name = 'keep', kind = 'copy', from = 'colors', to = 'kept' },
              },
            })
            local kept = device:buffer(64, {})
            for _ = 1, 2 do
              computed:run({ output = output, kept = kept })
              local first = buffer.readf32(kept:read(0, 4), 0)
              assert(math.abs(first - 0.4) < 1e-6, first)
              assert(buffer.readu32(output:read(), 0) == 0x66666666)
            end

        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// What `device:plan` and `plan:run` refuse, each naming the mistake: (script, what the error
/// says).
#[cfg(feature = "gpu")]
const PLAN_MISTAKES: [(&str, &str); 24] = [
    (
        "device:plan({ viewport = viewport, targets = { { name = 't', format = 'rgba8Unorm', imported = true, \
         kept = true } } })",
        "GpuDevice:plan: target 1 is imported and kept: the run gives it, or the plan keeps it",
    ),
    (
        "plan:read('output')",
        "GpuPlan:read: output is imported, not kept: only the plan's kept resources are read from it",
    ),
    ("plan:setEnabled('p', 1)", "GpuPlan:setEnabled: enabled is a boolean, not 1"),
    (
        "plan:setEnabled('nowhere', false)",
        "GpuPlan:setEnabled: the plan has no pass called nowhere",
    ),
    (
        "plan:setRegion('p', { from = { column = 1, offset = 4 } })",
        "GpuPlan:setRegion: its from starts at an offset or at a column and row, not both",
    ),
    (
        "plan:setRegion('p', {})",
        "GpuPlan:setRegion: pass p isn't a copy, so it has no region",
    ),
    (
        "device:plan({ viewport = viewport, targets = { { name = 't', format = 'rgba8Unorm', samples = 3 } } })",
        "GpuDevice:plan: target \"t\" needs 1 or 4 samples a pixel, not 3",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'fullscreen', \
         pipeline = filling, colors = { { target = 'output', level = 1 } } } } })",
        "GpuDevice:plan: pass \"p\" uses level 1 of layer 0 of \"output\", which it doesn't have",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'q', kind = 'copy', \
         from = { target = 'output', offset = 4 }, to = 'output' } } })",
        "GpuDevice:plan: pass q's from is a target, which starts at a column and row, not an offset",
    ),
    ("device:plan(1)", "GpuDevice:plan takes a table, not number"),
    (
        "device:plan({ viewport = viewport, pass = {} })",
        "unknown GpuDevice:plan option 'pass'",
    ),
    (
        "device:plan({ viewport = viewport, passes = { { name = 'p', kind = 'mesh' } } })",
        "GpuDevice:plan: pass 1's kind is one of draw, fullscreen, compute, copy, not \"mesh\"",
    ),
    (
        "device:plan({ viewport = viewport, passes = { { name = 'p', kind = 'copy', colors = {} } } })",
        "unknown GpuDevice:plan copy pass option 'colors'",
    ),
    (
        "device:plan({ viewport = viewport, passes = { { name = 'p', kind = 'draw', colors = { 'nowhere' } } } })",
        "GpuDevice:plan: pass p uses nowhere, which the plan has no target or buffer called",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'compute', \
         pipeline = computing, storage = { 'output' } } } })",
        "GpuDevice:plan: pass \"p\" writes render target \"output\" through a storage attachment, which only a \
         buffer allows",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'fullscreen', \
         pipeline = filling, colors = { { target = 'output', clear = { 0, 0, 0, 1 }, load = true } } } } })",
        "GpuDevice:plan: pass p's write clears output and loads it: expected one or the other",
    ),
    (
        "device:plan({ viewport = viewport, buffers = { { name = 'b', size = 16 } }, passes = { { name = 'p', \
         kind = 'compute', pipeline = computing, storage = { { buffer = 'b', clear = 1 } } } } })",
        "a buffer's clear is true, to zero",
    ),
    (
        "device:plan({ viewport = viewport, targets = { { name = 't', format = 'rgba8Unorm', scale = 1, width = 2, \
         height = 2 } } })",
        "GpuDevice:plan: target 1 has a scale and a width and height: expected one or the other",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'compute', \
         pipeline = filling } } })",
        "GpuDevice:plan: pass p's pipeline draws, and the pass computes",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'draw', \
         pass = {}, colors = { 'output' } } } })",
        "GpuDevice:plan: pass p makes its pass group from bindings, and names no pipeline to lay them out for",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'fullscreen', \
         pipeline = tinted, material = { [0] = 'output' }, colors = { 'output' } } } })",
        "GpuDevice:plan: pass p's material group: binding 0 is the uniform block: expected a resource, not a name",
    ),
    (
        "device:plan({ viewport = viewport, targets = targets, passes = { { name = 'p', kind = 'fullscreen', \
         pipeline = filling, colors = { 'output' } }, { name = 'q', kind = 'copy', from = 'output', to = 'output' \
         } } })",
        "GpuDevice:plan: pass \"q\" both reads and writes \"output\"",
    ),
    (
        "plan:run({ output = 1 })",
        "GpuPlan:run: output is a GpuTexture or a GpuBuffer, not number",
    ),
    (
        "plan:run()",
        "GpuPlan:run: pass p is invalid: the run gives nothing for output",
    ),
];

#[cfg(feature = "gpu")]
#[test]
fn plan_errors_name_the_mistake() {
    let Some(runtime) = gpu_runtime("plan_errors_name_the_mistake") else {
        return;
    };
    let prelude = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local function fullscreen(material)
              return shader.program({
                material = material,
                vertex = function(input) return { position = shader.vec4(0, 0, 0, 1) } end,
                fragment = function(input) return shader.vec4(1) end,
              })
            end
            local filling = device:pipeline(fullscreen(nil), { colors = { 'rgba8Unorm' } })
            local tinted = device:pipeline(fullscreen({ tint = shader.vec4 }), { colors = { 'rgba8Unorm' } })
            local computing = device:pipeline(shader.compute({ workgroupSize = 1, compute = function() end }))
            local viewport = { width = 4, height = 4 }
            local targets = { { name = 'output', format = 'rgba8Unorm', imported = true } }
            local plan = device:plan({
              viewport = viewport,
              targets = targets,
              passes = { { name = 'p', kind = 'fullscreen', pipeline = filling, colors = { 'output' } } },
            })
        ";
    for (script, expected) in PLAN_MISTAKES {
        let error = runtime.exec(&format!("{prelude}\n{script}")).unwrap_err().to_string();
        assert!(error.contains(expected), "{script}: {error}");
    }
}

/// What `a_script_shows_a_plan_in_a_window` runs: everything up to showing its plan in a second
/// window, which is refused.
#[cfg(feature = "window")]
const WINDOW: &str = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local window = jess.gpu.window(device, {
              title = 'jess window test',
              width = 64,
              height = 48,
            })
            assert(window.open and window.width > 0 and window.scale > 0, tostring(window))
            assert(string.find(tostring(window), 'GpuWindow(jess window test, ', 1, true) == 1, tostring(window))

            local green = shader.program({
              vertex = function(input)
                local index = input.builtin.vertexIndex
                local corner = shader.vec2(
                  shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2))),
                  shader.float(shader.bitAnd(index, shader.uint(2)))
                )
                return { position = shader.vec4(corner * 2 - 1, 0, 1) }
              end,
              fragment = function() return shader.vec4(0, 1, 0, 1) end,
            })
            local pipeline = device:pipeline(green, { colors = { window.format } })
            local plan = device:plan({
              viewport = { width = window.width, height = window.height },
              targets = { { name = 'screen', format = window.format, imported = true } },
              buffers = { { name = 'pixels', image = window.format, kept = true } },
              passes = {
                { name = 'fill', kind = 'fullscreen', pipeline = pipeline, colors = { 'screen' } },
                { name = 'grab', kind = 'copy', from = 'screen', to = 'pixels' },
              },
            })
            window:show(plan, 'screen')
            for _ = 1, 10 do
              jess.gpu.pump()
            end
            -- The script's plan is the plan the window draws, at the window's size.
            local extent = plan:extent('screen')
            assert(extent.width == window.width and extent.height == window.height)
            local row = plan:bytes('pixels') // window.height
            local middle = row * (window.height // 2) + 4 * (window.width // 2)
            local texel = buffer.readu32(plan:read('pixels'), middle)
            assert(texel == 0xff00ff00, string.format('%08x', texel))
            -- A scene nothing changes draws nothing more; a commit that changes something draws it.
            local frames = window.frames
            assert(frames >= 1, tostring(frames))
            for _ = 1, 5 do
              jess.gpu.pump()
            end
            assert(window.frames == frames, `{window.frames} frames, not {frames}`)
            device:buffer(4, {}):write(0, 'abcd')
            jess.gpu.pump()
            assert(window.frames == frames + 1, `{window.frames} frames, not {frames + 1}`)

            -- What happened is the script's to read, once.
            assert(typeof(window:events()) == 'table' and #window:events() == 0)
            local input = jess.input.runtime({ windows = true })
            assert(typeof(input:tick()) == 'table')
            -- Asked for a size, it says what it got, and the plan goes with it.
            window:resize(96, 72)
            local resized = nil
            for _ = 1, 50 do
              jess.gpu.pump()
              for _, event in window:events() do
                if event.kind == 'resized' then resized = event end
              end
              if resized then break end
            end
            assert(resized and resized.width == window.width and resized.height == window.height)
            extent = plan:extent('screen')
            assert(extent.width == window.width and extent.height == window.height)
            -- Left unread, its news is the host's to announce.
            window:resize(window.width + 16, window.height + 16)
            for _ = 1, 10 do
              jess.gpu.pump()
            end

            local closing = jess.gpu.window(device, { title = 'closing', width = 32, height = 32 })
            closing:close()
            jess.gpu.pump()
            assert(not closing.open and closing.format == nil)

            -- A plan takes the size of the one window that shows it: showing it in another fails.
            jess.gpu.window(device, { title = 'other', width = 32, height = 32 }):show(plan, 'screen')
";

/// A window shows a plan every frame `jess.gpu.pump` draws, here copying what it drew into a
/// buffer to read, and closes. It opens a real window, so it runs only where `JESS_WINDOW_TESTS`
/// is set, on a desktop: winit allows one event loop a process, so this is the suite's only
/// window.
#[cfg(feature = "window")]
#[test]
fn a_script_shows_a_plan_in_a_window() {
    if std::env::var_os("JESS_WINDOW_TESTS").is_none() {
        eprintln!("SKIPPED a_script_shows_a_plan_in_a_window: set JESS_WINDOW_TESTS on a desktop to open one");
        return;
    }
    if crate::gpu::gpu("a_script_shows_a_plan_in_a_window").is_none() {
        return;
    }
    let capabilities = ["gpu.device".to_owned(), "gpu.window".to_owned(), "input.actions".to_owned()];
    let runtime = jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &capabilities).unwrap();
    let refusal = runtime.exec(WINDOW).unwrap_err().to_string();
    assert!(refusal.contains("shown in another window"), "{refusal}");
    jess::window::pump();
    let news = jess::window::news();
    assert!(!news.is_empty(), "the windows had news no one read");
    assert!(jess::window::news().is_empty(), "news is announced once until it is read");
    drop(runtime);
    kept_windows_outlive_their_code(&capabilities);
    a_shown_scene_that_does_not_change_runs_no_luau();
}

/// What `a_shown_scene_that_does_not_change_runs_no_luau` launches: an application that shows a
/// plan in a window and handles input and the window's news, as a viewer does.
#[cfg(feature = "window")]
const STILL: [(&str, &str); 3] = [
    (
        "jess.toml",
        r#"manifest_version = 1

[package]
id = "org.example.still"
version = "1.0.0"
jess_api = "1"
include = ["src"]

[managed.handlers]
module = "src/handlers.jsl"

[targets.still]
kind = "application"
entry = "src/main.jsl"
capabilities = ["gpu.device", "gpu.window"]
"#,
    ),
    (
        "src/handlers.jsl",
        r"--!strict
return {
  on = {
    host = { input = function() end },
    event = { ['jess.window'] = function() end },
  },
}
",
    ),
    (
        "src/main.jsl",
        r"--!strict
local shader = jess.shader
local device = jess.gpu.device()
local window = jess.gpu.window(device, { title = 'still', width = 48, height = 32 })
local format = assert(window.format)
local fill = shader.program {
  vertex = function(input)
    local index = input.builtin.vertexIndex
    local x = shader.float(shader.bitAnd(shader.shiftLeft(index, shader.uint(1)), shader.uint(2)))
    local y = shader.float(shader.bitAnd(index, shader.uint(2)))
    return { position = shader.vec4(shader.vec2(x, y) * 2 - 1, 0, 1) }
  end,
  fragment = function() return shader.vec4(0, 0, 1, 1) end,
}
local plan = device:plan {
  viewport = { width = window.width, height = window.height },
  targets = { { name = 'screen', format = format, imported = true } },
  passes = {
    { name = 'fill', kind = 'fullscreen', pipeline = device:pipeline(fill, { colors = { format } }), colors = { 'screen' } },
  },
}
window:show(plan, 'screen')
",
    ),
];

/// An application showing a scene that doesn't change: frame after frame, its host runs no Luau,
/// and its window presents nothing new. Part of `a_script_shows_a_plan_in_a_window`, on the thread
/// its windows are on.
#[cfg(feature = "window")]
fn a_shown_scene_that_does_not_change_runs_no_luau() {
    use jess::lua::package::{self, Launched, TargetRequest};

    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("still");
    for (path, text) in STILL {
        let path = root.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let request = TargetRequest {
        package: root.display().to_string(),
        target: Some("still".to_owned()),
        grant: Some(vec!["gpu.device".to_owned(), "gpu.window".to_owned()]),
        ..TargetRequest::default()
    };
    let Launched::Application(mut application) =
        package::launch(&jess::lua::Options::default(), &request).unwrap_or_else(|error| panic!("{error}"))
    else {
        panic!("still is an application");
    };
    application.start().unwrap();
    let frame = |application: &mut jess::lua::application::Application| -> usize {
        let report = application
            .step(std::time::Duration::from_millis(16), &mut |line| panic!("{line}"))
            .unwrap();
        let input = report.input.map_or(0, |phase| phase.ran);
        let phases: usize = report.tick.phases.iter().map(|phase| phase.ran).sum();
        input + phases + report.tick.events.delivered
    };
    // Its first frames show the plan, and the window manager may say a few things about it.
    for _ in 0..10 {
        frame(&mut application);
    }
    jess::window::wait_presented(std::time::Duration::from_secs(1));
    let window = jess::window::windows()
        .into_iter()
        .find(|window| window.title() == "still")
        .expect("the application's window is open");
    let presented = window.frames();
    assert!(presented >= 1, "the scene was drawn");
    let entered: usize = (0..30).map(|_| frame(&mut application)).sum();
    jess::window::wait_presented(std::time::Duration::from_secs(1));
    assert_eq!(entered, 0, "no handler ran in 30 frames of a scene that doesn't change");
    assert_eq!(window.frames(), presented, "nothing new was presented");
    drop(window);
    application.stop().unwrap();
}

/// A window kept by a key outlives the code that opened it, and the next code takes it back, on
/// its own device: part of `a_script_shows_a_plan_in_a_window`, on the thread its windows are on.
#[cfg(feature = "window")]
fn kept_windows_outlive_their_code(capabilities: &[String]) {
    let generation = || jess::lua::runtime_with_capabilities(jess::lua::Options::default(), capabilities).unwrap();
    generation()
        .exec(
            "local window = jess.gpu.window(jess.gpu.device(), { key = 'kept', title = 'one', width = 64, height = 48 })
             assert(window.key == 'kept' and window.open)",
        )
        .unwrap_or_else(|error| panic!("{error}"));
    jess::window::pump();
    assert_eq!(jess::window::kept(), ["kept"], "every handle dropped with its runtime");
    let reclaimed: Result<(), l3i::Error> = jess::window::reclaim(|| {
        generation().exec(
            "local window = jess.gpu.window(jess.gpu.device(), { key = 'kept', title = 'two' })
             assert(window.open and window.title == 'two' and window.format ~= nil, tostring(window))",
        )
    });
    reclaimed.unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(jess::window::kept(), ["kept"]);
    assert!(jess::window::reclaim(|| Err::<(), _>("the load failed")).is_err());
    assert_eq!(jess::window::kept(), ["kept"], "a failed load closes nothing");
    jess::window::reclaim(|| Ok::<(), ()>(())).unwrap();
    jess::window::pump();
    assert!(jess::window::kept().is_empty(), "a load that didn't open it closed it");
}

/// A compute program compacts what it keeps through an atomic counter: every invocation that
/// keeps its index takes its own slot from `atomicAdd`.
#[cfg(feature = "gpu")]
#[test]
fn a_script_compacts_through_atomics() {
    let Some(runtime) = gpu_runtime("a_script_compacts_through_atomics") else {
        return;
    };
    runtime
        .exec(
            r"
            local shader = require('@jess/shader')
            local compact = shader.compute({
              workgroupSize = 64,
              pass = {
                counts = shader.storage(shader.uint, 'atomic'),
                kept = shader.storage(shader.uint, 'readWrite'),
              },
              compute = function(input)
                local index = input.builtin.globalInvocationId.x
                shader.atomicAdd(input.pass.counts, 0, shader.uint(1))
                shader.atomicMax(input.pass.counts, 1, index)
                shader.when(shader.equal(index % shader.uint(2), shader.uint(0)), function()
                  local slot = shader.atomicAdd(input.pass.counts, 2, shader.uint(1))
                  shader.write(input.pass.kept, slot, index)
                end)
              end,
            })
            local device = jess.gpu.device()
            local pipeline = device:pipeline(compact)
            local counts = device:buffer(16, 'storage')
            local kept = device:buffer(128 * 4, 'storage')
            local bindings = {}
            for _, binding in compact.layout.pass.bindings do
              bindings[binding.binding] = if binding.name == 'counts' then counts else kept
            end
            device:dispatch({ pipeline = pipeline, pass = pipeline:bind('pass', bindings), workgroups = 4 })
            local total, largest, keptCount = string.unpack('<I4I4I4', buffer.tostring(counts:read(0, 12)))
            assert(total == 256 and largest == 255 and keptCount == 128, `{total} {largest} {keptCount}`)
            local sum = 0
            local bytes = kept:read()
            for slot = 0, 127 do
              local index = buffer.readu32(bytes, slot * 4)
              assert(index % 2 == 0, index)
              sum += index
            end
            assert(sum == 127 * 128, sum)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}

/// What `a_script_culls_on_the_gpu` runs.
#[cfg(feature = "gpu")]
const GPU_CULLING: &str = r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()

            -- Eight instances across the target; the cull keeps the four on the right half.
            local cull = shader.compute({
              workgroupSize = 64,
              pass = {
                everything = shader.storage(shader.vec4),
                kept = shader.storage(shader.vec4, 'readWrite'),
                arguments = shader.storage(shader.uint, 'atomic'),
              },
              compute = function(input)
                local index = input.builtin.globalInvocationId.x
                local pass = input.pass
                shader.when(shader.less(index, shader.arrayLength(pass.everything)), function()
                  local offset = pass.everything[index]
                  shader.when(shader.greater(offset.x, 0), function()
                    local slot = shader.atomicAdd(pass.arguments, 1, shader.uint(1))
                    shader.write(pass.kept, slot, offset)
                  end)
                end)
              end,
            })
            local square = shader.program({
              attributes = { corner = shader.vec2 },
              instance = { offset = shader.vec4 },
              vertex = function(input)
                local at = input.attributes.corner * shader.vec2(0.05, 0.9) + input.instance.offset.xy
                return { position = shader.vec4(at, 0, 1) }
              end,
              fragment = function() return shader.vec4(1, 1, 1, 1) end,
            })
            local culling = device:pipeline(cull)
            local drawing = device:pipeline(square, { colors = { 'rgba8Unorm' } })

            local function filled(size, usage, values, format)
              local bytes = buffer.create(size)
              for index, value in values do
                if format == 'u32' then
                  buffer.writeu32(bytes, (index - 1) * 4, value)
                else
                  buffer.writef32(bytes, (index - 1) * 4, value)
                end
              end
              local made = device:buffer(size, usage)
              made:write(0, bytes)
              return made
            end
            local positions = {}
            for index = 0, 7 do
              for _, value in { -0.875 + index * 0.25, 0, 0, 1 } do
                table.insert(positions, value)
              end
            end
            local everything = filled(128, 'storage', positions)
            local kept = filled(128, { 'storage', 'vertex' }, {})
            local arguments = filled(16, { 'storage', 'indirect' }, {}, 'u32')
            local template = filled(16, {}, { 6, 0, 0, 0 }, 'u32')
            local corners = filled(48, 'vertex', { -1, -1, 1, -1, -1, 1, 1, -1, 1, 1, -1, 1 })

            device:drawSet({
              pipeline = drawing,
              vertices = corners,
              instances = kept,
              indirect = arguments,
              tags = { 'squares' },
            })
            local storage = {}
            for _, binding in cull.layout.pass.bindings do
              storage[binding.binding] = binding.name
            end
            local plan = device:plan({
              viewport = { width = 64, height = 8 },
              targets = { { name = 'screen', format = 'rgba8Unorm', imported = true } },
              buffers = {
                { name = 'template', size = 16, imported = true },
                { name = 'arguments', size = 16, imported = true },
                { name = 'everything', size = 128, imported = true },
                { name = 'kept', size = 128, imported = true },
              },
              passes = {
                { name = 'reset', kind = 'copy', from = 'template', to = 'arguments' },
                {
                  name = 'cull',
                  kind = 'compute',
                  pipeline = culling,
                  pass = storage,
                  reads = { 'everything' },
                  storage = { 'kept', { buffer = 'arguments', load = true } },
                },
                {
                  name = 'draw',
                  kind = 'draw',
                  tags = { 'squares' },
                  reads = { 'kept', 'arguments' },
                  colors = { { target = 'screen', clear = { 0, 0, 0, 1 } } },
                },
              },
            })
            assert(table.concat(plan.order, ' ') == 'reset cull draw', table.concat(plan.order, ' '))

            local screen = device:texture({ width = 64, height = 8, format = 'rgba8Unorm', usage = 'target' })
            local imports = { screen = screen, template = template, arguments = arguments, everything = everything, kept = kept }
            for _ = 1, 2 do
              plan:run(imports)
              local counts = arguments:read()
              assert(buffer.readu32(counts, 0) == 6 and buffer.readu32(counts, 4) == 4, buffer.readu32(counts, 4))
              local pixels = screen:read()
              local function lit(column) return buffer.readu8(pixels, (4 * 64 + column) * 4) == 255 end
              -- Instance 1 at -0.625 and instance 6 at 0.625: culled, and drawn.
              assert(not lit(12), 'a culled instance is not drawn')
              assert(lit(52), 'a kept instance is drawn')
            end
";

/// A compute pass culls instances on the GPU, compacting the ones it keeps through an atomic
/// count in the arguments a draw set draws indirectly from: no count goes back to Luau.
#[cfg(feature = "gpu")]
#[test]
fn a_script_culls_on_the_gpu() {
    let Some(runtime) = gpu_runtime("a_script_culls_on_the_gpu") else {
        return;
    };
    runtime.exec(GPU_CULLING).unwrap_or_else(|error| panic!("{error}"));
}

/// A headless input runtime asked for its windows' input polls them at every tick, which takes
/// nothing before a window has seen any; a live one has its host's already.
#[cfg(feature = "window")]
#[test]
fn a_headless_input_runtime_takes_its_windows_input_when_asked() {
    let capabilities = ["input.actions".to_owned()];
    let runtime = jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &capabilities).unwrap();
    runtime
        .exec(
            r"
            local input = jess.input.runtime({ windows = true })
            assert(#input:tick() == 0)
            assert(#jess.input.runtime({ windows = false }):tick() == 0)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
    let error = runtime.exec("jess.input.runtime({ windows = 1 })").unwrap_err().to_string();
    assert!(
        error.contains("jess.input.runtime: windows is a boolean, not number"),
        "{error}"
    );
    let error = runtime.exec("jess.input.runtime({ window = true })").unwrap_err().to_string();
    assert!(error.contains("'window'"), "{error}");
}
