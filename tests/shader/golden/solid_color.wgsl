struct Semantics {
    model: mat4x4<f32>,
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> semantics: Semantics;

struct Parameters {
    p_color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> parameters: Parameters;

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
    let e1 = semantics.model * e0;
    let e2 = semantics.view * e1;
    let e3 = semantics.projection * e2;
    var output: VertexOutput;
    output.position = e3;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    var output: FragmentOutput;
    output.color0 = parameters.p_color;
    return output;
}
