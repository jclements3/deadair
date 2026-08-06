// IR eyeshine pass — night vision only.
//
// The standout detail of the IR-illuminated NV footage: animal eyes
// retro-reflect the scope's own beam as brilliant white dots, far brighter
// than anything else in frame. Drawn as camera-facing point sprites,
// additively blended into the color target's rgb AND its emissive channel,
// so the bloom pass gives each dot the halo the sensor shows.
//
// Only the color target is written: thermal reads the temperature buffer,
// which this pass leaves untouched (retro-reflection is light, not heat).

struct Globals {
    view_proj: mat4x4<f32>,
    // x = moonlight, y = ambient_f, z = sky_temp_f, w = aspect (w/h)
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct VsIn {
    @builtin(vertex_index) vi: u32,
    // instance: xyz = world position, w = strength 0..1
    @location(0) pos_strength: vec4<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) strength: f32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    // Two triangles, corners at (±1, ±1).
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[in.vi % 6u];
    var clip = globals.view_proj * vec4<f32>(in.pos_strength.xyz, 1.0);
    // The sprite shrinks with range but never below a legible dot — that is
    // how eyeshine behaves through a real tube: a pinprick that stays
    // visible long after the animal's body has dissolved into grain.
    let ndc_r = clamp(0.14 / max(clip.w, 0.001), 0.022, 0.10);
    clip = vec4<f32>(
        clip.x + c.x * ndc_r * clip.w / max(globals.params.w, 0.001),
        clip.y + c.y * ndc_r * clip.w,
        clip.z,
        clip.w,
    );
    var out: VsOut;
    out.clip = clip;
    out.local = c;
    out.strength = in.pos_strength.w;
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
    let s = clamp(in.strength, 0.0, 1.0);
    // Saturated core plus an exponential skirt: the sensor blows out on the
    // return and the surrounding pixels smear.
    let core = smoothstep(0.42, 0.0, d);
    let skirt = exp(-d * d * 4.0) * 0.35;
    let v = s * (core * 2.5 + skirt);
    var out: FsOut;
    out.color = vec4<f32>(vec3<f32>(v), v);
    out.temp = vec2<f32>(0.0, 0.0); // write-masked off
    return out;
}
