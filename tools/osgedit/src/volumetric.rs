//! Pass 2: FIRE / FOG / CLOUD as a post-process over the solid render.
//!
//! Volumetrics are *not* primitives and never enter the SDF tree. Pass 1
//! marches the CSG exactly as it always did and now also hands out the ray
//! parameter it stopped at (`t_hit`, +inf on a sky miss). Pass 2 takes that
//! depth, and for every volumetric AABB the ray crosses accumulates density
//! over `[t_enter, min(t_exit, t_hit)]` front-to-back, compositing over the
//! pass-1 colour.
//!
//! Clipping the march to `t_hit` is what buys correct occlusion for free:
//! fog in front of a building dims it, a cloud behind a mountain is cut off
//! at the rock, and a fire on terrain reads as grounded rather than pasted.
//! A volume entirely behind the nearest solid contributes nothing and is
//! skipped before a single density sample is taken.
//!
//! Determinism: hand-rolled value-noise FBM, no `rand`, no clock. Density is
//! a pure function of `(x, y, z, seed, phase)`, and the one stochastic term
//! (the step-offset jitter that keeps fixed stepping from banding) is drawn
//! from the same frame-invariant per-sample seed the pixel jitter uses. Same
//! text plus same phase gives identical pixels, every run and every frame.

use crate::render::{hash, SUN_DIR};
use crate::sdf::{v3, Aabb, Vec3};
use serde::Deserialize;

// --- quality knobs ----------------------------------------------------------

/// Hard cap on samples per volume per ray. The per-kind step sizes below all
/// land well under this at sane scales; the cap is what stops a pathological
/// scale (a 10 km fog slab) from stalling the render.
const MAX_STEPS: usize = 96;
/// Stop marching once the volume is essentially opaque.
const MIN_TRANS: f32 = 0.01;
/// Most volumes a single ray will accumulate through.
const MAX_ON_RAY: usize = 8;

// --- schema -----------------------------------------------------------------

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VolKind {
    Fire,
    Fog,
    Cloud,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct BoundsSpec {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

/// Only the fields the kind uses are written; the rest default.
#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct Params {
    #[serde(default)]
    pub r: f32,
    #[serde(default)]
    pub h: f32,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub d: f32,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub phase: f32,
}

/// One entry of the model file's `"volumetrics"` array.
#[derive(Clone, Debug, Deserialize)]
pub struct VolumetricSpec {
    pub kind: VolKind,
    pub bounds: BoundsSpec,
    #[serde(default)]
    pub params: Params,
    /// local -> world affine, row-major 4x4.
    pub xform: [f32; 16],
}

// --- loaded form ------------------------------------------------------------

/// A cloud is a handful of overlapping ellipsoids, placed from the seed at
/// load time so the per-sample density stays cheap.
#[derive(Clone, Copy, Debug)]
struct Puff {
    c: Vec3,
    /// reciprocal radii, so the density inner loop multiplies
    inv_r: Vec3,
}

#[derive(Clone, Debug)]
pub struct Volumetric {
    pub kind: VolKind,
    pub bounds: Aabb,
    pub p: Params,
    /// world -> local affine, row-major 3x4 (the inverse of `xform`).
    inv: [f32; 12],
    puffs: Vec<Puff>,
}

impl Volumetric {
    /// Build the render-side field, or explain why the record is unusable.
    pub fn load(s: &VolumetricSpec) -> Result<Volumetric, String> {
        let bounds = Aabb::new(Vec3::from_arr(s.bounds.min), Vec3::from_arr(s.bounds.max));
        if !bounds.is_finite() {
            return Err(format!(
                "{:?} volumetric: bounds must be a finite AABB (got [{:?}] .. [{:?}]) -- \
                 an unbounded extent has no ray entry/exit and breaks scene fitting",
                s.kind, s.bounds.min, s.bounds.max
            ));
        }
        let inv = affine_inverse(&s.xform).ok_or_else(|| {
            format!("{:?} volumetric: `xform` is singular (a zero scale?)", s.kind)
        })?;
        let puffs = if s.kind == VolKind::Cloud { cloud_puffs(s.params) } else { Vec::new() };
        Ok(Volumetric { kind: s.kind, bounds, p: s.params, inv, puffs })
    }

    fn to_local(&self, p: Vec3) -> Vec3 {
        let m = &self.inv;
        v3(
            m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3],
            m[4] * p.x + m[5] * p.y + m[6] * p.z + m[7],
            m[8] * p.x + m[9] * p.y + m[10] * p.z + m[11],
        )
    }

    /// World-space march step for this field: fine enough that the structure
    /// the noise puts in is resolved, coarse enough to stay affordable.
    fn step(&self) -> f32 {
        match self.kind {
            VolKind::Fire => (0.045 * self.p.h).max(0.01),
            VolKind::Fog => (0.09 * self.p.h).max(0.25),
            VolKind::Cloud => (0.085 * self.p.r).max(0.02),
        }
    }
}

/// Invert a row-major 4x4 affine (bottom row assumed 0,0,0,1) into a 3x4.
fn affine_inverse(m: &[f32; 16]) -> Option<[f32; 12]> {
    let (a, b, c) = (m[0], m[1], m[2]);
    let (d, e, f) = (m[4], m[5], m[6]);
    let (g, h, i) = (m[8], m[9], m[10]);
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let s = 1.0 / det;
    let r = [
        (e * i - f * h) * s,
        (c * h - b * i) * s,
        (b * f - c * e) * s,
        (f * g - d * i) * s,
        (a * i - c * g) * s,
        (c * d - a * f) * s,
        (d * h - e * g) * s,
        (b * g - a * h) * s,
        (a * e - b * d) * s,
    ];
    let t = [m[3], m[7], m[11]];
    Some([
        r[0],
        r[1],
        r[2],
        -(r[0] * t[0] + r[1] * t[1] + r[2] * t[2]),
        r[3],
        r[4],
        r[5],
        -(r[3] * t[0] + r[4] * t[1] + r[5] * t[2]),
        r[6],
        r[7],
        r[8],
        -(r[6] * t[0] + r[7] * t[1] + r[8] * t[2]),
    ])
}

// ---------------------------------------------------------------------------
// deterministic 3D value noise (same family as render.rs `hash`/`noise2`)
// ---------------------------------------------------------------------------

fn vhash(ix: i64, iy: i64, iz: i64, seed: u64) -> f32 {
    let mut s = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    s ^= (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    s ^= (iz as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
    s ^= seed.wrapping_mul(0x27D4_EB2F_1656_67C5);
    hash(s) as f32
}

/// Trilinear value noise on the integer lattice, smoothstep-faded. In [0, 1].
fn vnoise3(p: Vec3, seed: u64) -> f32 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (tx, ty, tz) = (p.x - fx, p.y - fy, p.z - fz);
    let sx = tx * tx * (3.0 - 2.0 * tx);
    let sy = ty * ty * (3.0 - 2.0 * ty);
    let sz = tz * tz * (3.0 - 2.0 * tz);
    let (ix, iy, iz) = (fx as i64, fy as i64, fz as i64);
    let c = |dx: i64, dy: i64, dz: i64| vhash(ix + dx, iy + dy, iz + dz, seed);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), sx);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), sx);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), sx);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), sx);
    lerp(lerp(x00, x10, sy), lerp(x01, x11, sy), sz)
}

/// Fractal sum, `oct` octaves, gain 0.5, lacunarity 2.17 (irrational-ish so
/// the octaves do not align on the lattice). Normalized to ~[0, 1].
fn fbm3(p: Vec3, seed: u64, oct: u32) -> f32 {
    let mut amp = 0.5f32;
    let mut freq = 1.0f32;
    let mut sum = 0.0f32;
    let mut norm = 0.0f32;
    for k in 0..oct {
        sum += amp * vnoise3(p * freq, seed ^ ((k as u64 + 1).wrapping_mul(0x9E37_79B1)));
        norm += amp;
        amp *= 0.5;
        freq *= 2.17;
    }
    if norm > 0.0 {
        sum / norm
    } else {
        0.0
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ---------------------------------------------------------------------------
// density + shading, per kind (all in the field's local frame, Y up)
// ---------------------------------------------------------------------------

/// What one sample of a field contributes: an extinction coefficient (per
/// world metre), the colour that extinction scatters, and an emission that
/// ignores lighting entirely.
struct Sample {
    sigma: f32,
    albedo: Vec3,
    emission: Vec3,
}

const CLEAR: Sample =
    Sample { sigma: 0.0, albedo: Vec3::ZERO, emission: Vec3::ZERO };

// --- fire -------------------------------------------------------------------

/// Flame density: a tapering column, swayed and eroded by rising noise.
/// `phase` scrolls the noise downward through the column, which reads as the
/// flame licking upward.
fn fire_density(p: Vec3, q: &Params) -> (f32, f32) {
    let (r, h) = (q.r.max(1e-4), q.h.max(1e-4));
    let f = p.y / h;
    if !(-0.05..=1.25).contains(&f) {
        return (0.0, f);
    }
    // sway: the column leans and twists with height, driven by phase
    let ph = q.phase;
    let sway = 0.20 * r * f * (p.y * 2.4 / h * std::f32::consts::PI + ph * 3.1).sin();
    let swz = 0.14 * r * f * (p.y * 2.0 / h * std::f32::consts::PI + ph * 2.3).cos();
    let (lx, lz) = (p.x - sway, p.z - swz);
    let lat = (lx * lx + lz * lz).sqrt();

    // Two noise fields, both scrolling downward through the column so the
    // structure appears to rise: one warps the flame's *edge* (this is what
    // makes licks rather than a smooth cone), one modulates the interior.
    let warp = fbm3(
        v3(lx * 4.6 / r, p.y * 3.1 / h - ph * 2.4, lz * 4.6 / r),
        q.seed ^ 0xF12E_0001,
        4,
    );
    let body = fbm3(
        v3(lx * 8.5 / r + 11.0, p.y * 5.5 / h - ph * 3.3, lz * 8.5 / r - 7.0),
        q.seed ^ 0xF12E_0002,
        3,
    );

    let rr = (r * (1.0 - 0.62 * f.clamp(0.0, 1.0))).max(0.08 * r);
    // noisy silhouette: 0.45rr .. 1.45rr, wobbling with height and phase
    let edge = rr * (0.45 + 1.0 * warp);
    let radial = 1.0 - smoothstep(0.30 * edge, edge, lat);
    let base = smoothstep(-0.05, 0.10, f);

    // The cut-off climbs with height: solid at the base, breaking into
    // detached licks near the tip, gone by f = 1.
    let cut = 0.08 + 0.72 * f.clamp(0.0, 1.0);
    let d = radial * base * (0.42 + 1.30 * body) - cut;
    (d.max(0.0) * 2.4, f)
}

/// white -> yellow -> orange -> red -> clear over the height fraction.
fn fire_ramp(f: f32) -> Vec3 {
    const WHITE: Vec3 = v3(1.00, 0.96, 0.88);
    const YELLOW: Vec3 = v3(1.00, 0.82, 0.28);
    const ORANGE: Vec3 = v3(1.00, 0.42, 0.07);
    const RED: Vec3 = v3(0.82, 0.11, 0.02);
    let f = f.max(0.0);
    if f < 0.12 {
        WHITE.lerp(YELLOW, f / 0.12)
    } else if f < 0.38 {
        YELLOW.lerp(ORANGE, (f - 0.12) / 0.26)
    } else if f < 0.72 {
        ORANGE.lerp(RED, (f - 0.38) / 0.34)
    } else {
        RED * (1.0 - smoothstep(0.72, 1.15, f))
    }
}

fn fire_sample(p: Vec3, q: &Params) -> Sample {
    let (d, f) = fire_density(p, q);
    if d <= 0.0 {
        return CLEAR;
    }
    Sample {
        // Flame is emissive, not occluding: extinction stays low so the
        // column adds light instead of stamping a dark silhouette on the
        // sky where the ramp fades out. (No smoke plume is modeled -- that
        // is what `fog` is for.)
        sigma: d * 0.30,
        albedo: v3(0.22, 0.14, 0.09),
        emission: fire_ramp(f) * (d * 7.5),
    }
}

// --- fog --------------------------------------------------------------------

fn fog_sample(p: Vec3, q: &Params, rd: Vec3) -> Sample {
    let h = q.h.max(1e-4);
    let hy = (p.y / h).clamp(0.0, 1.0);
    // exp(-k z) slab: dense at the ground, thinning upward
    let vert = (-2.6 * hy).exp();
    let ph = q.phase;
    let n = fbm3(
        v3(p.x * 0.09 + ph * 0.30, p.y * 0.30, p.z * 0.09 - ph * 0.17),
        q.seed ^ 0xF06_0002,
        3,
    );
    // soften the slab walls so the box never shows
    let edge = |v: f32, ext: f32| 1.0 - smoothstep(0.36 * ext, 0.50 * ext, v.abs());
    let fade = edge(p.x, q.w.max(1e-4)) * edge(p.z, q.d.max(1e-4))
        * (1.0 - smoothstep(0.80, 1.0, hy));
    // 0.20 / m at the ground, before the height and noise terms: thick
    // enough that a few metres of slab reads, thin enough that a 40 m
    // sightline down a colonnade still resolves the far end.
    let sigma = 0.20 * vert * fade * (0.30 + 1.30 * n);
    if sigma <= 1e-4 {
        return CLEAR;
    }
    // multiplicative attenuation toward the fog colour, lightened slightly
    // where the ray looks into the sun
    let sun = SUN_DIR.normalize();
    let glow = rd.dot(sun).max(0.0);
    Sample {
        sigma,
        albedo: v3(0.78, 0.82, 0.89) * (0.86 + 0.42 * glow * glow),
        emission: Vec3::ZERO,
    }
}

// --- cloud ------------------------------------------------------------------

/// 5..9 sub-ellipsoids placed by `hash(seed, i)`. The placement envelope is
/// held inside `CLOUD_XZ_BOUND`/`CLOUD_Y_BOUND` from the exporter so the
/// declared AABB always contains the field.
fn cloud_puffs(q: Params) -> Vec<Puff> {
    let r = q.r.max(1e-4);
    let pick = |i: u64| hash(q.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ i.wrapping_mul(0x1000_0193)) as f32;
    let n = 5 + (pick(0) * 4.999) as usize; // 5..9
    let mut out = Vec::with_capacity(n);
    // Envelope: |c| <= 0.62r in xz and 0.28r in y, radii <= 0.75r (0.70x in
    // y). That keeps the field inside the exporter's declared
    // CLOUD_XZ_BOUND = 1.45r / CLOUD_Y_BOUND = 0.85r -- see the
    // `declared_bounds_contain_the_field` test.
    for i in 0..n as u64 {
        let a = pick(i * 7 + 1) * std::f32::consts::TAU;
        let rad = 0.62 * r * pick(i * 7 + 2).sqrt();
        let cy = (pick(i * 7 + 3) - 0.5) * 0.56 * r;
        let rr = r * (0.42 + 0.33 * pick(i * 7 + 4));
        out.push(Puff {
            c: v3(rad * a.cos(), cy, rad * a.sin()),
            inv_r: v3(1.0 / rr, 1.0 / (rr * 0.70), 1.0 / rr),
        });
    }
    out
}

fn cloud_density(p: Vec3, q: &Params, puffs: &[Puff]) -> f32 {
    // shape: 1 at a puff centre, 0 on its surface, negative outside
    let mut shape = -1.0f32;
    for pf in puffs {
        let d = (p - pf.c).mul(pf.inv_r).length();
        shape = shape.max(1.0 - d);
    }
    if shape <= -0.35 {
        return 0.0;
    }
    let r = q.r.max(1e-4);
    let ph = q.phase;
    let n = fbm3(
        v3(p.x * 1.7 / r + ph * 0.11, p.y * 1.7 / r, p.z * 1.7 / r - ph * 0.07),
        q.seed ^ 0xC10D_0003,
        4,
    );
    // erode the edges with the noise: the interior survives, the rim breaks up
    ((shape - 0.38 * (1.0 - n)) * 2.2).clamp(0.0, 1.0)
}

fn cloud_sample(p: Vec3, q: &Params, puffs: &[Puff]) -> Sample {
    let d = cloud_density(p, q, puffs);
    if d <= 1e-3 {
        return CLEAR;
    }
    // Lambert on the density gradient: density falls off outward, so -grad
    // is the outward normal. Forward differences at a deliberately coarse
    // step -- we want the puff's shape, not the noise's per-sample wiggle.
    let hstep = 0.16 * q.r.max(1e-3);
    let gx = cloud_density(p + v3(hstep, 0.0, 0.0), q, puffs) - d;
    let gy = cloud_density(p + v3(0.0, hstep, 0.0), q, puffs) - d;
    let gz = cloud_density(p + v3(0.0, 0.0, hstep), q, puffs) - d;
    let g = v3(gx, gy, gz);
    let lam = if g.length() > 1e-6 {
        (-g.normalize()).dot(SUN_DIR.normalize()).max(0.0)
    } else {
        0.5
    };
    let shadow = v3(0.46, 0.52, 0.64);
    let lit = v3(1.00, 0.98, 0.94);
    Sample {
        // sigma is per *world* metre; a cloud authored at radius r should be
        // about as opaque at any r, so scale the coefficient by 1/r
        sigma: d * 3.6 / q.r.max(1e-3),
        albedo: shadow.lerp(lit, lam * lam * (3.0 - 2.0 * lam)),
        emission: Vec3::ZERO,
    }
}

// ---------------------------------------------------------------------------
// the pass
// ---------------------------------------------------------------------------

/// Composite every volumetric the ray crosses over the pass-1 colour.
///
/// `t_hit` is pass 1's depth for this ray (`f32::INFINITY` for a sky miss);
/// every march is clipped to it, so a solid always occludes what is behind
/// it. `seed` is the render's frame-invariant per-sample seed -- it only
/// picks the sub-step offset, so the result is stable across frames.
pub fn composite(
    vols: &[Volumetric],
    ro: Vec3,
    rd: Vec3,
    t_hit: f32,
    base: Vec3,
    seed: u64,
) -> Vec3 {
    let inv_rd = v3(
        1.0 / if rd.x.abs() < 1e-9 { 1e-9 } else { rd.x },
        1.0 / if rd.y.abs() < 1e-9 { 1e-9 } else { rd.y },
        1.0 / if rd.z.abs() < 1e-9 { 1e-9 } else { rd.z },
    );

    // gather the crossings, nearest entry first
    let mut hits: [(f32, f32, usize); MAX_ON_RAY] = [(0.0, 0.0, 0); MAX_ON_RAY];
    let mut n = 0usize;
    for (i, v) in vols.iter().enumerate() {
        if n >= MAX_ON_RAY {
            break;
        }
        // the slab test clips to t_hit itself: a volume entirely behind the
        // nearest solid produces no range at all
        if let Some((t0, t1)) = v.bounds.ray_hit(ro, inv_rd, t_hit) {
            if t1 > t0 {
                hits[n] = (t0, t1, i);
                n += 1;
            }
        }
    }
    if n == 0 {
        return base;
    }
    hits[..n].sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    let jitter = hash(seed ^ 0xA24B_AED4_963E_E407) as f32;
    let mut acc = Vec3::ZERO;
    let mut trans = 1.0f32;

    for &(t0, t1, i) in &hits[..n] {
        if trans < MIN_TRANS {
            break;
        }
        let v = &vols[i];
        let span = t1 - t0;
        let steps = ((span / v.step()).ceil() as usize).clamp(1, MAX_STEPS);
        let dt = span / steps as f32;
        // a fixed grid bands badly; the offset is deterministic per sample
        let mut t = t0 + jitter * dt;
        for _ in 0..steps {
            let p = v.to_local(ro + rd * t);
            let s = match v.kind {
                VolKind::Fire => fire_sample(p, &v.p),
                VolKind::Fog => fog_sample(p, &v.p, rd),
                VolKind::Cloud => cloud_sample(p, &v.p, &v.puffs),
            };
            if s.sigma > 0.0 || s.emission.x > 0.0 {
                let a = 1.0 - (-s.sigma * dt).exp();
                acc = acc + (s.albedo * a + s.emission * dt) * trans;
                trans *= 1.0 - a;
                if trans < MIN_TRANS {
                    break;
                }
            }
            t += dt;
        }
    }
    acc + base * trans
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(kind: VolKind, p: Params, bounds: ([f32; 3], [f32; 3])) -> VolumetricSpec {
        VolumetricSpec {
            kind,
            bounds: BoundsSpec { min: bounds.0, max: bounds.1 },
            params: p,
            xform: [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    fn a_cloud(seed: u64) -> Volumetric {
        let p = Params { r: 3.0, seed, ..Default::default() };
        Volumetric::load(&spec(
            VolKind::Cloud,
            p,
            ([-4.35, -2.55, -4.35], [4.35, 2.55, 4.35]),
        ))
        .unwrap()
    }

    /// Every seed must actually produce a field -- a degenerate placement
    /// would render an invisible cloud with no error anywhere.
    #[test]
    fn every_seed_produces_a_dense_core() {
        for seed in 0u64..24 {
            let q = Params { r: 3.0, seed, ..Default::default() };
            let puffs = cloud_puffs(q);
            assert!((5..=9).contains(&puffs.len()), "seed {seed}: {} puffs", puffs.len());
            let mut mx = 0.0f32;
            for pf in &puffs {
                mx = mx.max(cloud_density(pf.c, &q, &puffs));
            }
            assert!(mx > 0.5, "seed {seed}: peak density only {mx}");
        }
    }

    #[test]
    fn affine_inverse_round_trips() {
        // translate(3, -4, 5) * scale(2, 0.5, 3), row-major
        let m = [
            2.0, 0.0, 0.0, 3.0, //
            0.0, 0.5, 0.0, -4.0, //
            0.0, 0.0, 3.0, 5.0, //
            0.0, 0.0, 0.0, 1.0,
        ];
        let inv = affine_inverse(&m).unwrap();
        let p = v3(1.0, 2.0, -1.0);
        let w = v3(
            m[0] * p.x + m[1] * p.y + m[2] * p.z + m[3],
            m[4] * p.x + m[5] * p.y + m[6] * p.z + m[7],
            m[8] * p.x + m[9] * p.y + m[10] * p.z + m[11],
        );
        let back = v3(
            inv[0] * w.x + inv[1] * w.y + inv[2] * w.z + inv[3],
            inv[4] * w.x + inv[5] * w.y + inv[6] * w.z + inv[7],
            inv[8] * w.x + inv[9] * w.y + inv[10] * w.z + inv[11],
        );
        assert!((back - p).length() < 1e-5, "{back:?}");
    }

    #[test]
    fn singular_xform_is_rejected() {
        let mut s = spec(VolKind::Cloud, Params { r: 1.0, ..Default::default() },
                         ([-2.0; 3], [2.0; 3]));
        s.xform[0] = 0.0; // zero x scale
        assert!(Volumetric::load(&s).unwrap_err().contains("singular"));
    }

    #[test]
    fn infinite_bounds_are_rejected() {
        let s = spec(
            VolKind::Fog,
            Params { w: 4.0, d: 4.0, h: 1.0, ..Default::default() },
            ([-1.0e9, 0.0, -1.0e9], [1.0e9, 1.0, 1.0e9]),
        );
        assert!(Volumetric::load(&s).unwrap_err().contains("finite"));
    }

    /// Density must be a pure function of position: same point, same value.
    #[test]
    fn density_is_a_pure_function() {
        let c = a_cloud(5);
        let p = v3(0.4, 0.1, -0.7);
        let a = cloud_density(p, &c.p, &c.puffs);
        let b = cloud_density(p, &c.p, &c.puffs);
        assert_eq!(a.to_bits(), b.to_bits());
        let q = Params { r: 1.0, h: 2.0, seed: 3, phase: 0.4, ..Default::default() };
        assert_eq!(fire_density(p, &q).0.to_bits(), fire_density(p, &q).0.to_bits());
    }

    /// Different seeds must give different fields.
    #[test]
    fn seed_changes_the_field() {
        let (a, b) = (a_cloud(1), a_cloud(2));
        let mut diff = 0.0f32;
        for i in 0..400 {
            let t = i as f32 * 0.017;
            let p = v3(t.sin() * 2.0, (t * 1.7).cos() * 1.2, (t * 0.9).sin() * 2.0);
            diff += (cloud_density(p, &a.p, &a.puffs) - cloud_density(p, &b.p, &b.puffs)).abs();
        }
        assert!(diff > 1.0, "seeds 1 and 2 produced near-identical clouds ({diff})");
    }

    /// A phase sweep must move the field smoothly -- no popping between
    /// neighbouring frames. Compare the per-step change against the change
    /// over a 10x larger phase jump: smooth motion keeps the ratio small.
    #[test]
    fn phase_sweep_is_smooth() {
        let pts: Vec<Vec3> = (0..300)
            .map(|i| {
                let t = i as f32 * 0.021;
                v3(t.sin() * 0.5, 0.2 + (t * 1.3).cos().abs() * 1.4, (t * 0.7).sin() * 0.5)
            })
            .collect();
        let dens = |ph: f32| -> Vec<f32> {
            let q = Params { r: 0.6, h: 2.0, seed: 7, phase: ph, ..Default::default() };
            pts.iter().map(|p| fire_density(*p, &q).0).collect()
        };
        let l1 = |a: &[f32], b: &[f32]| {
            a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>()
        };
        let base = dens(0.0);
        let small = l1(&base, &dens(0.02));
        let big = l1(&base, &dens(0.20));
        assert!(small > 0.0, "phase did nothing");
        // a 10x bigger phase step must move things a lot more than 3x as far,
        // i.e. the small step is genuinely a small motion, not a re-roll
        assert!(
            small * 3.0 < big,
            "phase 0.02 moved {small:.4} but phase 0.20 only {big:.4} -- \
             the sweep is popping, not sweeping"
        );
    }

    /// Occlusion: a solid nearer than the volume's entry leaves pass 1 alone.
    #[test]
    fn a_nearer_solid_is_untouched() {
        let c = a_cloud(4);
        let vols = [c];
        let ro = v3(0.0, 0.0, -40.0);
        let rd = v3(0.0, 0.0, 1.0);
        let base = v3(0.3, 0.4, 0.5);
        // t_enter for the cloud is ~35.65; a solid at t = 10 is well in front
        let front = composite(&vols, ro, rd, 10.0, base, 12345);
        assert_eq!(front.x.to_bits(), base.x.to_bits());
        assert_eq!(front.y.to_bits(), base.y.to_bits());
        assert_eq!(front.z.to_bits(), base.z.to_bits());
        // with no solid in the way the same ray must actually change
        let sky = composite(&vols, ro, rd, f32::INFINITY, base, 12345);
        assert!((sky - base).length() > 0.05, "cloud contributed nothing: {sky:?}");
    }

    /// A solid part-way through the volume clips the march: strictly less
    /// contribution than the unoccluded ray, strictly more than none.
    #[test]
    fn a_solid_inside_the_volume_clips_the_march() {
        let c = a_cloud(4);
        let vols = [c];
        let ro = v3(0.0, 0.0, -40.0);
        let rd = v3(0.0, 0.0, 1.0);
        let base = v3(0.0, 0.0, 0.0);
        let full = composite(&vols, ro, rd, f32::INFINITY, base, 7).length();
        let half = composite(&vols, ro, rd, 40.0, base, 7).length();
        assert!(half > 0.0, "clipped ray got nothing");
        assert!(half < full, "clipping did not reduce the contribution ({half} vs {full})");
    }

    /// Bit-exact repeatability of the whole pass.
    #[test]
    fn composite_is_bit_identical_on_repeat() {
        let c = a_cloud(9);
        let vols = [c];
        let ro = v3(1.0, -2.0, -30.0);
        let rd = v3(0.02, 0.05, 1.0).normalize();
        let a = composite(&vols, ro, rd, f32::INFINITY, v3(0.2, 0.3, 0.4), 99);
        let b = composite(&vols, ro, rd, f32::INFINITY, v3(0.2, 0.3, 0.4), 99);
        assert_eq!(a.x.to_bits(), b.x.to_bits());
        assert_eq!(a.y.to_bits(), b.y.to_bits());
        assert_eq!(a.z.to_bits(), b.z.to_bits());
    }

    /// The exporter's declared bounds must actually contain the field. Sample
    /// a shell just outside each local AABB and require zero density.
    #[test]
    fn declared_bounds_contain_the_field() {
        // cloud: exporter uses +/-1.45r in xz, +/-0.85r in y
        let c = a_cloud(3);
        let r = c.p.r;
        for i in 0..2000 {
            let t = i as f32 * 0.0031;
            let dir = v3((t * 6.1).sin(), (t * 2.7).cos(), (t * 4.3).sin()).normalize();
            let p = v3(dir.x * 1.46 * r, dir.y * 0.86 * r, dir.z * 1.46 * r);
            // only test points genuinely outside the box
            if p.x.abs() > 1.45 * r || p.y.abs() > 0.85 * r || p.z.abs() > 1.45 * r {
                assert_eq!(
                    cloud_density(p, &c.p, &c.puffs),
                    0.0,
                    "cloud density outside the declared bounds at {p:?}"
                );
            }
        }
        // fire: exporter uses +/-1.25r in xz, [-0.02h, 1.20h] in y
        let q = Params { r: 0.6, h: 2.0, seed: 7, phase: 0.7, ..Default::default() };
        for i in 0..2000 {
            let t = i as f32 * 0.0037;
            let p = v3(
                1.26 * q.r * (t * 5.3).cos(),
                -0.03 * q.h + 1.24 * q.h * ((t * 3.1).sin() * 0.5 + 0.5),
                1.26 * q.r * (t * 5.3).sin(),
            );
            let outside = p.x.abs() > 1.25 * q.r
                || p.z.abs() > 1.25 * q.r
                || p.y < -0.02 * q.h
                || p.y > 1.20 * q.h;
            if outside {
                assert_eq!(fire_density(p, &q).0, 0.0, "fire density outside bounds at {p:?}");
            }
        }
    }

    /// Fire is emissive: it adds light rather than only attenuating.
    #[test]
    fn fire_adds_light() {
        let q = Params { r: 0.6, h: 2.0, seed: 7, ..Default::default() };
        let v = Volumetric::load(&spec(
            VolKind::Fire,
            q,
            ([-0.75, -0.04, -0.75], [0.75, 2.4, 0.75]),
        ))
        .unwrap();
        let vols = [v];
        let base = v3(0.05, 0.05, 0.05);
        // look horizontally through the middle of the column
        let out = composite(&vols, v3(0.0, 0.5, -6.0), v3(0.0, 0.0, 1.0), f32::INFINITY, base, 3);
        assert!(out.x > base.x + 0.05, "fire did not brighten the pixel: {out:?}");
        assert!(out.x > out.z, "fire should be warm-tinted: {out:?}");
    }
}
