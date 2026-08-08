// Geometry pass: instanced primitives -> two targets.
//   target0 (rgba16f): rgb = lit albedo (moonlight lambert) + emissive, a = emissive strength
//   target1 (rg16f):   r = display temperature (degF), g = glass flag
// Every optic post pass reads these; none gets privileged data.

struct Globals {
    view_proj: mat4x4<f32>,
    // x = moonlight 0..1, y = ambient_f, z = sky_temp_f, w = unused
    params: vec4<f32>,
    // xyz = camera world position (analytic sphere ray origin), w = unused
    cam_pos: vec4<f32>,
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
    // Object-space position: coat mottle samples here so the pattern is
    // pinned to the body and rides the gait (world-space noise would make
    // the coat "swim" through a moving animal).
    @location(4) local_pos: vec3<f32>,
    // Analytic sphere: xyz = world centre, w = world radius. Written by
    // vs_sphere; vs_main leaves it zeroed (fs_main never reads it).
    @location(5) sphere: vec4<f32>,
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
    out.local_pos = in.pos;
    out.sphere = vec4<f32>(0.0);   // unused on the mesh path
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

// Shared shading. Split out of fs_main so the analytic sphere path below
// gets IDENTICAL lighting, mottle and thermal handling -- a second copy
// would drift.
fn shade(
    normal: vec3<f32>,
    world_pos: vec3<f32>,
    local_pos: vec3<f32>,
    albedo_emissive: vec4<f32>,
    temp_glass: vec4<f32>,
) -> FsOut {
    let moon_dir = normalize(vec3<f32>(0.3, 0.8, 0.4));
    let ndl = max(dot(normalize(normal), moon_dir), 0.0);
    let moonlight = globals.params.x;
    var lit = albedo_emissive.rgb * (moonlight * (0.15 + 0.85 * ndl) + 0.02)
        + albedo_emissive.a * albedo_emissive.rgb;
    var temp = temp_glass.x;
    let namp = temp_glass.z;
    if (namp > 0.0) {
        let n = ground_noise(world_pos.xz);
        temp += n * namp;
        lit *= 1.0 + n * 0.5;
    }
    let camp = temp_glass.w;
    if (camp > 0.0) {
        let off = albedo_emissive.rgb * 47.0;
        let cn = value_noise(local_pos.xy * vec2<f32>(2.2, 7.0) + off.rg) * 0.5
            + value_noise(local_pos.xy * vec2<f32>(5.0, 16.0) + off.gb) * 0.3
            + value_noise(local_pos.zy * 9.0 + off.rb) * 0.2
            - 0.5;
        temp += cn * camp * 2.0;
        lit *= 1.0 + cn * 0.6;
    }
    var out: FsOut;
    out.color = vec4<f32>(lit, albedo_emissive.a);
    out.temp = vec2<f32>(temp, temp_glass.y);
    return out;
}

// ---------------------------------------------------------------------------
// Analytic sphere
//
// A tessellated sphere is a polyhedron: its silhouette is a polygon and the
// facets show at close range no matter how the normals are smoothed. This
// path keeps the sphere mesh only as PROXY geometry to get fragments, then
// ray-traces the true sphere per pixel. The silhouette is then exactly
// circular at any zoom and the normal is exact rather than interpolated.
//
// The proxy must CONTAIN the sphere or the real silhouette would be clipped
// by the polyhedron, so vs_sphere inflates it (see SPHERE_PROXY_INFLATE).
// Fragments that miss are discarded, and frag_depth is written from the true
// hit so spheres interpenetrate other geometry correctly.
// ---------------------------------------------------------------------------

struct SphereOut {
    @builtin(frag_depth) depth: f32,
    @location(0) color: vec4<f32>,
    @location(1) temp: vec2<f32>,
};

@vertex
fn vs_sphere(in: VsIn) -> VsOut {
    let model = mat4x4<f32>(in.m0, in.m1, in.m2, in.m3);
    var out: VsOut;
    // Inflate the proxy so the polyhedron circumscribes the unit sphere.
    let wp = model * vec4<f32>(in.pos * 1.06, 1.0);
    out.world_pos = wp.xyz;
    out.clip = globals.view_proj * wp;
    out.normal = normalize((model * vec4<f32>(in.normal, 0.0)).xyz);
    out.albedo_emissive = in.albedo_emissive;
    out.temp_glass = in.temp_glass;
    out.local_pos = vec3<f32>(0.0);
    // Centre and radius recovered from the instance transform. Sphere
    // instances are uniformly scaled, so any basis vector gives the radius.
    let centre = (model * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;
    let radius = length((model * vec4<f32>(1.0, 0.0, 0.0, 0.0)).xyz);
    out.sphere = vec4<f32>(centre, radius);
    return out;
}

@fragment
fn fs_sphere(in: VsOut) -> SphereOut {
    // Reconstruct centre/radius per fragment from the same transform the
    // vertex stage used: the model matrix is carried in the interpolants
    // only via world_pos, so instead derive the ray and solve against the
    // sphere implied by this instance's proxy.
    let ro = globals.cam_pos.xyz;
    let rd = normalize(in.world_pos - ro);

    // The proxy surface is at 1.06r from the centre along the vertex
    // direction; centre is therefore reachable from world_pos and normal.
    // normal is the (rotated) unit outward direction of the proxy vertex,
    // so centre = world_pos - normal * (1.06 * r). Radius comes from the
    // instance scale, recovered below from the proxy offset magnitude.
    // Both are passed exactly via a dedicated interpolant instead.
    let centre = in.sphere.xyz;
    let r = in.sphere.w;

    let oc = ro - centre;
    let b = dot(oc, rd);
    let c = dot(oc, oc) - r * r;
    let h = b * b - c;
    if (h < 0.0) {
        discard;
    }
    let sh = sqrt(h);
    var t = -b - sh;
    if (t < 0.0) {
        t = -b + sh;        // camera inside the sphere: take the far root
    }
    if (t < 0.0) {
        discard;            // entirely behind the camera
    }

    let hit = ro + rd * t;
    let n = normalize(hit - centre);

    var out: SphereOut;
    let clip = globals.view_proj * vec4<f32>(hit, 1.0);
    out.depth = clip.z / clip.w;
    let s = shade(n, hit, n * r, in.albedo_emissive, in.temp_glass);
    out.color = s.color;
    out.temp = s.temp;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> FsOut {
    // Mesh path. Interpolated normal, rasterized depth. Shading is shared
    // with the analytic sphere path so the two can never drift apart.
    return shade(in.normal, in.world_pos, in.local_pos, in.albedo_emissive, in.temp_glass);
}
