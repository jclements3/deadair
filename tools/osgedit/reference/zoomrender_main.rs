// Zoom-in animation on a basketball-sized green sphere, flanked by a red cube
// (1 m to its left) and a blue cone (1 m to its right), on a sunny day.
// Camera dollies from "sphere is ~1 pixel" to 1 mm from its surface.
// Anti-shimmer: 16x stratified supersampling per pixel (jittered), gamma-correct
// filtering. Output: animated WebP, 1024x1024.

use rayon::prelude::*;
use std::ops::{Add, Mul, Neg, Sub};
use webp_animation::{Encoder, EncoderOptions, EncodingConfig, EncodingType, LossyEncodingConfig};

#[derive(Clone, Copy, Debug)]
struct V3 {
    x: f64,
    y: f64,
    z: f64,
}
fn v(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}
impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl Mul<f64> for V3 {
    type Output = V3;
    fn mul(self, s: f64) -> V3 {
        v(self.x * s, self.y * s, self.z * s)
    }
}
impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v(-self.x, -self.y, -self.z)
    }
}
impl V3 {
    fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    fn cross(self, o: V3) -> V3 {
        v(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    fn norm(self) -> V3 {
        self * (1.0 / self.len())
    }
    fn mul_v(self, o: V3) -> V3 {
        v(self.x * o.x, self.y * o.y, self.z * o.z)
    }
}

// ---------- Scene ----------
const BALL_R: f64 = 0.12; // basketball ~24 cm diameter
const H: f64 = 2.0 * BALL_R; // common height 0.24 m
const SPACING: f64 = 1.0;

const SPHERE_C: V3 = V3 { x: 0.0, y: BALL_R, z: 0.0 };
const CUBE_C: V3 = V3 { x: -SPACING, y: BALL_R, z: 0.0 }; // half-extent BALL_R -> height H
const CONE_APEX: V3 = V3 { x: SPACING, y: H, z: 0.0 }; // base radius BALL_R, height H

const GREEN: V3 = V3 { x: 0.10, y: 0.55, z: 0.12 };
const RED: V3 = V3 { x: 0.65, y: 0.07, z: 0.06 };
const BLUE: V3 = V3 { x: 0.08, y: 0.15, z: 0.62 };
const GROUND: V3 = V3 { x: 0.42, y: 0.40, z: 0.34 }; // dry sunny ground

struct Hit {
    t: f64,
    n: V3,
    albedo: V3,
    spec: f64,
}

fn hit_sphere(ro: V3, rd: V3) -> Option<(f64, V3)> {
    let oc = ro - SPHERE_C;
    let b = oc.dot(rd);
    let c = oc.dot(oc) - BALL_R * BALL_R;
    let disc = b * b - c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let mut t = -b - s;
    if t < 1e-7 {
        t = -b + s;
    }
    if t < 1e-7 {
        return None;
    }
    let p = ro + rd * t;
    Some((t, (p - SPHERE_C).norm()))
}

fn hit_cube(ro: V3, rd: V3) -> Option<(f64, V3)> {
    let half = BALL_R;
    let mn = CUBE_C - v(half, half, half);
    let mx = CUBE_C + v(half, half, half);
    let inv = v(1.0 / rd.x, 1.0 / rd.y, 1.0 / rd.z);
    let t1 = (mn - ro).mul_v(inv);
    let t2 = (mx - ro).mul_v(inv);
    let tmin3 = v(t1.x.min(t2.x), t1.y.min(t2.y), t1.z.min(t2.z));
    let tmax3 = v(t1.x.max(t2.x), t1.y.max(t2.y), t1.z.max(t2.z));
    let tmin = tmin3.x.max(tmin3.y).max(tmin3.z);
    let tmax = tmax3.x.min(tmax3.y).min(tmax3.z);
    if tmax < tmin.max(1e-7) {
        return None;
    }
    let t = if tmin > 1e-7 { tmin } else { tmax };
    // normal = axis of the slab we entered through
    let n = if t == tmin3.x || t == tmax3.x {
        v(-rd.x.signum(), 0.0, 0.0)
    } else if t == tmin3.y || t == tmax3.y {
        v(0.0, -rd.y.signum(), 0.0)
    } else {
        v(0.0, 0.0, -rd.z.signum())
    };
    Some((t, n))
}

fn hit_cone(ro: V3, rd: V3) -> Option<(f64, V3)> {
    // Apex at CONE_APEX, axis pointing down (0,-1,0), tan(half-angle) = r/h = 0.5
    let d = v(0.0, -1.0, 0.0);
    let cos2 = (H * H) / (H * H + BALL_R * BALL_R); // cos^2(theta)
    let co = ro - CONE_APEX;
    let rd_d = rd.dot(d);
    let co_d = co.dot(d);
    let a = rd_d * rd_d - cos2;
    let b = rd_d * co_d - rd.dot(co) * cos2;
    let c = co_d * co_d - co.dot(co) * cos2;
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let mut best: Option<(f64, V3)> = None;
    for t in [(-b - s) / a, (-b + s) / a] {
        if t < 1e-7 {
            continue;
        }
        let p = ro + rd * t;
        let cp = p - CONE_APEX;
        let m = cp.dot(d);
        if m < 0.0 || m > H {
            continue; // outside finite cone
        }
        if best.map_or(true, |(bt, _)| t < bt) {
            // outward normal: perpendicular to surface, away from axis
            let axis_pt = CONE_APEX + d * m;
            let out = (p - axis_pt).norm();
            let mut n = (cp * cos2 - d * cp.dot(d)).norm();
            if n.dot(out) < 0.0 {
                n = -n;
            }
            best = Some((t, n));
        }
    }
    best
}

fn hit_ground(ro: V3, rd: V3) -> Option<(f64, V3)> {
    if rd.y.abs() < 1e-9 {
        return None;
    }
    let t = -ro.y / rd.y;
    if t < 1e-7 {
        return None;
    }
    Some((t, v(0.0, 1.0, 0.0)))
}

fn intersect(ro: V3, rd: V3) -> Option<Hit> {
    let mut best: Option<Hit> = None;
    let mut consider = |h: Option<(f64, V3)>, albedo: V3, spec: f64| {
        if let Some((t, n)) = h {
            if best.as_ref().map_or(true, |bh| t < bh.t) {
                best = Some(Hit { t, n, albedo, spec });
            }
        }
    };
    consider(hit_sphere(ro, rd), GREEN, 0.35);
    consider(hit_cube(ro, rd), RED, 0.25);
    consider(hit_cone(ro, rd), BLUE, 0.30);
    consider(hit_ground(ro, rd), GROUND, 0.0);
    best
}

fn occluded(ro: V3, rd: V3) -> bool {
    hit_sphere(ro, rd).is_some() || hit_cube(ro, rd).is_some() || hit_cone(ro, rd).is_some()
}

// ---------- Lighting: sunny day ----------
const SUN_DIR: V3 = V3 { x: -0.35, y: 0.80, z: 0.55 }; // toward the sun (normalized on use)

fn sky(rd: V3) -> V3 {
    // simple clear-sky gradient + sun glow
    let t = rd.y.max(0.0).powf(0.6);
    let horizon = v(0.85, 0.90, 0.98);
    let zenith = v(0.25, 0.48, 0.86);
    let base = horizon * (1.0 - t) + zenith * t;
    let sun = SUN_DIR.norm();
    let a = rd.dot(sun).max(0.0);
    base + v(1.0, 0.95, 0.85) * (a.powf(600.0) * 8.0 + a.powf(6.0) * 0.12)
}

fn shade(ro: V3, rd: V3) -> V3 {
    match intersect(ro, rd) {
        None => sky(rd),
        Some(h) => {
            let p = ro + rd * h.t;
            let sun = SUN_DIR.norm();
            let sun_col = v(1.0, 0.97, 0.90) * 2.6;
            let in_shadow = occluded(p + h.n * 1e-6, sun);
            let mut col = v(0.0, 0.0, 0.0);
            if !in_shadow {
                let ndl = h.n.dot(sun).max(0.0);
                col = col + h.albedo.mul_v(sun_col) * ndl;
                if h.spec > 0.0 {
                    let hv = (sun - rd).norm();
                    let sp = h.n.dot(hv).max(0.0).powf(120.0);
                    col = col + sun_col * (sp * h.spec);
                }
            }
            // sky/ambient fill (hemispheric) + warm ground bounce
            let sky_amb = v(0.35, 0.45, 0.62) * (0.5 * (1.0 + h.n.y)) * 0.55;
            let bounce = v(0.30, 0.28, 0.24) * (0.5 * (1.0 - h.n.y)) * 0.35;
            col + h.albedo.mul_v(sky_amb + bounce)
        }
    }
}

fn tonemap(c: V3) -> V3 {
    let m = |x: f64| {
        let x = (x * (1.0 + x / 4.0)) / (1.0 + x);
        x.clamp(0.0, 1.0).powf(1.0 / 2.2)
    };
    v(m(c.x), m(c.y), m(c.z))
}

// small deterministic hash for per-sample jitter
fn hash(mut s: u64) -> f64 {
    s ^= s >> 33;
    s = s.wrapping_mul(0xff51afd7ed558ccd);
    s ^= s >> 33;
    s = s.wrapping_mul(0xc4ceb9fe1a85ec53);
    s ^= s >> 33;
    (s as f64) / (u64::MAX as f64)
}

const W: usize = 1024;
const HGT: usize = 1024;
const SS: usize = 4; // 4x4 = 16 stratified samples/pixel
const FRAMES: usize = 120;
const FPS: f64 = 30.0;
const FOV_Y_DEG: f64 = 40.0;

fn render_frame(frame: usize) -> Vec<u8> {
    let t01 = frame as f64 / (FRAMES - 1) as f64;
    let e = t01 * t01 * (3.0 - 2.0 * t01); // ease in/out of the zoom

    let fov = FOV_Y_DEG.to_radians();
    // start: sphere subtends ~1 pixel -> distance = diameter / pixel_angle
    let px_angle = fov / HGT as f64;
    let d_start = (2.0 * BALL_R) / px_angle; // ~352 m
    let d_end = BALL_R + 0.001; // stop 1 mm from the surface
    let dist = d_start * (d_end / d_start).powf(e); // exponential dolly = constant zoom feel

    // camera approaches along a fixed direction (slightly elevated, from the front)
    let cam_dir = v(0.18, 0.34, 1.0).norm();
    let eye = SPHERE_C + cam_dir * dist;
    let fwd = (SPHERE_C - eye).norm();
    let right = fwd.cross(v(0.0, 1.0, 0.0)).norm();
    let up = right.cross(fwd);
    let half_h = (fov / 2.0).tan();
    let half_w = half_h * (W as f64 / HGT as f64);

    let mut rgba = vec![0u8; W * HGT * 4];
    rgba.par_chunks_mut(W * 4).enumerate().for_each(|(y, row)| {
        for x in 0..W {
            let mut acc = v(0.0, 0.0, 0.0);
            for sy in 0..SS {
                for sx in 0..SS {
                    let seed = (((frame * HGT + y) as u64) << 24)
                        | ((x as u64) << 8)
                        | ((sy * SS + sx) as u64);
                    let jx = (sx as f64 + hash(seed)) / SS as f64;
                    let jy = (sy as f64 + hash(seed ^ 0x9e3779b97f4a7c15)) / SS as f64;
                    let u = ((x as f64 + jx) / W as f64) * 2.0 - 1.0;
                    let vv = 1.0 - ((y as f64 + jy) / HGT as f64) * 2.0;
                    let rd = (fwd + right * (u * half_w) + up * (vv * half_h)).norm();
                    acc = acc + shade(eye, rd);
                }
            }
            let c = tonemap(acc * (1.0 / (SS * SS) as f64));
            let i = x * 4;
            row[i] = (c.x * 255.0 + 0.5) as u8;
            row[i + 1] = (c.y * 255.0 + 0.5) as u8;
            row[i + 2] = (c.z * 255.0 + 0.5) as u8;
            row[i + 3] = 255;
        }
    });
    rgba
}

fn main() {
    let cfg = EncodingConfig {
        encoding_type: EncodingType::Lossy(LossyEncodingConfig::default()),
        quality: 92.0,
        method: 4,
    };
    let opts = EncoderOptions {
        encoding_config: Some(cfg),
        ..Default::default()
    };
    let mut enc = Encoder::new_with_options((W as u32, HGT as u32), opts).unwrap();

    for f in 0..FRAMES {
        let rgba = render_frame(f);
        let ts = (f as f64 * 1000.0 / FPS) as i32;
        enc.add_frame(&rgba, ts).unwrap();
        // dump a few PNGs for visual verification
        if f == 0 || f == FRAMES / 3 || f == 2 * FRAMES / 3 || f == FRAMES - 1 {
            image::save_buffer(
                format!("frame_{f:03}.png"),
                &rgba,
                W as u32,
                HGT as u32,
                image::ColorType::Rgba8,
            )
            .unwrap();
        }
        eprintln!("frame {f}/{FRAMES}");
    }
    // hold the last frame for half a second before looping
    let final_ts = ((FRAMES as f64 + 15.0) * 1000.0 / FPS) as i32;
    let data = enc.finalize(final_ts).unwrap();
    std::fs::write("zoom.webp", &data).unwrap();
    eprintln!("wrote zoom.webp ({} bytes)", data.len());
}
