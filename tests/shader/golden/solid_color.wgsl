struct PassUniforms {
    f_view: mat4x4<f32>,
    f_projection: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> u_pass: PassUniforms;

struct MaterialUniforms {
    f_color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> u_material: MaterialUniforms;

struct DrawUniforms {
    f_model: mat4x4<f32>,
}

@group(2) @binding(0) var<uniform> u_draw: DrawUniforms;

struct VertexInput {
    @location(0) a_position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

struct FragmentOutput {
    @location(0) color0: vec4<f32>,
}

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let e0 = vec4<f32>(input.a_position, 1.0);
    let e1 = u_draw.f_model * e0;
    let e2 = u_pass.f_view * e1;
    let e3 = u_pass.f_projection * e2;
    var output: VertexOutput;
    output.position = e3;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    var output: FragmentOutput;
    output.color0 = u_material.f_color;
    return output;
}
