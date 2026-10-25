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
