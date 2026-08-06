// Residual-heat decal pass (SDD §2.3, FR-T4).
//
// Ground-projected warm discs — bedding forms, tracks, a laid-down barrel.
// They are TEMPERATURE, not light, so they are additively blended into the
// temperature G-buffer only; the color target is write-masked off and the
// eye/NV pipelines therefore cannot see them at all.

struct Globals {
    view_proj: mat4x4<f32>,
    // x = moonlight 0..1, y = ambient_f, z = sky_temp_f, w = aspect (w/h)
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // instance: xyz = world center, w = radius (m)
    @location(2) center_radius: vec4<f32>,
    // instance: x = delta_f above ambient, yzw unused
    @location(3) params: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) delta_f: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let r = in.center_radius.w;
    // Lift a hair off the ground so the depth test against the ground patch
    // is unambiguous on every rasterizer.
    let world = in.center_radius.xyz + vec3<f32>(in.pos.x * r, 0.02, in.pos.z * r);
    var out: VsOut;
    out.clip = globals.view_proj * vec4<f32>(world, 1.0);
    out.local = in.pos.xz;
    out.delta_f = in.params.x;
    return out;
}

struct FsOut {
    @location(0) color: vec4<f32>,
    @location(1) temp: vec2<f32>,
};

@fragment
fn fs_main(in: VsOut) -> FsOut {
    let d = length(in.local);
    if (d > 1.0) { discard; }
    // Soft-edged disc: conduction spreads heat, so real bedding forms have
    // no hard rim. Squared smoothstep reads closest to the footage.
    let f = smoothstep(1.0, 0.0, d);
    var out: FsOut;
    out.color = vec4<f32>(0.0, 0.0, 0.0, 0.0); // write-masked off
    out.temp = vec2<f32>(in.delta_f * f * f, 0.0);
    return out;
}
