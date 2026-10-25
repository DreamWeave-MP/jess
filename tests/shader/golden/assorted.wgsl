struct PassUniforms {
    f_viewportSize: vec2<f32>,
    f_time: f32,
}

@group(0) @binding(0) var<uniform> u_pass: PassUniforms;

struct MaterialUniforms {
    f_tint: vec3<f32>,
    f_offset: i32,
}

@group(1) @binding(0) var<uniform> u_material: MaterialUniforms;

struct VertexInput {
    @location(0) a_position: vec4<f32>,
    @location(1) a_index: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) v_index: u32,
    @location(1) @interpolate(flat) v_signed: i32,
}

struct FragmentOutput {
    @location(0) color0: vec4<f32>,
    @location(1) color1: vec4<f32>,
}

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let e0 = u_material.f_offset - i32(-2147483648);
    var output: VertexOutput;
    output.position = input.a_position;
    output.v_index = input.a_index;
    output.v_signed = e0;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let e0 = cross(u_material.f_tint, vec3<f32>(0.0, 0.0, 1.0));
    let e1 = mix(u_material.f_tint, e0, u_pass.f_time);
    let e2 = length(e1);
    let e3 = min(u_pass.f_viewportSize, vec2<f32>(e2));
    let e4 = vec4<f32>(e1, 1.0);
    let e5 = vec4<f32>(e3, 0.5, 0.5);
    let e6 = e5 / e2;
    var output: FragmentOutput;
    output.color0 = e4;
    output.color1 = e6;
    return output;
}
