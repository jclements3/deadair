//! Editable camera path: a JSON file of timed keyframes, interpolated with a
//! non-uniform Catmull-Rom (cubic Hermite) spline so the dolly is C1-smooth.
//! Duplicate a keyframe at a later time to hold the camera still.

use crate::sdf::{v3, Vec3};
use serde::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Key {
    /// Time in seconds.
    pub t: f32,
    pub eye: [f32; 3],
    pub look: [f32; 3],
    /// Optional per-key vertical FOV override (degrees).
    pub fov_deg: Option<f32>,
    #[serde(default)]
    pub label: String,
}

#[derive(Deserialize, Clone)]
pub struct PathSpec {
    #[serde(default = "d_size")]
    pub size: u32,
    #[serde(default = "d_fps")]
    pub fps: f32,
    /// Supersampling grid side: 4 -> 4x4 = 16 stratified samples per pixel.
    #[serde(default = "d_ss")]
    pub supersample: u32,
    #[serde(default = "d_fov")]
    pub fov_deg: f32,
    #[serde(default = "d_quality")]
    pub quality: f32,
    pub keys: Vec<Key>,
}

fn d_size() -> u32 {
    1024
}
fn d_fps() -> f32 {
    20.0
}
fn d_ss() -> u32 {
    4
}
fn d_fov() -> f32 {
    42.0
}
fn d_quality() -> f32 {
    92.0
}

pub struct CamSample {
    pub eye: Vec3,
    pub look: Vec3,
    pub fov_deg: f32,
}

impl PathSpec {
    pub fn load(path: &str) -> PathSpec {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("cannot read camera path {path}: {e}"));
        let spec: PathSpec = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("cannot parse camera path {path}: {e}"));
        assert!(spec.keys.len() >= 2, "camera path needs at least 2 keyframes");
        for w in spec.keys.windows(2) {
            assert!(
                w[1].t > w[0].t,
                "keyframe times must be strictly increasing ({} then {})",
                w[0].t,
                w[1].t
            );
        }
        spec
    }

    pub fn duration(&self) -> f32 {
        self.keys.last().unwrap().t - self.keys[0].t
    }

    /// Sample the spline at absolute time `t` (seconds).
    pub fn sample(&self, t: f32) -> CamSample {
        let keys = &self.keys;
        let t = t.clamp(keys[0].t, keys.last().unwrap().t);
        let mut i = 0;
        while i + 2 < keys.len() && t > keys[i + 1].t {
            i += 1;
        }
        let (k0, k1) = (&keys[i], &keys[i + 1]);
        let h = k1.t - k0.t;
        let u = ((t - k0.t) / h).clamp(0.0, 1.0);

        let eye = hermite(keys, i, u, |k| Vec3::from_arr(k.eye));
        let look = hermite(keys, i, u, |k| Vec3::from_arr(k.look));
        let f0 = k0.fov_deg.unwrap_or(self.fov_deg);
        let f1 = k1.fov_deg.unwrap_or(self.fov_deg);
        let su = u * u * (3.0 - 2.0 * u);
        CamSample { eye, look, fov_deg: f0 + (f1 - f0) * su }
    }
}

/// Cubic Hermite over segment [i, i+1] with non-uniform finite-difference
/// (Catmull-Rom) tangents; one-sided at the endpoints.
fn hermite(keys: &[Key], i: usize, u: f32, get: impl Fn(&Key) -> Vec3) -> Vec3 {
    let p1 = get(&keys[i]);
    let p2 = get(&keys[i + 1]);
    let (t1, t2) = (keys[i].t, keys[i + 1].t);
    let h = t2 - t1;

    let m1 = if i == 0 {
        (p2 - p1) * (1.0 / h)
    } else {
        let p0 = get(&keys[i - 1]);
        let t0 = keys[i - 1].t;
        ((p1 - p0) * (1.0 / (t1 - t0)) + (p2 - p1) * (1.0 / h)) * 0.5
    };
    let m2 = if i + 2 >= keys.len() {
        (p2 - p1) * (1.0 / h)
    } else {
        let p3 = get(&keys[i + 2]);
        let t3 = keys[i + 2].t;
        ((p2 - p1) * (1.0 / h) + (p3 - p2) * (1.0 / (t3 - t2))) * 0.5
    };

    let u2 = u * u;
    let u3 = u2 * u;
    let h00 = 2.0 * u3 - 3.0 * u2 + 1.0;
    let h10 = u3 - 2.0 * u2 + u;
    let h01 = -2.0 * u3 + 3.0 * u2;
    let h11 = u3 - u2;
    p1 * h00 + m1 * (h10 * h) + p2 * h01 + m2 * (h11 * h)
}

pub fn _unused() -> Vec3 {
    v3(0.0, 0.0, 0.0)
}
