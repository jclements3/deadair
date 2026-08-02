//! Raymarch renderer with the zoomrender anti-shimmer machinery:
//! stratified supersampling with a per-pixel jitter pattern that is FIXED
//! across frames (no per-frame grain reseeds), gamma-correct tonemapping,
//! distance-scaled march epsilon, and smooth distance fog so far detail
//! fades instead of crawling.

use crate::campath::CamSample;
use crate::scene::{Mat, Scene};
use crate::sdf::{v3, Vec3};
use rayon::prelude::*;

const T_FAR: f32 = 2500.0;
const MAX_STEPS: u32 = 220;
const SHADOW_T_MAX: f32 = 140.0;
const MAX_ACTIVE: usize = 48;

const SUN_DIR: Vec3 = v3(-0.45, 0.62, 0.38); // toward the sun
const SUN_COL: Vec3 = v3(1.0, 0.97, 0.90);

// ---------------------------------------------------------------------------
// small deterministic hashes (from zoomrender)
// ---------------------------------------------------------------------------

fn hash(mut s: u64) -> f64 {
    s ^= s >> 33;
    s = s.wrapping_mul(0xff51afd7ed558ccd);
    s ^= s >> 33;
    s = s.wrapping_mul(0xc4ceb9fe1a85ec53);
    s ^= s >> 33;
    (s as f64) / (u64::MAX as f64)
}

fn hash2(ix: i64, iz: i64) -> f32 {
    hash(((ix as u64) << 32) ^ (iz as u64 & 0xffff_ffff) ^ 0x5bd1_e995) as f32
}

/// Smooth 2D value noise, 2 octaves. Static in time -> shimmer-free.
fn noise2(x: f32, z: f32) -> f32 {
    let mut amp = 0.6;
    let mut f = 1.0;
    let mut sum = 0.0;
    for _ in 0..2 {
        let (xf, zf) = (x * f, z * f);
        let (ix, iz) = (xf.floor(), zf.floor());
        let (fx, fz) = (xf - ix, zf - iz);
        let (sx, sz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
        let (ix, iz) = (ix as i64, iz as i64);
        let a = hash2(ix, iz);
        let b = hash2(ix + 1, iz);
        let c = hash2(ix, iz + 1);
        let d = hash2(ix + 1, iz + 1);
        sum += amp * (a + (b - a) * sx + (c - a) * sz + (a - b - c + d) * sx * sz);
        amp *= 0.5;
        f *= 2.7;
    }
    sum
}

// ---------------------------------------------------------------------------
// sky / ground
// ---------------------------------------------------------------------------

fn sky(rd: Vec3) -> Vec3 {
    let t = rd.y.max(0.0).powf(0.6);
    let horizon = v3(0.85, 0.90, 0.98);
    let zenith = v3(0.25, 0.48, 0.86);
    let base = horizon.lerp(zenith, t);
    let sun = SUN_DIR.normalize();
    let a = rd.dot(sun).max(0.0);
    base + v3(1.0, 0.95, 0.85) * (a.powf(600.0) * 8.0 + a.powf(6.0) * 0.12)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Ground albedo: meadow -> farmland patchwork -> town -> mountain scrub,
/// with a curved road running the length of the world.
fn ground_albedo(p: Vec3) -> Vec3 {
    let meadow = v3(0.20, 0.36, 0.14);
    let wheat = v3(0.52, 0.45, 0.19);
    let town = v3(0.30, 0.34, 0.22);
    let scrub = v3(0.31, 0.29, 0.21);

    // patchwork fields only matter in the farm band
    let patch = smoothstep(0.42, 0.58, noise2(p.x * 0.035, p.z * 0.035));
    let farm = meadow.lerp(wheat, patch);

    let mut c = meadow;
    c = c.lerp(farm, smoothstep(28.0, 45.0, p.x) * (1.0 - smoothstep(88.0, 108.0, p.x)));
    c = c.lerp(town, smoothstep(108.0, 122.0, p.x) * (1.0 - smoothstep(168.0, 190.0, p.x)));
    c = c.lerp(scrub, smoothstep(190.0, 240.0, p.x));

    // gentle large-scale mottling so the plain isn't flat-shaded
    let m = noise2(p.x * 0.008 + 31.0, p.z * 0.008 - 17.0);
    c = c * (0.92 + 0.16 * m);

    // road: from the tee toward the mountains, gently curving
    let road_z = 6.0 + 3.5 * (p.x * 0.025).sin();
    let d = (p.z - road_z).abs();
    let asphalt = v3(0.16, 0.16, 0.17);
    let shoulder = v3(0.38, 0.34, 0.26);
    if p.x > 8.0 && p.x < 205.0 {
        let fade = smoothstep(8.0, 14.0, p.x) * (1.0 - smoothstep(196.0, 205.0, p.x));
        c = c.lerp(shoulder, (1.0 - smoothstep(2.6, 3.4, d)) * fade);
        c = c.lerp(asphalt, (1.0 - smoothstep(2.2, 2.6, d)) * fade);
    }
    c
}

// ---------------------------------------------------------------------------
// marching
// ---------------------------------------------------------------------------

struct ActiveSet {
    idx: [u16; MAX_ACTIVE],
    n: usize,
}

fn active_objects(scene: &Scene, ro: Vec3, rd: Vec3, t_max: f32) -> ActiveSet {
    let inv = v3(
        1.0 / if rd.x.abs() < 1e-9 { 1e-9 } else { rd.x },
        1.0 / if rd.y.abs() < 1e-9 { 1e-9 } else { rd.y },
        1.0 / if rd.z.abs() < 1e-9 { 1e-9 } else { rd.z },
    );
    let mut set = ActiveSet { idx: [0; MAX_ACTIVE], n: 0 };
    for (i, o) in scene.objects.iter().enumerate() {
        if set.n >= MAX_ACTIVE {
            break;
        }
        if o.aabb.expand(0.5).ray_hit(ro, inv, t_max).is_some() {
            set.idx[set.n] = i as u16;
            set.n += 1;
        }
    }
    set
}

/// Distance to the nearest active object; cheap AABB lower bound when far.
fn scene_dist(scene: &Scene, set: &ActiveSet, p: Vec3) -> (f32, usize) {
    let mut best = f32::MAX;
    let mut who = usize::MAX;
    for k in 0..set.n {
        let i = set.idx[k] as usize;
        let o = &scene.objects[i];
        let ad = o.aabb.distance(p);
        if ad >= best {
            continue;
        }
        let d = if ad > 0.8 { ad } else { o.node.dist(p) };
        if d < best {
            best = d;
            who = i;
        }
    }
    (best, who)
}

struct March {
    t: f32,
    obj: usize,
}

fn march(scene: &Scene, set: &ActiveSet, ro: Vec3, rd: Vec3, t_max: f32) -> Option<March> {
    if set.n == 0 {
        return None;
    }
    let mut t = 0.02f32;
    for _ in 0..MAX_STEPS {
        if t >= t_max {
            return None;
        }
        let p = ro + rd * t;
        let (d, who) = scene_dist(scene, set, p);
        let eps = 1e-3 + t * 7e-4;
        if d < eps {
            return Some(March { t, obj: who });
        }
        t += d * 0.9;
    }
    None
}

fn normal_of(scene: &Scene, obj: usize, p: Vec3, t: f32) -> Vec3 {
    let h = (1e-3 + t * 3e-4).min(0.05);
    let n = &scene.objects[obj].node;
    // tetrahedron gradient
    let e = [v3(1.0, -1.0, -1.0), v3(-1.0, -1.0, 1.0), v3(-1.0, 1.0, -1.0), v3(1.0, 1.0, 1.0)];
    let mut g = Vec3::ZERO;
    for k in e {
        g = g + k * n.dist(p + k * h);
    }
    g.normalize()
}

/// Soft shadow toward the sun over the CSG objects (the ground never
/// occludes the sun in this scene).
fn sun_visibility(scene: &Scene, p: Vec3) -> f32 {
    let sun = SUN_DIR.normalize();
    let set = active_objects(scene, p, sun, SHADOW_T_MAX);
    if set.n == 0 {
        return 1.0;
    }
    let mut res: f32 = 1.0;
    let mut t = 0.06f32;
    for _ in 0..64 {
        if t >= SHADOW_T_MAX {
            break;
        }
        let (d, _) = scene_dist(scene, &set, p + sun * t);
        res = res.min(16.0 * d / t);
        if res < 0.02 {
            return 0.0;
        }
        t += d.clamp(0.03, 4.0);
    }
    res.clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// shading
// ---------------------------------------------------------------------------

fn fog(col: Vec3, rd: Vec3, t: f32) -> Vec3 {
    let f = 1.0 - (-t * 0.0010).exp();
    // fog toward the sky color at the horizon in that direction
    let fog_col = sky(v3(rd.x, rd.y.max(0.02) * 0.3, rd.z));
    col.lerp(fog_col, f)
}

fn shade(scene: &Scene, ro: Vec3, rd: Vec3) -> Vec3 {
    // analytic ground plane bounds the march
    let t_ground = if rd.y < -1e-6 { -ro.y / rd.y } else { T_FAR };
    let t_max = t_ground.min(T_FAR);

    let set = active_objects(scene, ro, rd, t_max);
    let hit = march(scene, &set, ro, rd, t_max);

    let sun = SUN_DIR.normalize();
    match hit {
        Some(h) => {
            let p = ro + rd * h.t;
            let o = &scene.objects[h.obj];
            let n = normal_of(scene, h.obj, p, h.t);
            let albedo = o.mat.albedo(p);
            let vis = sun_visibility(scene, p + n * 0.05);
            let mut col = Vec3::ZERO;
            let ndl = n.dot(sun).max(0.0);
            col = col + albedo.mul(SUN_COL * 2.6) * (ndl * vis);
            if o.spec > 0.0 && vis > 0.0 {
                let hv = (sun - rd).normalize();
                let sp = n.dot(hv).max(0.0).powf(120.0);
                col = col + SUN_COL * 2.6 * (sp * o.spec * vis);
            }
            let sky_amb = v3(0.35, 0.45, 0.62) * (0.5 * (1.0 + n.y)) * 0.55;
            let bounce = v3(0.30, 0.28, 0.24) * (0.5 * (1.0 - n.y)) * 0.35;
            col = col + albedo.mul(sky_amb + bounce);
            fog(col, rd, h.t)
        }
        None if t_ground < T_FAR => {
            let p = ro + rd * t_ground;
            let albedo = ground_albedo(p);
            let vis = sun_visibility(scene, p + v3(0.0, 0.05, 0.0));
            let ndl = sun.y.max(0.0);
            let mut col = albedo.mul(SUN_COL * 2.6) * (ndl * vis);
            let sky_amb = v3(0.35, 0.45, 0.62) * 0.55;
            col = col + albedo.mul(sky_amb);
            fog(col, rd, t_ground)
        }
        None => sky(rd),
    }
}

fn tonemap(c: Vec3) -> Vec3 {
    let m = |x: f32| {
        let x = (x * (1.0 + x / 4.0)) / (1.0 + x);
        x.clamp(0.0, 1.0).powf(1.0 / 2.2)
    };
    v3(m(c.x), m(c.y), m(c.z))
}

// ---------------------------------------------------------------------------
// frame
// ---------------------------------------------------------------------------

pub fn render_frame(scene: &Scene, cam: &CamSample, size: usize, ss: usize) -> Vec<u8> {
    let eye = cam.eye;
    let fwd = (cam.look - eye).normalize();
    let right = fwd.cross(v3(0.0, 1.0, 0.0)).normalize();
    let up = right.cross(fwd);
    let fov = cam.fov_deg.to_radians();
    let half_h = (fov / 2.0).tan();
    let half_w = half_h; // square frame

    let w = size;
    let mut rgba = vec![0u8; w * w * 4];
    rgba.par_chunks_mut(w * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w {
            let mut acc = Vec3::ZERO;
            for sy in 0..ss {
                for sx in 0..ss {
                    // NOTE: seed has no frame term — the jitter pattern is
                    // identical every frame (anti-flicker hard gate).
                    let seed = ((y as u64) << 40) | ((x as u64) << 16) | ((sy * ss + sx) as u64);
                    let jx = (sx as f64 + hash(seed)) / ss as f64;
                    let jy = (sy as f64 + hash(seed ^ 0x9e3779b97f4a7c15)) / ss as f64;
                    let u = ((x as f64 + jx) / w as f64) * 2.0 - 1.0;
                    let vv = 1.0 - ((y as f64 + jy) / w as f64) * 2.0;
                    let rd = (fwd + right * (u as f32 * half_w) + up * (vv as f32 * half_h))
                        .normalize();
                    acc = acc + shade(scene, eye, rd);
                }
            }
            let c = tonemap(acc * (1.0 / (ss * ss) as f32));
            let i = x * 4;
            row[i] = (c.x * 255.0 + 0.5) as u8;
            row[i + 1] = (c.y * 255.0 + 0.5) as u8;
            row[i + 2] = (c.z * 255.0 + 0.5) as u8;
            row[i + 3] = 255;
        }
    });
    rgba
}
