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
            "GpuDevice:buffer: a usage is one of vertex, index, uniform, storage, not \"vertexx\"",
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

/// A draw set made from Luau is kept on its device until it is removed; its count, instance
/// count, and visibility change in place, checked as a pass checks a draw.
#[cfg(feature = "gpu")]
#[test]
fn a_script_keeps_draw_sets() {
    let Some(runtime) = gpu_runtime("a_script_keeps_draw_sets") else {
        return;
    };
    runtime
        .exec(
            r"
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
            assert(set.count == 6 and set.instanceCount == 1 and set.visible and set.added)
            assert(table.concat(set.tags, ' ') == 'opaque shadow')
            assert(tostring(set) == 'GpuDrawSet(6 vertices, opaque shadow)', tostring(set))

            set.count = 3
            set.visible = false
            assert(set.count == 3 and not set.visible)
            assert(tostring(set) == 'GpuDrawSet(3 vertices, opaque shadow, hidden)', tostring(set))

            assert(set:remove() and not set.added and not set:remove())
            set:add()
            set:add()
            assert(set.added)

            -- A material change writes a buffer; it compiles nothing, and the same program and
            -- state made again is the pipeline the device already has.
            local before = device.pipelines
            color:write(0, buffer.create(16))
            assert(device:pipeline(program, { colors = { 'rgba8Unorm' } }) ~= nil)
            local after = device.pipelines
            assert(after.compiled == before.compiled and after.reused == before.reused + 1)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));

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

            rectangle(solid, -1, 0, 0.25, { 0, 1, 0, 1 }, 'opaque')
            rectangle(solid, -1, 1, 0.5, { 1, 0, 0, 1 }, 'opaque')
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

            local small = plan:resized(8, 8)
            assert(small.viewport.width == 8 and plan.viewport.width == 16)
";

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
const PLAN_MISTAKES: [(&str, &str); 15] = [
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
    let capabilities = ["gpu.device".to_owned(), "gpu.window".to_owned()];
    jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &capabilities)
        .unwrap()
        .exec(
            r"
            local shader = require('@jess/shader')
            local device = jess.gpu.device()
            local window = jess.gpu.window(device, {
              title = 'jess window test',
              width = 64,
              height = 48,
              resizable = false,
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
            local row = math.ceil(window.width * 4 / 256) * 256
            local plan = device:plan({
              viewport = { width = window.width, height = window.height },
              targets = { { name = 'screen', format = window.format, imported = true } },
              buffers = { { name = 'pixels', size = row * window.height, imported = true } },
              passes = {
                { name = 'fill', kind = 'fullscreen', pipeline = pipeline, colors = { 'screen' } },
                { name = 'grab', kind = 'copy', from = 'screen', to = 'pixels' },
              },
            })
            local pixels = device:buffer(row * window.height, {})
            window:show(plan, 'screen', { pixels = pixels })
            for _ = 1, 10 do
              jess.gpu.pump()
            end
            local middle = row * (window.height // 2) + 4 * (window.width // 2)
            local texel = buffer.readu32(pixels:read(middle, 4), 0)
            assert(texel == 0xff00ff00, string.format('%08x', texel))

            window:close()
            jess.gpu.pump()
            assert(not window.open and window.format == nil)
        ",
        )
        .unwrap_or_else(|error| panic!("{error}"));
}
