// Emissive bloom: a cheap separable blur of the geometry pass's emissive
// channel, composited additively by the optic pass in NV and Eye modes.
//
// Reference: streetlights, lit windows, the IR illuminator and the eyeshine
// returns all halo in the footage — a light source through a real objective
// is never a hard-edged shape. Thermal gets none of this: it is a light
// effect, and the thermal core deliberately never touches the color buffer.
//
// Fixed 9-tap Gaussian, run at half resolution, two passes (H then V) —
// small enough to stay honest on llvmpipe in the headless tests.

struct BlurParams {
    // xy = texel step (in target UV) for this direction, z = source scale
    // (emissive gain), w unused
    p: vec4<f32>,
};
@group(0) @binding(0) var<uniform> bp: BlurParams;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var out: VsOut;
    let x = f32(i32(vi & 1u) * 4 - 1);
    let y = f32(i32(vi >> 1u) * 4 - 1);
    out.clip = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5 + 0.5, 0.5 - y * 0.5);
    return out;
}

const W0: f32 = 0.227027;
const W1: f32 = 0.194594;
const W2: f32 = 0.121621;
const W3: f32 = 0.054054;
const W4: f32 = 0.016216;

// Pass 1: extract emissive (rgb premultiplied by the emissive channel) and
// blur horizontally in one go.
@fragment
fn fs_extract_h(in: VsOut) -> @location(0) vec4<f32> {
    let d = bp.p.xy;
    var acc = vec3<f32>(0.0);
    var w = array<f32, 5>(W0, W1, W2, W3, W4);
    for (var i: i32 = -4; i <= 4; i = i + 1) {
        let s = textureSampleLevel(src, samp, in.uv + d * f32(i), 0.0);
        // Emissive-only: geometry lit by the moon must not bloom.
        acc = acc + max(s.rgb * s.a, vec3<f32>(s.a * 0.5)) * w[abs(i)];
    }
    return vec4<f32>(acc * bp.p.z, 1.0);
}

// Pass 2: vertical blur of the half-res emissive image.
@fragment
fn fs_blur_v(in: VsOut) -> @location(0) vec4<f32> {
    let d = bp.p.xy;
    var acc = vec3<f32>(0.0);
    var w = array<f32, 5>(W0, W1, W2, W3, W4);
    for (var i: i32 = -4; i <= 4; i = i + 1) {
        acc = acc + textureSampleLevel(src, samp, in.uv + d * f32(i), 0.0).rgb * w[abs(i)];
    }
    return vec4<f32>(acc, 1.0);
}
