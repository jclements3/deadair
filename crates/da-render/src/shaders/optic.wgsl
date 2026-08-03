// Optic post pass: fullscreen triangle over the geometry G-buffers.
// mode 0 = naked eye, 1 = NV, 2 = thermal.

struct OpticParams {
    // x = mode, y = frame index, z = grain seed, w = gain (NV) / unused
    a: vec4<f32>,
    // x = agc_lo_f, y = agc_hi_f, z = sky_temp_f, w = scope_mask (0/1)
    b: vec4<f32>,
    // x = eye exposure, y = nv_visibility, z = moonlight, w = aspect (w/h)
    c: vec4<f32>,
};
@group(0) @binding(0) var<uniform> p: OpticParams;
@group(0) @binding(1) var color_tex: texture_2d<f32>;
@group(0) @binding(2) var temp_tex: texture_2d<f32>;
@group(0) @binding(3) var depth_tex: texture_depth_2d;
@group(0) @binding(4) var palette_tex: texture_2d<f32>;
@group(0) @binding(5) var samp: sampler;
// Half-res blurred emissive (see bloom.wgsl). Zeroed in thermal mode.
@group(0) @binding(6) var bloom_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle.
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}

fn hash(x: u32, y: u32, frame: u32, seed: u32) -> f32 {
    var h = x * 0x8DA6B343u + y * 0xD8163841u + frame * 0xCB1AB31Fu + seed;
    h ^= h >> 13u;
    h = h * 0x5BD1E995u;
    h ^= h >> 15u;
    return f32(h & 0x00FFFFFFu) / 16777216.0;
}

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.299, 0.587, 0.114));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(color_tex));
    let px = vec2<u32>(in.uv * dims);
    let color = textureLoad(color_tex, vec2<i32>(px), 0);
    let tg = textureLoad(temp_tex, vec2<i32>(px), 0);
    let depth = textureLoad(depth_tex, vec2<i32>(px), 0);
    let is_sky = depth >= 1.0;
    let mode = i32(p.a.x);
    // Bloom is a light-path effect: NV and eye only, never thermal.
    var bloom = vec3<f32>(0.0);
    if (mode != 2) {
        bloom = textureSampleLevel(bloom_tex, samp, in.uv, 0.0).rgb;
    }

    var rgb: vec3<f32>;
    if (mode == 2) {
        // THERMAL: temperature only. Sky = uniform cold floor. Glass reads
        // its own surface temp (already in the buffer) - opaque by nature.
        var t_f = tg.x;
        if (is_sky) { t_f = p.b.z; }
        let x = clamp((t_f - p.b.x) / max(p.b.y - p.b.x, 0.001), 0.0, 1.0);
        rgb = textureSampleLevel(palette_tex, samp, vec2<f32>(x, 0.5), 0.0).rgb;
    } else if (mode == 1) {
        // NV: amplified luminance, phosphor-gray mono, gain grain, emissive bloom.
        var v = luma(color.rgb);
        v = 1.0 - exp(-v * 12.0 * p.c.y); // amplification tone curve
        if (is_sky) { v = 0.25 * p.c.y * (0.4 + 0.6 * p.c.z); } // sky glow
        v = v + color.a * 0.6;            // emissive sources read hot
        v = v + luma(bloom) * 1.6;        // ...and halo into their surrounds
        let n = hash(px.x, px.y, u32(p.a.y), u32(p.a.z)) - 0.5;
        let amp = p.a.w * (0.02 + 0.10 * (1.0 - clamp(v, 0.0, 1.0)));
        v = clamp(v + n * amp, 0.0, 1.0);
        rgb = vec3<f32>(v * 0.92, v, v * 0.94); // slightly cool mono
    } else {
        // NAKED EYE: dim scene, scotopic desaturation toward blue-gray.
        var c = color.rgb * p.c.x;
        if (is_sky) { c = vec3<f32>(0.02, 0.03, 0.06) * (0.3 + 0.7 * p.c.z); }
        let g = luma(c);
        c = mix(vec3<f32>(g), c, 0.35); // rods dominate: low color at night
        // Glare around light sources: the dark-adapted eye blooms harder
        // than any sensor does.
        rgb = c * vec3<f32>(0.85, 0.9, 1.05) + bloom * 0.9;
    }

    // Circular scope mask (aspect-corrected so it stays round).
    if (p.b.w > 0.5) {
        let d = length((in.uv - vec2<f32>(0.5, 0.5)) * vec2<f32>(p.c.w, 1.0));
        let m = 1.0 - smoothstep(0.46, 0.5, d);
        rgb = rgb * m;
    }
    return vec4<f32>(rgb, 1.0);
}
