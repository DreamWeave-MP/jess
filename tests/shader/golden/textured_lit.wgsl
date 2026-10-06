struct Semantics {
    model: mat4x4<f32>,
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> semantics: Semantics;

struct Parameters {
    p_normalMatrix: mat3x3<f32>,
    p_lightDirection: vec3<f32>,
    p_ambient: f32,
}

@group(1) @binding(0) var<uniform> parameters: Parameters;
@group(1) @binding(1) var t_diffuse: texture_2d<f32>;
@group(1) @binding(2) var s_diffuse: sampler;

struct VertexInput {
    @location(0) a_position: vec3<f32>,
    @location(1) a_normal: vec3<f32>,
    @location(2) a_uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) v_normal: vec3<f32>,
    @location(1) v_uv: vec2<f32>,
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
    let e4 = parameters.p_normalMatrix * input.a_normal;
    var output: VertexOutput;
    output.position = e3;
    output.v_normal = e4;
    output.v_uv = input.a_uv;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let e0 = textureSample(t_diffuse, s_diffuse, input.v_uv);
    let e1 = normalize(input.v_normal);
    let e2 = dot(e1, parameters.p_lightDirection);
    let e3 = max(e2, 0.0);
    let e4 = e3 + parameters.p_ambient;
    let e5 = e0.xyz;
    let e6 = e5 * e4;
    let e7 = clamp(e6, vec3<f32>(0.0), vec3<f32>(1.0));
    let e8 = e0.w;
    let e9 = vec4<f32>(e7, e8);
    var output: FragmentOutput;
    output.color0 = e9;
    return output;
}
