// Geometry pass: instanced primitives -> two targets.
//   target0 (rgba16f): rgb = lit albedo (moonlight lambert) + emissive, a = emissive strength
//   target1 (rg16f):   r = display temperature (degF), g = glass flag
// Every optic post pass reads these; none gets privileged data.

struct Globals {
    view_proj: mat4x4<f32>,
    // x = moonlight 0..1, y = ambient_f, z = sky_temp_f, w = unused
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // instance
    @location(2) m0: vec4<f32>,
    @location(3) m1: vec4<f32>,
    @location(4) m2: vec4<f32>,
    @location(5) m3: vec4<f32>,
    @location(6) albedo_emissive: vec4<f32>,
    @location(7) temp_glass: vec4<f32>, // x=temp_f, y=glass, zw unused
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) albedo_emissive: vec4<f32>,
    @location(2) temp_glass: vec4<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    var out: VsOut;
    out.clip = globals.view_proj * model * vec4<f32>(in.pos, 1.0);
    // Normal via model rotation (uniform-ish scales; fine for primitives).
    out.normal = normalize((model * vec4<f32>(in.normal, 0.0)).xyz);
    out.albedo_emissive = in.albedo_emissive;
    out.temp_glass = in.temp_glass;
    return out;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @location(1) temp: vec2<f32>,
};

@fragment
fn fs_main(in: VsOut) -> FsOut {
    let moon_dir = normalize(vec3<f32>(0.3, 0.8, 0.4));
    let ndl = max(dot(normalize(in.normal), moon_dir), 0.0);
    let moonlight = globals.params.x;
    // Moon key + faint sky ambient so shadowed faces aren't void.
    let lit = in.albedo_emissive.rgb * (moonlight * (0.15 + 0.85 * ndl) + 0.02)
        + in.albedo_emissive.a * in.albedo_emissive.rgb;
    var out: FsOut;
    out.color = vec4<f32>(lit, in.albedo_emissive.a);
    out.temp = vec2<f32>(in.temp_glass.x, in.temp_glass.y);
    return out;
}
