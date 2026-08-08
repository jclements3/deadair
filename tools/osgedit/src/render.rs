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

pub const SUN_DIR: Vec3 = v3(-0.45, 0.62, 0.38); // toward the sun
const SUN_COL: Vec3 = v3(1.0, 0.97, 0.90);

// ---------------------------------------------------------------------------
// small deterministic hashes (from zoomrender)
// ---------------------------------------------------------------------------

pub fn hash(mut s: u64) -> f64 {
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

/// C1: bisection root refinement of sphere-trace hits. Always on.
///
/// The plain marcher accepts a hit as soon as the distance drops below the
/// acceptance band `eps` (1e-3 + 7e-4*t), leaving a hit error on the order
/// of eps. With refinement, as soon as the march enters twice that band we
/// try to convert the near-hit into a true sign-change bracket on the
/// accepted object's SDF and bisect it:
///
/// 1. Secant-guided oversteps (slope from the marcher's last two samples,
///    x2 overstep, capped at 4*eps) until the SDF goes negative — a
///    transversal hit crosses in one probe, two at most.
/// 2. Bisect the bracket `REFINE_ITERS` = 5 times (width / 32), tracking the
///    end-point SDF values.
/// 3. One false-position interpolation inside the final bracket — no extra
///    SDF eval; the SDF is locally linear over the sub-mm bracket, so the
///    residual hit error lands at ~1e-5 m or below (vs ~eps ~ 1e-2 at
///    100 m before).
///
/// Attempting at 2*eps lets a successful refinement skip the marcher's final
/// creep steps toward the band, which pays for most of the bisection evals.
/// Grazing rays that never cross within the probe budget fall back to the
/// unrefined path: keep marching and accept at d < eps exactly as before.
/// Refinement evaluates only the accepted object's node: the bracket is
/// within ~eps of that surface, and it is the surface the hit is shaded
/// with.
const REFINE_ITERS: u32 = 5;
const REFINE_PROBES: u32 = 2;

#[allow(clippy::too_many_arguments)]
fn refine_hit(
    scene: &Scene,
    ro: Vec3,
    rd: Vec3,
    t_prev: f32,
    d_prev: f32,
    t: f32,
    d: f32,
    eps: f32,
    who: usize,
) -> Option<March> {
    let node = &scene.objects[who].node;
    let (mut t_lo, mut d_lo, mut t_hi, mut d_hi);
    if d <= 0.0 {
        // the marcher overshot inside: [t_prev, t] is already a bracket
        // (scene_dist(t_prev) >= eps_prev > 0 implies node dist > 0 there)
        t_lo = t_prev;
        d_lo = d_prev;
        t_hi = t;
        d_hi = d;
    } else {
        // probe forward past the surface, secant-guided
        t_lo = t;
        d_lo = d;
        let mut slope = (d_prev - d) / (t - t_prev).max(1e-9);
        let mut crossed = false;
        t_hi = t;
        d_hi = d;
        for _ in 0..REFINE_PROBES {
            if slope <= 1e-6 {
                break; // receding or flat: grazing, give up
            }
            let dt = (2.0 * d_lo / slope).clamp(1e-6, 4.0 * eps);
            let tn = t_lo + dt;
            let dn = node.dist(ro + rd * tn);
            if dn <= 0.0 {
                t_hi = tn;
                d_hi = dn;
                crossed = true;
                break;
            }
            slope = (d_lo - dn) / dt;
            t_lo = tn;
            d_lo = dn;
        }
        if !crossed {
            return None; // no sign change found: let the march continue
        }
    }

    for _ in 0..REFINE_ITERS {
        let tm = 0.5 * (t_lo + t_hi);
        let dm = node.dist(ro + rd * tm);
        if dm > 0.0 {
            t_lo = tm;
            d_lo = dm;
        } else {
            t_hi = tm;
            d_hi = dm;
        }
    }
    // false position inside the final bracket: free (no eval) and the SDF is
    // locally linear at this scale, so this lands within ~1e-5 of the root
    let w = d_lo - d_hi;
    let tr = if w > 1e-12 { t_lo + (t_hi - t_lo) * (d_lo / w) } else { 0.5 * (t_lo + t_hi) };
    Some(March { t: tr, obj: who })
}

fn march(scene: &Scene, set: &ActiveSet, ro: Vec3, rd: Vec3, t_max: f32) -> Option<March> {
    if set.n == 0 {
        return None;
    }
    let mut t = 0.02f32;
    let mut t_prev = t;
    let mut d_prev = f32::MAX;
    for _ in 0..MAX_STEPS {
        if t >= t_max {
            return None;
        }
        let p = ro + rd * t;
        let (d, who) = scene_dist(scene, set, p);
        let eps = 1e-3 + t * 7e-4;
        if d < eps * 2.0 {
            if let Some(m) = refine_hit(scene, ro, rd, t_prev, d_prev, t, d, eps, who) {
                return Some(m);
            }
            if d < eps {
                // grazing ray that never crossed: the pre-refinement accept
                return Some(March { t, obj: who });
            }
        }
        t_prev = t;
        d_prev = d;
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

/// Pass 1. Returns the solid colour **and** the ray parameter it stopped at:
/// the surface hit, the ground plane, or `f32::INFINITY` on a sky miss. The
/// depth is what pass 2 clips its density marches to (see `volumetric.rs`);
/// the colour path below is untouched by its addition, so scenes without
/// volumetrics render exactly the pixels they always did.
fn shade(scene: &Scene, ro: Vec3, rd: Vec3) -> (Vec3, f32) {
    // analytic ground plane bounds the march
    let t_ground = if rd.y < -1e-6 { -ro.y / rd.y } else { T_FAR };
    let t_max = t_ground.min(T_FAR);

    let set = active_objects(scene, ro, rd, t_max);
    let hit = march(scene, &set, ro, rd, t_max);

    let sun = SUN_DIR.normalize();
    let depth = match &hit {
        Some(h) => h.t,
        None if t_ground < T_FAR => t_ground,
        None => f32::INFINITY,
    };
    let col = match hit {
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
    };
    (col, depth)
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
                    let (col, t_hit) = shade(scene, eye, rd);
                    // Pass 2: composite the density fields over the solid
                    // colour, clipped to the depth pass 1 just measured. No
                    // volumetrics -> the pass-1 colour goes through untouched.
                    acc = acc
                        + if scene.volumetrics.is_empty() {
                            col
                        } else {
                            crate::volumetric::composite(
                                &scene.volumetrics,
                                eye,
                                rd,
                                t_hit,
                                col,
                                seed,
                            )
                        };
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

// ---------------------------------------------------------------------------
// tests: C1 root refinement, C3 temporal stability
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Mat, Object, Scene};
    use crate::sdf::{capsule, cylinder, smooth_union, sphere, translate, union, Node};

    fn one(node: Node) -> Scene {
        let aabb = node.aabb();
        Scene {
            objects: vec![Object {
                name: "test",
                node,
                mat: Mat::Flat(v3(0.62, 0.60, 0.56)),
                spec: 0.1,
                aabb,
            }],
            volumetrics: Vec::new(),
        }
    }

    /// C1: a refined primary hit lands within ~1e-4 of the analytic surface.
    /// The unrefined marcher only guarantees |d| < eps = 1e-3 + 7e-4*t
    /// (~7.3e-3 at t = 9); bisection tightens that by ~two orders.
    #[test]
    fn c1_refined_hit_error() {
        let sc = one(translate([0.0, 5.0, 0.0], sphere(1.0)));
        let ro = v3(0.0, 5.0, -10.0);
        let rd = v3(0.0, 0.0, 1.0);
        let set = active_objects(&sc, ro, rd, 100.0);
        let m = march(&sc, &set, ro, rd, 100.0).expect("ray through sphere center must hit");
        // exact first intersection is t = 9.0
        assert!(
            (m.t - 9.0).abs() < 1e-4,
            "refined t = {} (error {:.2e}, want < 1e-4)",
            m.t,
            (m.t - 9.0).abs()
        );
        let d = sc.objects[0].node.dist(ro + rd * m.t).abs();
        assert!(d < 1e-4, "|sdf| at refined hit = {d:.2e}, want < 1e-4");
    }

    /// C1: refinement is on for oblique hits too, not just axis-aligned rays.
    #[test]
    fn c1_refined_hit_error_oblique() {
        let sc = one(translate([0.0, 5.0, 0.0], sphere(1.0)));
        let ro = v3(7.0, 11.0, -6.0);
        let rd = (v3(0.0, 5.0, 0.0) - ro).normalize();
        let set = active_objects(&sc, ro, rd, 100.0);
        let m = march(&sc, &set, ro, rd, 100.0).expect("oblique ray at center must hit");
        let d = sc.objects[0].node.dist(ro + rd * m.t).abs();
        assert!(d < 1e-4, "|sdf| at refined oblique hit = {d:.2e}, want < 1e-4");
    }

    /// C1: a grazing ray (chord depth ~5e-4, shallower than the acceptance
    /// band) must still report a hit no worse than the unrefined marcher:
    /// within the eps band of the surface.
    #[test]
    fn c1_grazing_ray_keeps_hit() {
        let sc = one(translate([0.0, 5.0, 0.0], sphere(1.0)));
        let ro = v3(0.0, 5.9995, -10.0);
        let rd = v3(0.0, 0.0, 1.0);
        let set = active_objects(&sc, ro, rd, 100.0);
        let m = march(&sc, &set, ro, rd, 100.0).expect("grazing ray must still hit");
        let eps = 1e-3 + m.t * 7e-4;
        let d = sc.objects[0].node.dist(ro + rd * m.t).abs();
        assert!(d < eps, "grazing hit |sdf| = {d:.2e}, want < eps = {eps:.2e}");
    }

    /// C1: rays that miss keep missing.
    #[test]
    fn c1_miss_stays_miss() {
        let sc = one(translate([0.0, 5.0, 0.0], sphere(1.0)));
        let ro = v3(0.0, 6.2, -10.0);
        let rd = v3(0.0, 0.0, 1.0);
        let set = active_objects(&sc, ro, rd, 100.0);
        assert!(march(&sc, &set, ro, rd, 100.0).is_none());
    }

    /// C1: refinement is deterministic — same ray, same hit, bit for bit.
    #[test]
    fn c1_deterministic() {
        let sc = one(translate([0.0, 5.0, 0.0], sphere(1.0)));
        let ro = v3(3.0, 7.0, -9.0);
        let rd = (v3(0.0, 5.0, 0.0) - ro).normalize();
        let set = active_objects(&sc, ro, rd, 100.0);
        let a = march(&sc, &set, ro, rd, 100.0).unwrap();
        let b = march(&sc, &set, ro, rd, 100.0).unwrap();
        assert_eq!(a.t.to_bits(), b.t.to_bits());
        assert_eq!(a.obj, b.obj);
    }

    // -----------------------------------------------------------------
    // pass 2 (volumetrics) at the whole-frame level
    // -----------------------------------------------------------------

    use crate::volumetric::{BoundsSpec, Params, VolKind, Volumetric, VolumetricSpec};

    /// A cloud of radius `r` centred at `c`, exported bounds and all.
    fn cloud_at(c: [f32; 3], r: f32, seed: u64, phase: f32) -> Volumetric {
        let (a, b) = (1.45 * r, 0.85 * r);
        Volumetric::load(&VolumetricSpec {
            kind: VolKind::Cloud,
            bounds: BoundsSpec {
                min: [c[0] - a, c[1] - b, c[2] - a],
                max: [c[0] + a, c[1] + b, c[2] + a],
            },
            params: Params { r, seed, phase, ..Default::default() },
            xform: [
                1.0, 0.0, 0.0, c[0], //
                0.0, 1.0, 0.0, c[1], //
                0.0, 0.0, 1.0, c[2], //
                0.0, 0.0, 0.0, 1.0,
            ],
        })
        .unwrap()
    }

    /// A 6x6x6 block sitting on the ground at the origin, plus whatever
    /// fields are handed in.
    fn one_solid(name: &'static str, node: Node, vols: Vec<Volumetric>) -> Scene {
        let aabb = node.aabb();
        Scene {
            objects: vec![Object {
                name,
                node,
                mat: Mat::Flat(v3(0.55, 0.52, 0.48)),
                spec: 0.0,
                aabb,
            }],
            volumetrics: vols,
        }
    }

    /// V1-V4: a small post at the origin, so a cloud behind it is almost
    /// entirely unoccluded and the image metrics below have real signal.
    fn vol_scene(vols: Vec<Volumetric>) -> Scene {
        one_solid("post", translate([0.0, 1.5, 0.0], crate::sdf::cuboid([1.0, 1.5, 1.0], 0.0)), vols)
    }

    fn vol_cam() -> CamSample {
        CamSample { eye: v3(0.0, 4.0, -20.0), look: v3(0.0, 6.0, 0.0), fov_deg: 60.0 }
    }

    /// The field used by V1-V4: high, wide, and well clear of the post.
    fn sky_cloud(seed: u64, phase: f32) -> Volumetric {
        cloud_at([0.0, 9.0, 4.0], 6.0, seed, phase)
    }

    fn mean_abs_diff(a: &[u8], b: &[u8]) -> f64 {
        a.iter().zip(b).map(|(x, y)| (*x as f64 - *y as f64).abs()).sum::<f64>()
            / a.len() as f64
    }

    /// Pixels whose worst RGB channel moved by more than `thresh` levels.
    /// A mean over the whole frame washes out a field that covers a quarter
    /// of it at modest contrast (a white cloud on a pale sky); a pixel count
    /// says what actually changed.
    fn changed_pixels(a: &[u8], b: &[u8], thresh: u8) -> usize {
        a.chunks(4)
            .zip(b.chunks(4))
            .filter(|(p, q)| {
                (0..3).any(|k| (p[k] as i32 - q[k] as i32).unsigned_abs() > thresh as u32)
            })
            .count()
    }

    /// V1: the same 64x64 crop rendered twice is byte-identical.
    #[test]
    fn v1_volumetric_render_is_byte_identical_on_repeat() {
        let sc = vol_scene(vec![sky_cloud(3, 0.0)]);
        let cam = vol_cam();
        let a = render_frame(&sc, &cam, 64, 2);
        let b = render_frame(&sc, &cam, 64, 2);
        assert_eq!(a, b, "two renders of the same scene differ");
        // and it actually drew something other than the bare solid pass
        let bare = render_frame(&vol_scene(Vec::new()), &cam, 64, 2);
        let n = changed_pixels(&a, &bare, 4);
        assert!(n > 250, "the cloud contributed almost nothing ({n} px of 4096 changed)");
    }

    /// V2: seed A and seed B are different fields.
    #[test]
    fn v2_seed_changes_the_image() {
        let cam = vol_cam();
        let a = render_frame(&vol_scene(vec![sky_cloud(1, 0.0)]), &cam, 64, 2);
        let b = render_frame(&vol_scene(vec![sky_cloud(2, 0.0)]), &cam, 64, 2);
        let n = changed_pixels(&a, &b, 4);
        assert!(n > 200, "seeds 1 and 2 rendered near-identical images ({n} px differ)");
    }

    /// V3: a phase sweep moves the image smoothly. A small phase step must
    /// change far less than a large one -- popping (a per-frame re-roll)
    /// would make the two comparable.
    #[test]
    fn v3_phase_sweep_is_smooth() {
        let cam = vol_cam();
        let at = |ph: f32| render_frame(&vol_scene(vec![sky_cloud(5, ph)]), &cam, 64, 2);
        let base = at(0.0);
        let small = mean_abs_diff(&base, &at(0.05));
        let big = mean_abs_diff(&base, &at(4.0));
        assert!(small > 0.0, "phase did nothing at all");
        assert!(
            small * 4.0 < big,
            "phase 0.05 moved {small:.3} but phase 4.0 only {big:.3} -- popping, not sweeping"
        );
    }

    /// V4: pass 1 is untouched. A volumetric no ray reaches must leave the
    /// frame byte-identical to the no-volumetrics render -- the depth
    /// writeout and the pass-2 hook change nothing on their own.
    #[test]
    fn v4_an_unreached_volumetric_leaves_pass_1_byte_identical() {
        let cam = vol_cam();
        let bare = render_frame(&vol_scene(Vec::new()), &cam, 64, 2);
        // 400 m behind the camera: outside every ray's slab test
        let far =
            render_frame(&vol_scene(vec![cloud_at([0.0, 7.0, -400.0], 3.0, 3, 0.0)]), &cam, 64, 2);
        assert_eq!(bare, far, "an off-screen volumetric perturbed pass 1");
    }

    /// V5: occlusion at the frame level. A wall stands 20 m out, a cloud 26 m
    /// behind it and offset to one side. Pixels the wall covers must be
    /// bit-identical with and without the cloud; the rest of the frame must
    /// visibly change.
    #[test]
    fn v5_a_nearer_solid_hides_the_volume_behind_it() {
        let wall = |vols| {
            one_solid(
                "wall",
                translate([0.0, 5.0, 0.0], crate::sdf::cuboid([6.0, 5.0, 0.4], 0.0)),
                vols,
            )
        };
        let cam = CamSample { eye: v3(0.0, 5.0, -20.0), look: v3(0.0, 5.0, 0.0), fov_deg: 60.0 };
        // offset to one side and above: part of it clears the wall, the rest
        // is squarely behind it
        let with = render_frame(&wall(vec![cloud_at([14.0, 12.0, 26.0], 8.0, 4, 0.0)]), &cam, 64, 2);
        let bare = render_frame(&wall(Vec::new()), &cam, 64, 2);
        // the wall is centred and spans ~ +/-18 px across, +/-15 px down
        for y in 26..39 {
            for x in 26..39 {
                let i = (y * 64 + x) * 4;
                assert_eq!(
                    &with[i..i + 3],
                    &bare[i..i + 3],
                    "pixel ({x},{y}) on the wall changed: the cloud behind it leaked through"
                );
            }
        }
        // and the cloud is genuinely in frame, just hidden where the wall is
        // measured: 68 px of 4096 clear the wall; the floor is set at 40 so a
        // silently-empty field still fails
        let n = changed_pixels(&with, &bare, 4);
        assert!(n > 40, "the cloud is not in frame at all ({n} px changed)");
    }

    /// A water-tower-like prop: smooth tank + dome on four thin legs. The
    /// legs go subpixel at range — the classic shimmer stressor.
    fn tower() -> Scene {
        let tank = smooth_union(
            0.3,
            vec![
                translate([0.0, 6.5, 0.0], cylinder(2.0, 3.0)),
                translate([0.0, 9.5, 0.0], sphere(2.0)),
            ],
        );
        let mut legs = Vec::new();
        for (x, z) in [(1.4f32, 1.4f32), (-1.4, 1.4), (1.4, -1.4), (-1.4, -1.4)] {
            legs.push(capsule([x, 0.0, z], [x * 0.8, 6.6, z * 0.8], 0.10));
        }
        one(union(vec![tank, union(legs)]))
    }

    /// C3: the anti-shimmer claim as a CI number. Render the tower from a
    /// slowly orbiting camera (4 degrees over 40 frames, i.e. well under a
    /// pixel of image motion per frame) at 10 / 50 / 200 m, and require the
    /// mean per-pixel temporal luminance variance to stay under threshold.
    /// Real scene motion this slow contributes only smooth, tiny variance;
    /// shimmer (marching/jitter instability) shows up as high-frequency
    /// sparkle and blows the number up.
    ///
    /// Budget: 3 distances x 40 frames at 96x96, 2x2 samples/pixel.
    /// Measured runtime: ~0.3 s release, ~1.6 s debug on this workstation.
    #[test]
    fn c3_temporal_stability() {
        const RES: usize = 96;
        const SS: usize = 2;
        const FRAMES: usize = 40;
        // Measured mean variances (luminance in [0,1], variance in lum^2)
        // on the C1 renderer: 10 m = 2.42e-4, 50 m = 8.80e-5, 200 m =
        // 1.31e-4. Threshold 5e-4 gives ~2x headroom over the worst case;
        // a shimmering renderer is orders of magnitude above this.
        const THRESH: f64 = 5e-4;

        let sc = tower();
        let center = v3(0.0, 5.5, 0.0);
        for &dist in &[10.0f32, 50.0, 200.0] {
            let mut s1 = vec![0f64; RES * RES];
            let mut s2 = vec![0f64; RES * RES];
            for f in 0..FRAMES {
                let ang = (f as f32 / (FRAMES - 1) as f32) * 4.0f32.to_radians();
                let eye = v3(
                    center.x + dist * ang.cos(),
                    center.y + dist * 0.25,
                    center.z + dist * ang.sin(),
                );
                let cam = CamSample { eye, look: center, fov_deg: 42.0 };
                let rgba = render_frame(&sc, &cam, RES, SS);
                for i in 0..RES * RES {
                    let l = (0.2126 * rgba[i * 4] as f64
                        + 0.7152 * rgba[i * 4 + 1] as f64
                        + 0.0722 * rgba[i * 4 + 2] as f64)
                        / 255.0;
                    s1[i] += l;
                    s2[i] += l * l;
                }
            }
            let n = FRAMES as f64;
            let mean_var = (0..RES * RES)
                .map(|i| (s2[i] / n - (s1[i] / n) * (s1[i] / n)).max(0.0))
                .sum::<f64>()
                / (RES * RES) as f64;
            eprintln!("c3: dist {dist:>5} m  mean per-pixel luminance variance {mean_var:.3e}");
            assert!(
                mean_var < THRESH,
                "temporal shimmer at {dist} m: mean per-pixel luminance variance \
                 {mean_var:.3e} >= threshold {THRESH:.1e}"
            );
        }
    }
}
