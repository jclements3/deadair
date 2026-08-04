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
    @location(7) temp_glass: vec4<f32>, // x=temp_f, y=glass, z=ground-noise amp
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) albedo_emissive: vec4<f32>,
    @location(2) temp_glass: vec4<f32>,
    @location(3) world_pos: vec3<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    var out: VsOut;
    let wp = model * vec4<f32>(in.pos, 1.0);
    out.world_pos = wp.xyz;
    out.clip = globals.view_proj * wp;
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

// Static value noise keyed to world position — terrain texture that can
// never crawl, because it never changes.
fn ground_hash(p: vec2<f32>) -> f32 {
    let h = sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453;
    return fract(h);
}

fn value_noise(p: vec2<f32>) -> f32 {
    let c = floor(p);
    let f = smoothstep(vec2<f32>(0.0), vec2<f32>(1.0), fract(p));
    return mix(
        mix(ground_hash(c), ground_hash(c + vec2<f32>(1.0, 0.0)), f.x),
        mix(ground_hash(c + vec2<f32>(0.0, 1.0)), ground_hash(c + vec2<f32>(1.0, 1.0)), f.x),
        f.y,
    );
}

fn ground_noise(p: vec2<f32>) -> f32 {
    // Three octaves, each rotated off-axis so no view direction lines up
    // with a lattice axis (axis-aligned octaves band at grazing angles).
    let r1 = mat2x2<f32>(0.866, -0.5, 0.5, 0.866);
    let r2 = mat2x2<f32>(0.174, -0.985, 0.985, 0.174);
    let a = value_noise(r1 * p * 1.6);
    let b = value_noise(r2 * p * 0.45);
    let c = value_noise(p * 5.0);
    return (a * 0.45 + b * 0.35 + c * 0.2) - 0.5;
}

@fragment
fn fs_main(in: VsOut) -> FsOut {
    let moon_dir = normalize(vec3<f32>(0.3, 0.8, 0.4));
    let ndl = max(dot(normalize(in.normal), moon_dir), 0.0);
    let moonlight = globals.params.x;
    // Moon key + faint sky ambient so shadowed faces aren't void.
    var lit = in.albedo_emissive.rgb * (moonlight * (0.15 + 0.85 * ndl) + 0.02)
        + in.albedo_emissive.a * in.albedo_emissive.rgb;
    var temp = in.temp_glass.x;
    let namp = in.temp_glass.z;
    if (namp > 0.0) {
        let n = ground_noise(in.world_pos.xz);
        temp += n * namp;                 // thermal mottling, degF
        lit *= 1.0 + n * 0.5;             // matching visible/NV texture
    }
    var out: FsOut;
    out.color = vec4<f32>(lit, in.albedo_emissive.a);
    out.temp = vec2<f32>(temp, in.temp_glass.y);
    return out;
}
