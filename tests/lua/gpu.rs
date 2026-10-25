//! `jess.gpu` from Luau: the device a script opens under `gpu.device`, and what it reports.

use super::common;

/// Without `gpu.device`, asking for the device is denied, naming the function and the capability.
#[test]
fn the_device_needs_its_capability() {
    let runtime = common::runtime();
    let error = runtime.exec("jess.gpu.device()").unwrap_err().to_string();
    assert!(error.contains("jess.gpu.device") && error.contains("gpu.device"), "{error}");
}

/// With it, the device is the runtime's, opened once, describing its adapter; a jess built without
/// the `gpu` feature says it has none.
#[test]
fn the_device_describes_its_adapter() {
    let runtime = jess::lua::runtime_with_capabilities(jess::lua::Options::default(), &["gpu.device".to_owned()]).unwrap();
    let opened = runtime.exec(
        r"
        local ok, device = pcall(jess.gpu.device)
        if not ok then
            assert(string.find(device, 'no wgpu adapter', 1, true) or string.find(device, 'no GPU', 1, true), device)
            return
        end
        assert(device.name ~= '' and device.backend ~= '', tostring(device))
        assert(table.find({ 'discrete', 'integrated', 'virtual', 'cpu', 'other' }, device.kind), device.kind)
        assert(device.limits.textureSize >= 2048 and device.limits.workgroups >= 1)
        assert(string.find(tostring(device), 'GpuDevice(', 1, true) == 1)
        assert(require('@jess/gpu') == jess.gpu)
    ",
    );
    opened.unwrap_or_else(|error| panic!("{error}"));
    #[cfg(not(feature = "gpu"))]
    {
        let error = runtime.exec("jess.gpu.device()").unwrap_err().to_string();
        assert!(error.contains("built without the gpu feature"), "{error}");
    }
}
