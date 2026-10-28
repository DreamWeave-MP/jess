struct PassUniforms {
    f_view: mat4x4<f32>,
    f_projection: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> u_pass: PassUniforms;

struct MaterialUniforms {
    f_normalMatrix: mat3x3<f32>,
    f_lightDirection: vec3<f32>,
    f_ambient: f32,
}

@group(1) @binding(0) var<uniform> u_material: MaterialUniforms;

@group(1) @binding(1) var t_material_diffuse: texture_2d<f32>;
@group(1) @binding(2) var s_material_diffuse: sampler;

struct DrawUniforms {
    f_model: mat4x4<f32>,
}

@group(2) @binding(0) var<uniform> u_draw: DrawUniforms;

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
    let e1 = u_draw.f_model * e0;
    let e2 = u_pass.f_view * e1;
    let e3 = u_pass.f_projection * e2;
    let e4 = u_material.f_normalMatrix * input.a_normal;
    var output: VertexOutput;
    output.position = e3;
    output.v_normal = e4;
    output.v_uv = input.a_uv;
    return output;
}

@fragment
fn fragment_main(input: VertexOutput) -> FragmentOutput {
    let e0 = textureSample(t_material_diffuse, s_material_diffuse, input.v_uv);
    let e1 = normalize(input.v_normal);
    let e2 = dot(e1, u_material.f_lightDirection);
    let e3 = max(e2, 0.0);
    let e4 = e3 + u_material.f_ambient;
    let e5 = e0.xyz;
    let e6 = e5 * e4;
    let e7 = clamp(e6, vec3<f32>(0.0), vec3<f32>(1.0));
    let e8 = e0.w;
    let e9 = vec4<f32>(e7, e8);
    var output: FragmentOutput;
    output.color0 = e9;
    return output;
}
