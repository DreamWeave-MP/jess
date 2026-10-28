struct MaterialUniforms {
    f_color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> u_material: MaterialUniforms;

struct VertexInput {
    @location(0) a_position: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
}

struct FragmentOutput {
    @location(0) color0: vec4<f32>,
}

@vertex
fn vertex_main(input: VertexInput) -> VertexOutput {
    let e0 = vec4<f32>(input.a_position, 0.0, 1.0);
    var output: VertexOutput;
    output.position = e0;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let e0 = u_material.f_color * vec4<f32>(0.75, 0.75, 0.75, 0.75);
    var output: FragmentOutput;
    output.color0 = e0;
    return output;
}
