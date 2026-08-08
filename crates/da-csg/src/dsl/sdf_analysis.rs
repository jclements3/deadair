//! Exact-analysis tools for the **analytic SDF export** (Stream B of the
//! BRL-CAD-gap backlog): deterministic volume + AABB reports (B1), the
//! cross-backend volume-parity harness (B2), and pairwise overlap detection
//! for placed props (B3) — the moral equivalents of BRL-CAD's `gqa` volume
//! and overlap reports, run against the osgedit `Node` JSON that
//! [`crate::dsl::compile_sdf`] emits.
//!
//! This module re-implements the *evaluation* of that JSON in `f64` (the
//! emitter writes `f64` numbers; osgedit renders in `f32`). It deliberately
//! supports exactly the node kinds the emitter produces today — `prim`
//! (sphere / box / cylinder / cone / torus / plane), `union`, `subtract`,
//! `intersect`, `translate`, `rotate`, `scale` — and returns a clear error
//! for anything else, so new emitter kinds (Stream A's lathe, ...) fail loud
//! here until distance + AABB rules are added.
//!
//! Determinism contract: **no RNG, no clocks.** Volume and overlap sampling
//! use the R2 low-discrepancy sequence (a Kronecker sequence on the
//! generalized golden ratio), so every report is a pure function of the node
//! tree and the sample count — same inputs, bit-identical output, forever.
//!
//! The distance formulas are line-for-line f64 ports of
//! `tools/osgedit/src/sdf.rs` (read-only reference); keep them in sync.

use glam::{DQuat, DVec3};
use serde_json::Value as Json;

// ---------------------------------------------------------------------------
// Aabb (f64, mirrors osgedit's rules)
// ---------------------------------------------------------------------------

/// Same "effectively infinite" sentinel osgedit uses for unbounded prims.
const BIG: f64 = 1.0e5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    fn new(min: DVec3, max: DVec3) -> Aabb {
        Aabb { min, max }
    }
    fn huge() -> Aabb {
        Aabb::new(DVec3::splat(-BIG), DVec3::splat(BIG))
    }
    fn union(self, o: Aabb) -> Aabb {
        Aabb::new(self.min.min(o.min), self.max.max(o.max))
    }
    /// Clamped so an empty intersection degenerates to a zero-extent box.
    fn intersect(self, o: Aabb) -> Aabb {
        let mn = self.min.max(o.min);
        let mx = self.max.min(o.max);
        Aabb::new(mn, mn.max(mx))
    }
    fn corners(&self) -> [DVec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            DVec3::new(a.x, a.y, a.z),
            DVec3::new(b.x, a.y, a.z),
            DVec3::new(a.x, b.y, a.z),
            DVec3::new(b.x, b.y, a.z),
            DVec3::new(a.x, a.y, b.z),
            DVec3::new(b.x, a.y, b.z),
            DVec3::new(a.x, b.y, b.z),
            DVec3::new(b.x, b.y, b.z),
        ]
    }
    fn from_points(pts: &[DVec3]) -> Aabb {
        let mut mn = DVec3::splat(f64::MAX);
        let mut mx = DVec3::splat(f64::MIN);
        for p in pts {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        Aabb::new(mn, mx)
    }
    pub fn is_finite(&self) -> bool {
        self.min.cmpgt(DVec3::splat(-BIG)).all() && self.max.cmplt(DVec3::splat(BIG)).all()
    }
    fn contains(&self, p: DVec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }
    pub fn extent(&self) -> DVec3 {
        (self.max - self.min).max(DVec3::ZERO)
    }
    pub fn volume(&self) -> f64 {
        let e = self.extent();
        e.x * e.y * e.z
    }
}

// ---------------------------------------------------------------------------
// Primitives (f64 ports of osgedit's Prim::dist / Prim::aabb)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Prim {
    Sphere { r: f64 },
    Box { half: DVec3, round: f64 },
    Cylinder { r: f64, h: f64 },
    Cone { r1: f64, r2: f64, h: f64 },
    Torus { major: f64, minor: f64 },
    Plane { n: DVec3, offset: f64 },
}

fn v2len(x: f64, y: f64) -> f64 {
    (x * x + y * y).sqrt()
}

impl Prim {
    fn dist(&self, p: DVec3) -> f64 {
        match *self {
            Prim::Sphere { r } => p.length() - r,
            Prim::Box { half, round } => {
                let b = half - DVec3::splat(round);
                let q = p.abs() - b;
                q.max(DVec3::ZERO).length() + q.max_element().min(0.0) - round
            }
            Prim::Cylinder { r, h } => {
                let hh = h * 0.5;
                let dx = v2len(p.x, p.z) - r;
                let dy = p.y.abs() - hh;
                dx.max(dy).min(0.0) + v2len(dx.max(0.0), dy.max(0.0))
            }
            Prim::Cone { r1, r2, h } => {
                let hh = h * 0.5;
                let q = (v2len(p.x, p.z), p.y);
                let k1 = (r2, hh);
                let k2 = (r2 - r1, 2.0 * hh);
                let ca = (q.0 - q.0.min(if q.1 < 0.0 { r1 } else { r2 }), q.1.abs() - hh);
                let d2 = |a: (f64, f64)| a.0 * a.0 + a.1 * a.1;
                let t = (((k1.0 - q.0) * k2.0 + (k1.1 - q.1) * k2.1) / d2(k2)).clamp(0.0, 1.0);
                let cb = (q.0 - k1.0 + k2.0 * t, q.1 - k1.1 + k2.1 * t);
                let s = if cb.0 < 0.0 && ca.1 < 0.0 { -1.0 } else { 1.0 };
                s * d2(ca).min(d2(cb)).sqrt()
            }
            Prim::Torus { major, minor } => v2len(v2len(p.x, p.z) - major, p.y) - minor,
            Prim::Plane { n, offset } => n.dot(p) - offset,
        }
    }

    fn aabb(&self) -> Aabb {
        match *self {
            Prim::Sphere { r } => Aabb::new(DVec3::splat(-r), DVec3::splat(r)),
            Prim::Box { half, .. } => Aabb::new(-half, half),
            Prim::Cylinder { r, h } => {
                let hh = h * 0.5;
                Aabb::new(DVec3::new(-r, -hh, -r), DVec3::new(r, hh, r))
            }
            Prim::Cone { r1, r2, h } => {
                let hh = h * 0.5;
                let r = r1.max(r2);
                Aabb::new(DVec3::new(-r, -hh, -r), DVec3::new(r, hh, r))
            }
            Prim::Torus { major, minor } => {
                let r = major + minor;
                Aabb::new(DVec3::new(-r, -minor, -r), DVec3::new(r, minor, r))
            }
            Prim::Plane { .. } => Aabb::huge(),
        }
    }
}

// ---------------------------------------------------------------------------
// Node tree
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum Kind {
    Prim(Prim),
    Union(Vec<Node>),
    Subtract(Vec<Node>),
    Intersect(Vec<Node>),
    Translate { v: DVec3, child: Box<Node> },
    Rotate { q: DQuat, child: Box<Node> },
    Scale { s: DVec3, child: Box<Node> },
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    /// Conservative bound of the solid region under this node, precomputed
    /// at parse time (mirrors osgedit `Node::aabb`). Used for containment
    /// pruning: a point outside `aabb` is provably outside the solid.
    aabb: Aabb,
}

impl Node {
    /// Full signed distance (bound), mirroring osgedit `Node::dist` in f64.
    fn dist(&self, p: DVec3) -> f64 {
        match &self.kind {
            Kind::Prim(prim) => prim.dist(p),
            Kind::Union(children) => {
                children.iter().map(|c| c.dist(p)).fold(f64::MAX, f64::min)
            }
            Kind::Subtract(children) => {
                let mut it = children.iter();
                let mut d = match it.next() {
                    Some(c) => c.dist(p),
                    None => return f64::MAX,
                };
                for c in it {
                    d = d.max(-c.dist(p));
                }
                d
            }
            Kind::Intersect(children) => {
                children.iter().map(|c| c.dist(p)).fold(f64::MIN, f64::max)
            }
            Kind::Translate { v, child } => child.dist(p - *v),
            Kind::Rotate { q, child } => child.dist(q.conjugate() * p),
            Kind::Scale { s, child } => child.dist(p / *s) * s.abs().min_element(),
        }
    }

    /// Exact inside test. Booleans over SDFs combine via min/max/negation,
    /// so the *sign* of the tree is a pure boolean function of the leaf
    /// signs — this evaluates that boolean directly, with AABB pruning
    /// (`p` outside a node's box can never be inside its solid). Transform
    /// nodes preserve sign (the Scale Lipschitz factor is positive), so
    /// `inside` agrees exactly with `dist(p) < 0.0`.
    fn inside(&self, p: DVec3) -> bool {
        if !self.aabb.contains(p) {
            return false;
        }
        match &self.kind {
            Kind::Prim(prim) => prim.dist(p) < 0.0,
            Kind::Union(children) => children.iter().any(|c| c.inside(p)),
            Kind::Subtract(children) => {
                let mut it = children.iter();
                match it.next() {
                    Some(base) => base.inside(p) && !it.any(|c| c.inside(p)),
                    None => false,
                }
            }
            Kind::Intersect(children) => children.iter().all(|c| c.inside(p)),
            Kind::Translate { v, child } => child.inside(p - *v),
            Kind::Rotate { q, child } => child.inside(q.conjugate() * p),
            Kind::Scale { s, child } => child.inside(p / *s),
        }
    }

    /// AABB rules mirror osgedit `Node::aabb`: union of children for union,
    /// first child for subtract, clamped intersection for intersect,
    /// transformed corners for rotate/scale (huge stays huge).
    fn compute_aabb(kind: &Kind) -> Aabb {
        let zero = Aabb::new(DVec3::ZERO, DVec3::ZERO);
        match kind {
            Kind::Prim(prim) => prim.aabb(),
            Kind::Union(children) => children
                .iter()
                .map(|c| c.aabb)
                .reduce(Aabb::union)
                .unwrap_or(zero),
            Kind::Subtract(children) => children.first().map(|c| c.aabb).unwrap_or(zero),
            Kind::Intersect(children) => children
                .iter()
                .map(|c| c.aabb)
                .reduce(Aabb::intersect)
                .unwrap_or(zero),
            Kind::Translate { v, child } => {
                Aabb::new(child.aabb.min + *v, child.aabb.max + *v)
            }
            Kind::Rotate { q, child } => {
                if !child.aabb.is_finite() {
                    return Aabb::huge();
                }
                let pts: Vec<DVec3> = child.aabb.corners().iter().map(|c| *q * *c).collect();
                Aabb::from_points(&pts)
            }
            Kind::Scale { s, child } => {
                if !child.aabb.is_finite() {
                    return Aabb::huge();
                }
                let pts: Vec<DVec3> = child.aabb.corners().iter().map(|c| *c * *s).collect();
                Aabb::from_points(&pts)
            }
        }
    }

    fn from_kind(kind: Kind) -> Node {
        let aabb = Node::compute_aabb(&kind);
        Node { kind, aabb }
    }
}

// ---------------------------------------------------------------------------
// JSON -> Node
// ---------------------------------------------------------------------------

fn jf(v: &Json, what: &str) -> Result<f64, String> {
    v.as_f64()
        .ok_or_else(|| format!("sdf_analysis: `{what}` is not a number"))
}

fn jf_or(v: &Json, key: &str, default: f64) -> Result<f64, String> {
    match v.get(key) {
        Some(x) => jf(x, key),
        None => Ok(default),
    }
}

fn jv3(v: &Json, key: &str) -> Result<DVec3, String> {
    let arr = v
        .get(key)
        .and_then(Json::as_array)
        .ok_or_else(|| format!("sdf_analysis: missing array `{key}`"))?;
    if arr.len() != 3 {
        return Err(format!("sdf_analysis: `{key}` must have 3 elements"));
    }
    Ok(DVec3::new(jf(&arr[0], key)?, jf(&arr[1], key)?, jf(&arr[2], key)?))
}

fn jkey<'a>(v: &'a Json, key: &str) -> Result<&'a Json, String> {
    v.get(key)
        .ok_or_else(|| format!("sdf_analysis: missing `{key}`"))
}

fn parse_prim(v: &Json) -> Result<Prim, String> {
    let shape = jkey(v, "shape")?
        .as_str()
        .ok_or("sdf_analysis: `shape` is not a string")?;
    match shape {
        "sphere" => Ok(Prim::Sphere { r: jf(jkey(v, "r")?, "r")? }),
        "box" => Ok(Prim::Box {
            half: jv3(v, "half")?,
            round: jf_or(v, "round", 0.0)?,
        }),
        "cylinder" => Ok(Prim::Cylinder {
            r: jf(jkey(v, "r")?, "r")?,
            h: jf(jkey(v, "h")?, "h")?,
        }),
        "cone" => Ok(Prim::Cone {
            r1: jf(jkey(v, "r1")?, "r1")?,
            r2: jf(jkey(v, "r2")?, "r2")?,
            h: jf(jkey(v, "h")?, "h")?,
        }),
        "torus" => Ok(Prim::Torus {
            major: jf(jkey(v, "major")?, "major")?,
            minor: jf(jkey(v, "minor")?, "minor")?,
        }),
        "plane" => {
            let n = jv3(v, "n")?;
            let len = n.length();
            if len == 0.0 {
                return Err("sdf_analysis: plane normal is zero".into());
            }
            Ok(Prim::Plane {
                n: n / len,
                offset: jf(jkey(v, "offset")?, "offset")?,
            })
        }
        other => Err(format!(
            "sdf_analysis: unsupported prim shape `{other}` — supported: \
             sphere box cylinder cone torus plane"
        )),
    }
}

fn parse_children(v: &Json) -> Result<Vec<Node>, String> {
    jkey(v, "children")?
        .as_array()
        .ok_or_else(|| "sdf_analysis: `children` is not an array".to_string())?
        .iter()
        .map(parse_node)
        .collect()
}

fn parse_child(v: &Json) -> Result<Box<Node>, String> {
    Ok(Box::new(parse_node(jkey(v, "child")?)?))
}

fn parse_node(v: &Json) -> Result<Node, String> {
    let kind = jkey(v, "kind")?
        .as_str()
        .ok_or("sdf_analysis: `kind` is not a string")?;
    let k = match kind {
        "prim" => Kind::Prim(parse_prim(jkey(v, "prim")?)?),
        "union" => Kind::Union(parse_children(v)?),
        "subtract" => Kind::Subtract(parse_children(v)?),
        "intersect" => Kind::Intersect(parse_children(v)?),
        "translate" => Kind::Translate { v: jv3(v, "v")?, child: parse_child(v)? },
        "rotate" => {
            let arr = jkey(v, "q")?
                .as_array()
                .ok_or("sdf_analysis: `q` is not an array")?;
            if arr.len() != 4 {
                return Err("sdf_analysis: `q` must have 4 elements".into());
            }
            let q = DQuat::from_xyzw(
                jf(&arr[0], "q")?,
                jf(&arr[1], "q")?,
                jf(&arr[2], "q")?,
                jf(&arr[3], "q")?,
            );
            Kind::Rotate { q: q.normalize(), child: parse_child(v)? }
        }
        "scale" => {
            let s = jv3(v, "s")?;
            if s.x == 0.0 || s.y == 0.0 || s.z == 0.0 {
                return Err("sdf_analysis: scale component is zero".into());
            }
            Kind::Scale { s, child: parse_child(v)? }
        }
        other => Err(format!(
            "sdf_analysis: unsupported node kind `{other}` — supported: prim, \
             union, subtract, intersect, translate, rotate, scale (add distance \
             + AABB rules here when the emitter grows a new kind)"
        ))?,
    };
    Ok(Node::from_kind(k))
}

// ---------------------------------------------------------------------------
// Field — the public handle
// ---------------------------------------------------------------------------

/// A parsed, analyzable SDF tree (f64 evaluation of the osgedit `Node` JSON).
#[derive(Debug)]
pub struct Field {
    root: Node,
}

impl Field {
    /// Parse a bare CSG node, or a whole `vimtool --sdf` export file
    /// (`{ "id": ..., "csg": <node> }`).
    pub fn from_json(v: &Json) -> Result<Field, String> {
        let node = v.get("csg").unwrap_or(v);
        Ok(Field { root: parse_node(node)? })
    }

    /// Signed distance bound at `p` (negative inside), matching osgedit's
    /// evaluator in f64.
    pub fn dist(&self, p: [f64; 3]) -> f64 {
        self.root.dist(DVec3::from_array(p))
    }

    /// Exact inside test (equivalent to `dist(p) < 0`, but AABB-pruned).
    pub fn inside(&self, p: [f64; 3]) -> bool {
        self.root.inside(DVec3::from_array(p))
    }

    /// Conservative axis-aligned bounding box of the solid.
    pub fn aabb(&self) -> Aabb {
        self.root.aabb
    }
}

/// Compile `.vim` source straight to a [`Field`] (the analytic backend),
/// in osgedit's Y-up meters — ground at y = 0.
pub fn compile_sdf_field(src: &str) -> Result<Field, String> {
    Field::from_json(&crate::dsl::compile_sdf(src)?)
}

/// Compile `.vim` source to a [`Field`] placed the way `da-param` places a
/// `VimProp` mesh: uniform `scale`, then yaw about +Y (`yaw_deg`, right-
/// handed), then translate to `pos` (prop origin at ground level, Y-up).
pub fn compile_sdf_placed(
    src: &str,
    pos: [f64; 3],
    yaw_deg: f64,
    scale: f64,
) -> Result<Field, String> {
    if scale <= 0.0 {
        return Err("sdf_analysis: prop scale must be positive".into());
    }
    let node = crate::dsl::compile_sdf(src)?;
    let half = yaw_deg.to_radians() * 0.5;
    let placed = serde_json::json!({
        "kind": "translate", "v": pos,
        "child": {
            "kind": "rotate", "q": [0.0, half.sin(), 0.0, half.cos()],
            "child": {
                "kind": "scale", "s": [scale, scale, scale],
                "child": node,
            },
        },
    });
    Field::from_json(&placed)
}

// ---------------------------------------------------------------------------
// R2 low-discrepancy sequence (deterministic sampling; no RNG anywhere)
// ---------------------------------------------------------------------------

/// The 3D R2 (Kronecker) sequence on the plastic-like constant: the unique
/// real root of x^4 = x + 1 generalizes the golden ratio to 3D. Excellent
/// star discrepancy, trivially deterministic, no state beyond a counter.
struct R2 {
    n: u64,
}

impl R2 {
    // g is the root of g^4 = g + 1 (the "harmonious number" for d = 3).
    const G: f64 = 1.220744084605759475361686349108831;
    const A1: f64 = 1.0 / Self::G;
    const A2: f64 = 1.0 / (Self::G * Self::G);
    const A3: f64 = 1.0 / (Self::G * Self::G * Self::G);

    fn new() -> R2 {
        R2 { n: 0 }
    }

    fn next(&mut self) -> DVec3 {
        self.n += 1;
        let k = self.n as f64;
        let fr = |x: f64| x - x.floor();
        DVec3::new(
            fr(0.5 + Self::A1 * k),
            fr(0.5 + Self::A2 * k),
            fr(0.5 + Self::A3 * k),
        )
    }
}

// ---------------------------------------------------------------------------
// B1 — volume + AABB report
// ---------------------------------------------------------------------------

/// Deterministic volume estimate for a [`Field`].
#[derive(Clone, Debug)]
pub struct VolumeReport {
    /// Conservative AABB of the solid (meters, the tree's own frame).
    pub aabb: Aabb,
    /// Volume of that AABB.
    pub box_volume: f64,
    /// Estimated solid volume (m^3): inside fraction x box volume.
    pub volume: f64,
    /// 95% confidence half-width under the i.i.d. binomial model
    /// (1.96 sqrt(p(1-p)/n) x box volume). The R2 sequence is *low-
    /// discrepancy*, so its true error is typically far below this bound —
    /// treat `ci95` as conservative.
    pub ci95: f64,
    /// Points sampled / points found inside.
    pub samples: u64,
    pub inside: u64,
}

/// Estimate the volume of `field` with `samples` R2 points over its AABB.
/// Deterministic: same tree + same count = bit-identical report. Errors if
/// the tree is unbounded (a bare plane not clamped by an intersect).
pub fn volume_report(field: &Field, samples: u64) -> Result<VolumeReport, String> {
    if samples == 0 {
        return Err("sdf_analysis: volume_report needs at least 1 sample".into());
    }
    let aabb = field.aabb();
    if !aabb.is_finite() {
        return Err(
            "sdf_analysis: tree has an unbounded AABB (bare half-space?) — \
             cannot sample a volume"
                .into(),
        );
    }
    let ext = aabb.extent();
    let box_volume = aabb.volume();
    let mut inside = 0u64;
    if box_volume > 0.0 {
        let mut seq = R2::new();
        for _ in 0..samples {
            let p = aabb.min + ext * seq.next();
            if field.root.inside(p) {
                inside += 1;
            }
        }
    }
    let n = samples as f64;
    let p_hat = inside as f64 / n;
    Ok(VolumeReport {
        aabb,
        box_volume,
        volume: p_hat * box_volume,
        ci95: 1.96 * (p_hat * (1.0 - p_hat) / n).sqrt() * box_volume,
        samples,
        inside,
    })
}

// ---------------------------------------------------------------------------
// B3 — pairwise overlap detection
// ---------------------------------------------------------------------------

/// A named, world-placed field (see [`compile_sdf_placed`]).
#[derive(Debug)]
pub struct PlacedProp {
    pub name: String,
    pub field: Field,
}

/// One detected pairwise interpenetration.
#[derive(Clone, Debug)]
pub struct OverlapWarning {
    /// Names of the two overlapping props (input order preserved).
    pub a: String,
    pub b: String,
    /// Estimated shared volume (m^3): hit fraction x intersection-box volume.
    pub overlap_volume: f64,
    /// Deepest observed mutual penetration (m): max over hit samples of
    /// min(-dist_a, -dist_b). Approximate where the trees contain non-
    /// uniform scale (distance bounds, exact signs).
    pub penetration: f64,
    /// The sample point (world, Y-up) at the deepest penetration.
    pub at: [f64; 3],
    /// Sample statistics inside the AABB-intersection box.
    pub hits: u64,
    pub samples: u64,
}

/// Pairwise overlap check over placed props: AABB prefilter, then R2 sign
/// sampling of each AABB-intersection box (a point inside *both* fields is
/// an overlap). Deterministic; warnings come out in input pair order
/// (i < j). Props with an unbounded AABB never prefilter-out (the huge box
/// intersects everything), so they are still checked by sampling.
pub fn overlap_report(props: &[PlacedProp], samples_per_pair: u64) -> Vec<OverlapWarning> {
    let mut warnings = Vec::new();
    for i in 0..props.len() {
        for j in (i + 1)..props.len() {
            let (a, b) = (&props[i], &props[j]);
            let ibox = a.field.aabb().intersect(b.field.aabb());
            let ext = ibox.extent();
            if ext.x <= 0.0 || ext.y <= 0.0 || ext.z <= 0.0 {
                continue; // AABBs disjoint (or merely touching): no interior overlap.
            }
            let mut seq = R2::new();
            let mut hits = 0u64;
            let mut penetration = 0.0f64;
            let mut at = [0.0f64; 3];
            for _ in 0..samples_per_pair {
                let p = ibox.min + ext * seq.next();
                if a.field.root.inside(p) && b.field.root.inside(p) {
                    hits += 1;
                    let depth = (-a.field.root.dist(p)).min(-b.field.root.dist(p));
                    if depth > penetration {
                        penetration = depth;
                        at = p.to_array();
                    }
                }
            }
            if hits > 0 {
                warnings.push(OverlapWarning {
                    a: a.name.clone(),
                    b: b.name.clone(),
                    overlap_volume: hits as f64 / samples_per_pair as f64 * ibox.volume(),
                    penetration,
                    at,
                    hits,
                    samples: samples_per_pair,
                });
            }
        }
    }
    warnings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn field(src: &str) -> Field {
        compile_sdf_field(src).expect("compiles to SDF")
    }

    const N: u64 = 100_000;

    // --- B1: volume + AABB ------------------------------------------------

    #[test]
    fn sphere_volume_and_aabb() {
        let f = field("model sphere(r = 1)");
        let r = volume_report(&f, N).unwrap();
        let want = 4.0 / 3.0 * std::f64::consts::PI;
        assert!(
            (r.volume - want).abs() < 0.01 * want,
            "sphere volume {} vs {want}",
            r.volume
        );
        assert!(r.ci95 > 0.0 && r.ci95 < 0.02 * want, "ci95 {}", r.ci95);
        // AABB is the unit cube around the origin (rotated at the root, but
        // a sphere's corners stay put within f64 rounding).
        assert!((r.aabb.min + DVec3::ONE).length() < 1e-9, "{:?}", r.aabb);
        assert!((r.aabb.max - DVec3::ONE).length() < 1e-9, "{:?}", r.aabb);
        assert!((r.box_volume - 8.0).abs() < 1e-9);
    }

    #[test]
    fn cube_volume_is_near_exact() {
        // Axis-aligned box: every sample decision is exact, so the only
        // error is the sequence's discrepancy on a [0,1]-aligned event —
        // and the box fills its own AABB, so the estimate is *exactly* 8.
        let f = field("model cube(s = 2)");
        let r = volume_report(&f, N).unwrap();
        assert_eq!(r.inside, r.samples, "cube fills its own AABB");
        assert!((r.volume - 8.0).abs() < 1e-12);
        assert_eq!(r.ci95, 0.0);
    }

    #[test]
    fn boolean_volume_tube() {
        // tube = cylinder(R) - cylinder(r): exact volume pi (R^2 - r^2) h.
        let f = field("model tube(2, 1, 4)");
        let r = volume_report(&f, N).unwrap();
        let want = std::f64::consts::PI * (4.0 - 1.0) * 4.0;
        assert!(
            (r.volume - want).abs() < 0.01 * want,
            "tube volume {} vs {want}",
            r.volume
        );
    }

    #[test]
    fn transforms_preserve_volume() {
        // Rotated + translated + uniformly scaled cylinder: volume scales by s^3.
        let plain = volume_report(&field("model cylinder(r = 1, h = 2)"), N).unwrap();
        let moved = volume_report(
            &field("model cylinder(r = 1, h = 2).rotatex(30).rotatez(55).move(3, -2, 7).scale(1.5)"),
            N,
        )
        .unwrap();
        let want = plain.volume * 1.5f64.powi(3);
        assert!(
            (moved.volume - want).abs() < 0.02 * want,
            "scaled volume {} vs {want}",
            moved.volume
        );
    }

    #[test]
    fn report_is_deterministic() {
        let a = volume_report(&field("model sphere(1) + cube(1).move(2, 0, 0)"), 10_000).unwrap();
        let b = volume_report(&field("model sphere(1) + cube(1).move(2, 0, 0)"), 10_000).unwrap();
        assert_eq!(a.volume.to_bits(), b.volume.to_bits());
        assert_eq!(a.inside, b.inside);
        assert_eq!(a.ci95.to_bits(), b.ci95.to_bits());
    }

    #[test]
    fn inside_agrees_with_dist_sign() {
        // The pruned boolean evaluator must agree with the full distance
        // evaluator everywhere — sample a mixed CSG tree and compare.
        let f = field("model tube(2, 1, 4) + sphere(1).move(0, 0, 3) - cube(1)");
        let aabb = f.aabb();
        let ext = aabb.extent();
        let mut seq = R2::new();
        for _ in 0..20_000 {
            let p = aabb.min + ext * seq.next();
            assert_eq!(f.root.inside(p), f.root.dist(p) < 0.0, "at {p}");
        }
    }

    #[test]
    fn unsupported_kind_is_a_clear_error() {
        let v = serde_json::json!({ "kind": "smooth_union", "k": 0.5, "children": [] });
        let err = Field::from_json(&v).unwrap_err();
        assert!(err.contains("unsupported node kind `smooth_union`"), "{err}");
    }

    #[test]
    fn unbounded_tree_refuses_volume() {
        let v = serde_json::json!({
            "kind": "prim", "prim": { "shape": "plane", "n": [0.0, 1.0, 0.0], "offset": 0.0 }
        });
        let f = Field::from_json(&v).unwrap();
        let err = volume_report(&f, 100).unwrap_err();
        assert!(err.contains("unbounded"), "{err}");
    }

    #[test]
    fn wrapped_export_file_parses() {
        // vimtool --sdf writes { id, csg } — Field accepts the wrapper too.
        let node = crate::dsl::compile_sdf("model cube(1)").unwrap();
        let file = serde_json::json!({ "id": "cube", "csg": node });
        let f = Field::from_json(&file).unwrap();
        // `cube(1)` is centered on the origin in both frames.
        assert!(f.inside([0.0, 0.0, 0.0]));
        assert!(!f.inside([0.0, 0.6, 0.0]));
    }

    // --- B3: overlap ---------------------------------------------------------

    fn placed(name: &str, src: &str, pos: [f64; 3], yaw: f64, scale: f64) -> PlacedProp {
        PlacedProp {
            name: name.into(),
            field: compile_sdf_placed(src, pos, yaw, scale).unwrap(),
        }
    }

    #[test]
    fn overlapping_cubes_warn_disjoint_do_not() {
        let props = vec![
            placed("a", "model cube(2).move(0, 0, 1)", [0.0, 0.0, 0.0], 0.0, 1.0),
            placed("b", "model cube(2).move(0, 0, 1)", [1.0, 0.0, 1.0], 0.0, 1.0),
            placed("c", "model cube(2).move(0, 0, 1)", [10.0, 0.0, 0.0], 0.0, 1.0),
        ];
        let warnings = overlap_report(&props, 8_192);
        assert_eq!(warnings.len(), 1, "only a-b overlap: {warnings:?}");
        let w = &warnings[0];
        assert_eq!((w.a.as_str(), w.b.as_str()), ("a", "b"));
        // Shared region is a 1 x 2 x 1 box = 2 m^3; both cubes fill the
        // intersection box, so the estimate is exact.
        assert!((w.overlap_volume - 2.0).abs() < 1e-9, "{}", w.overlap_volume);
        // Deepest mutual penetration of two unit-offset 2m cubes is 0.5 m
        // (sampled: slightly under).
        assert!(w.penetration > 0.35 && w.penetration <= 0.5 + 1e-9, "{}", w.penetration);
    }

    #[test]
    fn aabb_touch_without_body_overlap_is_clean() {
        // Two spheres whose AABBs overlap at the corners but whose bodies
        // don't: the sign sampling must reject the false positive.
        let props = vec![
            placed("a", "model sphere(1).move(0, 0, 1)", [0.0, 0.0, 0.0], 0.0, 1.0),
            placed("b", "model sphere(1).move(0, 0, 1)", [1.9, 0.0, 1.9], 0.0, 1.0),
        ];
        assert!(overlap_report(&props, 8_192).is_empty());
    }

    #[test]
    fn yaw_moves_a_prop_into_collision() {
        // A long bar at the origin and a small cube parked beside its tip:
        // clear at yaw 0, overlapping once the bar yaws onto it.
        let bar = "model box(8, 1, 1).move(0, 0, 0.5)";
        let cube = "model cube(1).move(0, 0, 0.5)";
        let clear = vec![
            placed("bar", bar, [0.0, 0.0, 0.0], 0.0, 1.0),
            placed("cube", cube, [0.0, 0.0, 3.0], 0.0, 1.0),
        ];
        assert!(overlap_report(&clear, 8_192).is_empty());
        let hit = vec![
            placed("bar", bar, [0.0, 0.0, 0.0], 90.0, 1.0),
            placed("cube", cube, [0.0, 0.0, 3.0], 0.0, 1.0),
        ];
        let warnings = overlap_report(&hit, 8_192);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    #[test]
    fn overlap_report_is_deterministic() {
        let mk = || {
            vec![
                placed("a", "model sphere(2)", [0.0, 0.0, 0.0], 0.0, 1.0),
                placed("b", "model sphere(2)", [1.0, 0.5, 0.0], 30.0, 1.0),
            ]
        };
        let w1 = overlap_report(&mk(), 4_096);
        let w2 = overlap_report(&mk(), 4_096);
        assert_eq!(w1.len(), 1);
        assert_eq!(w1[0].overlap_volume.to_bits(), w2[0].overlap_volume.to_bits());
        assert_eq!(w1[0].penetration.to_bits(), w2[0].penetration.to_bits());
        assert_eq!(w1[0].hits, w2[0].hits);
    }
}
