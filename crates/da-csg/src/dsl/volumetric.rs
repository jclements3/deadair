//! Volumetric fields (`fire` / `fog` / `cloud`) -- density, not geometry.
//!
//! A volumetric is NOT a CSG primitive and never becomes an SDF operand. The
//! evaluator carries it beside the solid tree and the exporter writes it to a
//! sibling `"volumetrics"` array in the model JSON; the renderer composites it
//! as a second pass over the finished solid image (see
//! `tools/osgedit/src/volumetric.rs`). That split is what makes occlusion
//! correct for free: the solid pass hands pass 2 a depth buffer, and every
//! density march is clipped to it.
//!
//! Only rigid-plus-scale placement is meaningful here, so the accumulated
//! `.move` / `.scale` / `.rotate*` chain is collapsed into one 4x4 affine
//! (local -> world, row-major) and the emitted `bounds` is the post-transform
//! world AABB. Bounds must be finite: the renderer uses them for ray/slab
//! entry-exit, and an unbounded extent would make every ray march the whole
//! world.
//!
//! Everything here is a pure function of the source text -- no RNG, no clock.
//! `seed` picks a noise field, `phase` advances the animation; the same text
//! with the same phase always renders the same pixels.

use serde_json::{json, Value as Json};

// ---------------------------------------------------------------------------
// 4x4 affine, row-major
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f64; 16]);

impl Mat4 {
    pub const IDENT: Mat4 = Mat4([
        1.0, 0.0, 0.0, 0.0, //
        0.0, 1.0, 0.0, 0.0, //
        0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 0.0, 1.0,
    ]);

    /// `self * rhs` -- apply `rhs` first, then `self`.
    pub fn mul(self, rhs: Mat4) -> Mat4 {
        let (a, b) = (self.0, rhs.0);
        let mut o = [0.0f64; 16];
        for r in 0..4 {
            for c in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += a[r * 4 + k] * b[k * 4 + c];
                }
                o[r * 4 + c] = s;
            }
        }
        Mat4(o)
    }

    pub fn translate(x: f64, y: f64, z: f64) -> Mat4 {
        let mut m = Mat4::IDENT.0;
        m[3] = x;
        m[7] = y;
        m[11] = z;
        Mat4(m)
    }

    pub fn scale(x: f64, y: f64, z: f64) -> Mat4 {
        let mut m = Mat4::IDENT.0;
        m[0] = x;
        m[5] = y;
        m[10] = z;
        Mat4(m)
    }

    /// Right-handed rotation of `deg` degrees about the unit `axis`.
    pub fn rot(axis: [f64; 3], deg: f64) -> Mat4 {
        let n = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
        if n <= 0.0 {
            return Mat4::IDENT;
        }
        let (x, y, z) = (axis[0] / n, axis[1] / n, axis[2] / n);
        let (s, c) = deg.to_radians().sin_cos();
        let t = 1.0 - c;
        Mat4([
            t * x * x + c,
            t * x * y - s * z,
            t * x * z + s * y,
            0.0,
            t * x * y + s * z,
            t * y * y + c,
            t * y * z - s * x,
            0.0,
            t * x * z - s * y,
            t * y * z + s * x,
            t * z * z + c,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ])
    }

    pub fn point(&self, p: [f64; 3]) -> [f64; 3] {
        let m = &self.0;
        [
            m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
            m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
        ]
    }
}

// ---------------------------------------------------------------------------
// The three fields
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolKind {
    Fire,
    Fog,
    Cloud,
}

impl VolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VolKind::Fire => "fire",
            VolKind::Fog => "fog",
            VolKind::Cloud => "cloud",
        }
    }
}

/// One placed volumetric. Sizes are in the script's metres; the local frame
/// is **Y-up** (the renderer's world convention), which is why the exporter
/// seeds `xform` with the same +90 deg X rotation the round primitives use to
/// stand up in the script's Z-up frame.
#[derive(Clone, Copy, Debug)]
pub struct Vol {
    pub kind: VolKind,
    /// fire/cloud radius.
    pub r: f64,
    /// fire/fog height.
    pub h: f64,
    /// fog width (x) and depth (z).
    pub w: f64,
    pub d: f64,
    pub seed: u64,
    pub phase: f64,
    /// local -> world affine.
    pub xform: Mat4,
}

/// Bound multipliers, shared with the renderer's density functions. Changing
/// one of these without changing the matching constant in
/// `tools/osgedit/src/volumetric.rs` shows up as a clipped field.
pub const FIRE_R_BOUND: f64 = 1.25; // flame lick vs. the nominal radius
pub const FIRE_H_BOUND: f64 = 1.20; // tip overshoot vs. the nominal height
pub const CLOUD_XZ_BOUND: f64 = 1.45; // puff spread vs. the nominal radius
pub const CLOUD_Y_BOUND: f64 = 0.85; // clouds are squashed in y

impl Vol {
    pub fn fire(r: f64, h: f64, seed: u64, phase: f64) -> Vol {
        Vol { kind: VolKind::Fire, r, h, w: 0.0, d: 0.0, seed, phase, xform: Mat4::IDENT }
    }
    pub fn fog(w: f64, d: f64, h: f64, seed: u64, phase: f64) -> Vol {
        Vol { kind: VolKind::Fog, r: 0.0, h, w, d, seed, phase, xform: Mat4::IDENT }
    }
    pub fn cloud(r: f64, seed: u64, phase: f64) -> Vol {
        Vol { kind: VolKind::Cloud, r, h: 0.0, w: 0.0, d: 0.0, seed, phase, xform: Mat4::IDENT }
    }

    /// Apply a transform on the outside: `m * self.xform`.
    pub fn transformed(mut self, m: Mat4) -> Vol {
        self.xform = m.mul(self.xform);
        self
    }

    /// The local-space extent the density function can reach.
    pub fn local_aabb(&self) -> ([f64; 3], [f64; 3]) {
        match self.kind {
            VolKind::Fire => {
                let rr = self.r * FIRE_R_BOUND;
                ([-rr, -0.02 * self.h, -rr], [rr, FIRE_H_BOUND * self.h, rr])
            }
            VolKind::Fog => (
                [-self.w * 0.5, 0.0, -self.d * 0.5],
                [self.w * 0.5, self.h, self.d * 0.5],
            ),
            VolKind::Cloud => {
                let (a, b) = (self.r * CLOUD_XZ_BOUND, self.r * CLOUD_Y_BOUND);
                ([-a, -b, -a], [a, b, a])
            }
        }
    }

    /// The post-transform world AABB -- what the renderer slab-tests against.
    pub fn world_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let (lo, hi) = self.local_aabb();
        let mut mn = [f64::MAX; 3];
        let mut mx = [f64::MIN; 3];
        for i in 0..8 {
            let c = [
                if i & 1 == 0 { lo[0] } else { hi[0] },
                if i & 2 == 0 { lo[1] } else { hi[1] },
                if i & 4 == 0 { lo[2] } else { hi[2] },
            ];
            let p = self.xform.point(c);
            for k in 0..3 {
                mn[k] = mn[k].min(p[k]);
                mx[k] = mx[k].max(p[k]);
            }
        }
        (mn, mx)
    }

    /// Human-readable tag used in error messages and in the polygon path's
    /// "skipped these" warning.
    pub fn describe(&self) -> String {
        match self.kind {
            VolKind::Fire => format!("fire(r={}, h={})", num(self.r), num(self.h)),
            VolKind::Fog => {
                format!("fog(w={}, d={}, h={})", num(self.w), num(self.d), num(self.h))
            }
            VolKind::Cloud => format!("cloud(r={})", num(self.r)),
        }
    }

    /// The exported record. `params` carries only the fields the kind uses,
    /// so the JSON reads as the call that produced it.
    pub fn to_json(&self) -> Result<Json, String> {
        let (mn, mx) = self.world_bounds();
        for v in mn.iter().chain(mx.iter()) {
            if !v.is_finite() || v.abs() >= 1.0e5 {
                return Err(format!(
                    "{}: volumetric bounds must be finite and inside +/-1e5 \
                     (got [{:.3}, {:.3}, {:.3}] .. [{:.3}, {:.3}, {:.3}]) -- \
                     check the size arguments and the transform chain",
                    self.describe(),
                    mn[0],
                    mn[1],
                    mn[2],
                    mx[0],
                    mx[1],
                    mx[2]
                ));
            }
        }
        let mut params = serde_json::Map::new();
        match self.kind {
            VolKind::Fire => {
                params.insert("r".into(), json!(self.r));
                params.insert("h".into(), json!(self.h));
            }
            VolKind::Fog => {
                params.insert("w".into(), json!(self.w));
                params.insert("d".into(), json!(self.d));
                params.insert("h".into(), json!(self.h));
            }
            VolKind::Cloud => {
                params.insert("r".into(), json!(self.r));
            }
        }
        params.insert("seed".into(), json!(self.seed));
        params.insert("phase".into(), json!(self.phase));
        Ok(json!({
            "kind": self.kind.as_str(),
            "bounds": { "min": mn, "max": mx },
            "params": Json::Object(params),
            "xform": self.xform.0,
        }))
    }
}

/// Format a size the way the source most likely wrote it (no `1.0` for `1`).
fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1.0e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Validate the size arguments of a builtin. Zero or negative extents give a
/// field nothing to occupy, which is a modeling mistake, not a render-time
/// surprise -- fail with the offending name.
pub fn check_positive(what: &str, pairs: &[(&str, f64)]) -> Result<(), String> {
    for (name, v) in pairs {
        if !(*v > 0.0) || !v.is_finite() {
            return Err(format!(
                "`{what}`: argument `{name}` must be a positive, finite number (got {v})"
            ));
        }
    }
    Ok(())
}

/// `-` and `*` have no meaning against a density field. Naming both the
/// operator and the volumetric is the whole point of the message.
pub fn operator_error(op: char, names: &[String]) -> String {
    let word = match op {
        '-' => "difference",
        '*' => "intersection",
        _ => "this operator",
    };
    format!(
        "operator `{op}` ({word}) cannot take a volumetric: {}. Volumetrics are \
         density fields composited over the solid render, not CSG operands -- \
         only `+` (add to the scene) is defined for them.",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_bounds_are_the_local_box() {
        let f = Vol::fog(4.0, 6.0, 2.0, 0, 0.0);
        let (mn, mx) = f.world_bounds();
        assert_eq!(mn, [-2.0, 0.0, -3.0]);
        assert_eq!(mx, [2.0, 2.0, 3.0]);
    }

    #[test]
    fn transform_composes_outside_in() {
        // move after scale: the translation is not scaled.
        let v = Vol::cloud(1.0, 0, 0.0)
            .transformed(Mat4::scale(2.0, 2.0, 2.0))
            .transformed(Mat4::translate(10.0, 0.0, 0.0));
        let (mn, mx) = v.world_bounds();
        assert!((mn[0] - (10.0 - 2.0 * CLOUD_XZ_BOUND)).abs() < 1e-12, "{mn:?}");
        assert!((mx[0] - (10.0 + 2.0 * CLOUD_XZ_BOUND)).abs() < 1e-12, "{mx:?}");
    }

    #[test]
    fn rotation_is_right_handed() {
        // +90 deg about X takes +Y to +Z.
        let p = Mat4::rot([1.0, 0.0, 0.0], 90.0).point([0.0, 1.0, 0.0]);
        assert!(p[2] > 0.99 && p[1].abs() < 1e-9, "{p:?}");
    }

    #[test]
    fn json_carries_kind_bounds_params_and_xform() {
        let j = Vol::fire(1.0, 2.0, 7, 0.25).to_json().unwrap();
        assert_eq!(j["kind"], "fire");
        assert_eq!(j["params"]["seed"], 7);
        assert_eq!(j["params"]["phase"], 0.25);
        assert_eq!(j["xform"].as_array().unwrap().len(), 16);
        assert!(j["bounds"]["min"].as_array().unwrap().iter().all(|v| v.as_f64().is_some()));
    }

    #[test]
    fn runaway_bounds_are_rejected() {
        let v = Vol::cloud(1.0, 0, 0.0).transformed(Mat4::scale(1.0e9, 1.0, 1.0));
        assert!(v.to_json().unwrap_err().contains("finite"));
    }
}
