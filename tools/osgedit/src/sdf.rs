//! SDF primitives + CSG op-tree, ported from the vali `sdf-core` crate so the
//! conference-demo model JSONs load unchanged. Y is up, inside < 0.

use serde::{Deserialize, Serialize};

pub const TAU: f32 = std::f32::consts::TAU;

// ---------------------------------------------------------------------------
// Vec3
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl Vec3 {
    pub const ZERO: Vec3 = v3(0.0, 0.0, 0.0);
    pub const Y: Vec3 = v3(0.0, 1.0, 0.0);

    pub fn from_arr(a: [f32; 3]) -> Vec3 {
        v3(a[0], a[1], a[2])
    }
    pub fn dot(self, o: Vec3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: Vec3) -> Vec3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalize(self) -> Vec3 {
        let l = self.length();
        if l > 0.0 {
            self * (1.0 / l)
        } else {
            self
        }
    }
    pub fn abs(self) -> Vec3 {
        v3(self.x.abs(), self.y.abs(), self.z.abs())
    }
    pub fn max(self, o: Vec3) -> Vec3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }
    pub fn min(self, o: Vec3) -> Vec3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }
    pub fn max_comp(self) -> f32 {
        self.x.max(self.y).max(self.z)
    }
    pub fn min_comp(self) -> f32 {
        self.x.min(self.y).min(self.z)
    }
    pub fn mul(self, o: Vec3) -> Vec3 {
        v3(self.x * o.x, self.y * o.y, self.z * o.z)
    }
    pub fn div(self, o: Vec3) -> Vec3 {
        v3(self.x / o.x, self.y / o.y, self.z / o.z)
    }
    pub fn lerp(self, o: Vec3, t: f32) -> Vec3 {
        self + (o - self) * t
    }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl std::ops::Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl std::ops::Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f32) -> Vec3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}
impl std::ops::Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        v3(-self.x, -self.y, -self.z)
    }
}

fn v2len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

// ---------------------------------------------------------------------------
// Quaternion (x, y, z, w)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat(pub [f32; 4]);

impl Quat {
    pub fn from_axis_angle(axis: Vec3, angle: f32) -> Quat {
        let a = axis.normalize();
        let (s, c) = (angle * 0.5).sin_cos();
        Quat([a.x * s, a.y * s, a.z * s, c])
    }
    pub fn conj(self) -> Quat {
        let [x, y, z, w] = self.0;
        Quat([-x, -y, -z, w])
    }
    pub fn rotate(self, p: Vec3) -> Vec3 {
        let [x, y, z, w] = self.0;
        let u = v3(x, y, z);
        let t = u.cross(p) * 2.0;
        p + t * w + u.cross(t)
    }
}

// ---------------------------------------------------------------------------
// Aabb
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

pub const BIG: f32 = 1.0e5;

impl Aabb {
    pub fn new(min: Vec3, max: Vec3) -> Aabb {
        Aabb { min, max }
    }
    pub fn huge() -> Aabb {
        Aabb::new(v3(-BIG, -BIG, -BIG), v3(BIG, BIG, BIG))
    }
    pub fn union(self, o: Aabb) -> Aabb {
        Aabb::new(self.min.min(o.min), self.max.max(o.max))
    }
    pub fn intersect(self, o: Aabb) -> Aabb {
        let mn = self.min.max(o.min);
        let mx = self.max.min(o.max);
        Aabb::new(mn, mn.max(mx))
    }
    pub fn expand(self, m: f32) -> Aabb {
        Aabb::new(self.min - v3(m, m, m), self.max + v3(m, m, m))
    }
    pub fn corners(&self) -> [Vec3; 8] {
        let (a, b) = (self.min, self.max);
        [
            v3(a.x, a.y, a.z),
            v3(b.x, a.y, a.z),
            v3(a.x, b.y, a.z),
            v3(b.x, b.y, a.z),
            v3(a.x, a.y, b.z),
            v3(b.x, a.y, b.z),
            v3(a.x, b.y, b.z),
            v3(b.x, b.y, b.z),
        ]
    }
    pub fn from_points(pts: &[Vec3]) -> Aabb {
        let mut mn = v3(f32::MAX, f32::MAX, f32::MAX);
        let mut mx = v3(f32::MIN, f32::MIN, f32::MIN);
        for p in pts {
            mn = mn.min(*p);
            mx = mx.max(*p);
        }
        Aabb::new(mn, mx)
    }
    pub fn is_finite(&self) -> bool {
        self.min.x > -BIG
            && self.max.x < BIG
            && self.min.y > -BIG
            && self.max.y < BIG
            && self.min.z > -BIG
            && self.max.z < BIG
    }
    /// Distance from a point to the box (0 inside) — a valid lower bound on
    /// the distance to anything contained in the box.
    pub fn distance(&self, p: Vec3) -> f32 {
        let d = (self.min - p).max(p - self.max).max(Vec3::ZERO);
        d.length()
    }
    /// Ray/slab test; returns (t_enter, t_exit) if the ray touches the box.
    pub fn ray_hit(&self, ro: Vec3, inv_rd: Vec3, t_max: f32) -> Option<(f32, f32)> {
        let t1 = (self.min - ro).mul(inv_rd);
        let t2 = (self.max - ro).mul(inv_rd);
        let tn = t1.min(t2).max_comp();
        let tf = t1.max(t2).min_comp();
        if tf >= tn.max(0.0) && tn <= t_max {
            Some((tn.max(0.0), tf.min(t_max)))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Prim {
    Sphere {
        r: f32,
    },
    Box {
        half: [f32; 3],
        #[serde(default)]
        round: f32,
    },
    Cylinder {
        r: f32,
        h: f32,
    },
    Cone {
        r1: f32,
        r2: f32,
        h: f32,
    },
    Torus {
        major: f32,
        minor: f32,
    },
    Capsule {
        a: [f32; 3],
        b: [f32; 3],
        r: f32,
    },
    Plane {
        n: [f32; 3],
        offset: f32,
    },
    /// Body of revolution: a closed 2D profile of `(radius, z)` points spun
    /// about the **Z** axis. Distance is the exact 2D polyline SDF evaluated
    /// in (radius, z) space — the nearest point on a surface of revolution
    /// always lies in the query's own meridian half-plane, so line-segment
    /// profiles are exact at any zoom (and infinitely round in the revolve
    /// direction). Profiles that dip into r < 0 sweep their mirror too.
    Lathe {
        pts: Vec<[f32; 2]>,
    },
    /// Prism: a closed 2D cross-section of `(x, y)` points extruded along
    /// **Z** over `[-h/2, +h/2]` (exact 2D polygon SDF ∩ slab). A nonzero
    /// `twist_deg` rotates the section linearly with z (domain rotation);
    /// twisted distances are conservatively scaled so marching stays safe.
    Extrude {
        pts: Vec<[f32; 2]>,
        h: f32,
        #[serde(default)]
        twist_deg: f32,
    },
}

/// Exact signed distance from `(qx, qy)` to a closed 2D polygon (negative
/// inside, even-odd fill, winding-agnostic) — iq's polygon SDF. The loop is
/// implicitly closed from the last point back to the first. With `skip_axis`,
/// edges lying on the revolve axis (both endpoints at |x| < 1e-9) still count
/// for the inside/outside parity but not for the distance — they are the seam
/// that closes a lathe profile, not real surface (mirrors the mesh revolve,
/// which drops those segments too).
fn polygon_sdf_impl(pts: &[[f32; 2]], qx: f32, qy: f32, skip_axis: bool) -> f32 {
    let n = pts.len();
    if n < 2 {
        return f32::MAX;
    }
    let mut d = f32::MAX;
    let mut s = 1.0f32;
    let mut j = n - 1;
    for i in 0..n {
        let (ex, ey) = (pts[j][0] - pts[i][0], pts[j][1] - pts[i][1]);
        let (wx, wy) = (qx - pts[i][0], qy - pts[i][1]);
        if !(skip_axis && pts[i][0].abs() < 1e-9 && pts[j][0].abs() < 1e-9) {
            let ee = ex * ex + ey * ey;
            let t = if ee > 0.0 { ((wx * ex + wy * ey) / ee).clamp(0.0, 1.0) } else { 0.0 };
            let (bx, by) = (wx - ex * t, wy - ey * t);
            d = d.min(bx * bx + by * by);
        }
        // Winding parity: count edge crossings of the +x ray from q.
        let c0 = qy >= pts[i][1];
        let c1 = qy < pts[j][1];
        let c2 = ex * wy > ey * wx;
        if (c0 && c1 && c2) || (!c0 && !c1 && !c2) {
            s = -s;
        }
        j = i;
    }
    if d == f32::MAX {
        return f32::MAX; // every edge skipped: a degenerate on-axis profile
    }
    s * d.sqrt()
}

fn polygon_sdf(pts: &[[f32; 2]], qx: f32, qy: f32) -> f32 {
    polygon_sdf_impl(pts, qx, qy, false)
}

impl Prim {
    pub fn dist(&self, p: Vec3) -> f32 {
        match *self {
            Prim::Sphere { r } => p.length() - r,
            Prim::Box { half, round } => {
                let b = Vec3::from_arr(half) - v3(round, round, round);
                let q = p.abs() - b;
                q.max(Vec3::ZERO).length() + q.max_comp().min(0.0) - round
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
                let d2 = |a: (f32, f32)| a.0 * a.0 + a.1 * a.1;
                let t = (((k1.0 - q.0) * k2.0 + (k1.1 - q.1) * k2.1) / d2(k2)).clamp(0.0, 1.0);
                let cb = (q.0 - k1.0 + k2.0 * t, q.1 - k1.1 + k2.1 * t);
                let s = if cb.0 < 0.0 && ca.1 < 0.0 { -1.0 } else { 1.0 };
                s * d2(ca).min(d2(cb)).sqrt()
            }
            Prim::Torus { major, minor } => v2len(v2len(p.x, p.z) - major, p.y) - minor,
            Prim::Capsule { a, b, r } => {
                let (a, b) = (Vec3::from_arr(a), Vec3::from_arr(b));
                let pa = p - a;
                let ba = b - a;
                let t = (pa.dot(ba) / ba.dot(ba)).clamp(0.0, 1.0);
                (pa - ba * t).length() - r
            }
            Prim::Plane { n, offset } => Vec3::from_arr(n).normalize().dot(p) - offset,
            Prim::Lathe { ref pts } => {
                let rad = v2len(p.x, p.y);
                let d = polygon_sdf_impl(pts, rad, p.z, true);
                if pts.iter().any(|a| a[0] < 0.0) {
                    // Negative-radius profile parts sweep their mirror image;
                    // the revolved solid is the union of both (min).
                    d.min(polygon_sdf_impl(pts, -rad, p.z, true))
                } else {
                    d
                }
            }
            Prim::Extrude { ref pts, h, twist_deg } => {
                let hh = h * 0.5;
                // Twist rate in radians per unit z (0 = plain prism).
                let k = if twist_deg != 0.0 && h > 0.0 { twist_deg.to_radians() / h } else { 0.0 };
                let d2 = if k != 0.0 {
                    // Section at height z is the base section rotated by
                    // twist_deg * (z/h + 1/2); undo it in the domain.
                    let a = -k * (p.z + hh);
                    let (s, c) = a.sin_cos();
                    polygon_sdf(pts, p.x * c - p.y * s, p.x * s + p.y * c)
                } else {
                    polygon_sdf(pts, p.x, p.y)
                };
                let dz = p.z.abs() - hh;
                let d = d2.max(dz).min(0.0) + v2len(d2.max(0.0), dz.max(0.0));
                if k != 0.0 {
                    // Conservative step under the twist: any surface point s
                    // satisfies |p−s|·(1 + |k|(r + |p−s|)) ≥ d (the untwisted
                    // field is 1-Lipschitz and the domain map's Jacobian is
                    // bounded by 1 + |k|·r), so d / (1 + |k|(r + d)) is safe.
                    d / (1.0 + k.abs() * (v2len(p.x, p.y) + d.max(0.0)))
                } else {
                    d
                }
            }
        }
    }

    pub fn aabb(&self) -> Aabb {
        match *self {
            Prim::Sphere { r } => Aabb::new(v3(-r, -r, -r), v3(r, r, r)),
            Prim::Box { half, .. } => {
                let h = Vec3::from_arr(half);
                Aabb::new(-h, h)
            }
            Prim::Cylinder { r, h } => {
                let hh = h * 0.5;
                Aabb::new(v3(-r, -hh, -r), v3(r, hh, r))
            }
            Prim::Cone { r1, r2, h } => {
                let hh = h * 0.5;
                let r = r1.max(r2);
                Aabb::new(v3(-r, -hh, -r), v3(r, hh, r))
            }
            Prim::Torus { major, minor } => {
                let r = major + minor;
                Aabb::new(v3(-r, -minor, -r), v3(r, minor, r))
            }
            Prim::Capsule { a, b, r } => {
                let (a, b) = (Vec3::from_arr(a), Vec3::from_arr(b));
                Aabb::from_points(&[a, b]).expand(r)
            }
            Prim::Plane { .. } => Aabb::huge(),
            Prim::Lathe { ref pts } => {
                if pts.is_empty() {
                    return Aabb::new(Vec3::ZERO, Vec3::ZERO);
                }
                let r = pts.iter().map(|a| a[0].abs()).fold(0.0f32, f32::max);
                let zmin = pts.iter().map(|a| a[1]).fold(f32::MAX, f32::min);
                let zmax = pts.iter().map(|a| a[1]).fold(f32::MIN, f32::max);
                Aabb::new(v3(-r, -r, zmin), v3(r, r, zmax))
            }
            Prim::Extrude { ref pts, h, twist_deg } => {
                if pts.is_empty() {
                    return Aabb::new(Vec3::ZERO, Vec3::ZERO);
                }
                let hh = h * 0.5;
                if twist_deg != 0.0 {
                    // The section sweeps a circle as it rotates.
                    let r = pts.iter().map(|a| v2len(a[0], a[1])).fold(0.0f32, f32::max);
                    Aabb::new(v3(-r, -r, -hh), v3(r, r, hh))
                } else {
                    let xmin = pts.iter().map(|a| a[0]).fold(f32::MAX, f32::min);
                    let xmax = pts.iter().map(|a| a[0]).fold(f32::MIN, f32::max);
                    let ymin = pts.iter().map(|a| a[1]).fold(f32::MAX, f32::min);
                    let ymax = pts.iter().map(|a| a[1]).fold(f32::MIN, f32::max);
                    Aabb::new(v3(xmin, ymin, -hh), v3(xmax, ymax, hh))
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Op-tree
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    Prim { prim: Prim },
    Union { children: Vec<Node> },
    Subtract { children: Vec<Node> },
    Intersect { children: Vec<Node> },
    SmoothUnion { k: f32, children: Vec<Node> },
    SmoothSubtract { k: f32, children: Vec<Node> },
    SmoothIntersect { k: f32, children: Vec<Node> },
    Shell { thickness: f32, child: Box<Node> },
    Translate { v: [f32; 3], child: Box<Node> },
    Rotate { q: [f32; 4], child: Box<Node> },
    Scale { s: [f32; 3], child: Box<Node> },
    RadialRepeat { n: u32, child: Box<Node> },
    GridRepeat { cell: [f32; 3], count: [u32; 3], child: Box<Node> },
    Mirror { axis: Axis, child: Box<Node> },
}

pub fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b + (a - b) * h - k * h * (1.0 - h)
}
pub fn smax(a: f32, b: f32, k: f32) -> f32 {
    -smin(-a, -b, k)
}
pub fn ssub(base: f32, tool: f32, k: f32) -> f32 {
    smax(base, -tool, k)
}

fn grid_fold(p: f32, cell: f32, count: u32) -> f32 {
    if count <= 1 || cell == 0.0 {
        return p;
    }
    let m = (count - 1) as f32 * 0.5;
    let idx = (p / cell + m).round().clamp(0.0, (count - 1) as f32);
    p - cell * (idx - m)
}

impl Node {
    pub fn dist(&self, p: Vec3) -> f32 {
        match self {
            Node::Prim { prim } => prim.dist(p),
            Node::Union { children } => {
                children.iter().map(|c| c.dist(p)).fold(f32::MAX, f32::min)
            }
            Node::Subtract { children } => {
                let mut it = children.iter();
                let mut d = match it.next() {
                    Some(c) => c.dist(p),
                    None => return f32::MAX,
                };
                for c in it {
                    d = d.max(-c.dist(p));
                }
                d
            }
            Node::Intersect { children } => {
                children.iter().map(|c| c.dist(p)).fold(f32::MIN, f32::max)
            }
            Node::SmoothUnion { k, children } => {
                let mut d = f32::MAX;
                for c in children {
                    d = smin(d, c.dist(p), *k);
                }
                d
            }
            Node::SmoothSubtract { k, children } => {
                let mut it = children.iter();
                let mut d = match it.next() {
                    Some(c) => c.dist(p),
                    None => return f32::MAX,
                };
                for c in it {
                    d = ssub(d, c.dist(p), *k);
                }
                d
            }
            Node::SmoothIntersect { k, children } => {
                let mut d = f32::MIN;
                for c in children {
                    d = smax(d, c.dist(p), *k);
                }
                d
            }
            Node::Shell { thickness, child } => child.dist(p).abs() - thickness,
            Node::Translate { v, child } => child.dist(p - Vec3::from_arr(*v)),
            Node::Rotate { q, child } => child.dist(Quat(*q).conj().rotate(p)),
            Node::Scale { s, child } => {
                // Non-uniform scale is not distance-preserving, so no single
                // factor can turn the child's distance back into an exact
                // world distance. Multiplying by the *smallest* |s_i| is the
                // largest factor that is still a valid Lipschitz bound in
                // every direction: a world step of length L moves the child-
                // space point by at most L / min|s_i|, so
                //     |dist(p)| = |d_child| * min|s_i|  <=  true distance,
                // with the sign preserved. Sphere tracing therefore never
                // oversteps the surface, but the bound is conservative by up
                // to ratio = max|s_i| / min|s_i| in the worst direction (a
                // ray approaching along the most-stretched axis), so the
                // near-surface march count grows ~linearly with that ratio —
                // measured and bounded by `scale_aniso_march_count_is_bounded`
                // in this file's tests. A tighter per-direction factor would
                // need the child's local gradient, which a black-box `dist`
                // does not expose; per-axis factors alone cannot help because
                // the closest-feature direction is unknown at bound time.
                let s = Vec3::from_arr(*s);
                child.dist(p.div(s)) * s.abs().min_comp()
            }
            Node::RadialRepeat { n, child } => {
                if *n <= 1 {
                    return child.dist(p);
                }
                let sector = TAU / *n as f32;
                let ang = p.z.atan2(p.x);
                let a = (ang + 0.5 * sector).rem_euclid(sector) - 0.5 * sector;
                let r = v2len(p.x, p.z);
                child.dist(v3(r * a.cos(), p.y, r * a.sin()))
            }
            Node::GridRepeat { cell, count, child } => child.dist(v3(
                grid_fold(p.x, cell[0], count[0]),
                grid_fold(p.y, cell[1], count[1]),
                grid_fold(p.z, cell[2], count[2]),
            )),
            Node::Mirror { axis, child } => {
                let q = match axis {
                    Axis::X => v3(p.x.abs(), p.y, p.z),
                    Axis::Y => v3(p.x, p.y.abs(), p.z),
                    Axis::Z => v3(p.x, p.y, p.z.abs()),
                };
                child.dist(q)
            }
        }
    }

    pub fn aabb(&self) -> Aabb {
        match self {
            Node::Prim { prim } => prim.aabb(),
            Node::Union { children } => children
                .iter()
                .map(|c| c.aabb())
                .reduce(Aabb::union)
                .unwrap_or(Aabb::new(Vec3::ZERO, Vec3::ZERO)),
            Node::Subtract { children } | Node::SmoothSubtract { children, .. } => children
                .first()
                .map(|c| c.aabb())
                .unwrap_or(Aabb::new(Vec3::ZERO, Vec3::ZERO)),
            Node::Intersect { children } | Node::SmoothIntersect { children, .. } => children
                .iter()
                .map(|c| c.aabb())
                .reduce(Aabb::intersect)
                .unwrap_or(Aabb::new(Vec3::ZERO, Vec3::ZERO)),
            Node::SmoothUnion { k, children } => children
                .iter()
                .map(|c| c.aabb())
                .reduce(Aabb::union)
                .unwrap_or(Aabb::new(Vec3::ZERO, Vec3::ZERO))
                .expand(*k * 0.5),
            Node::Shell { thickness, child } => child.aabb().expand(*thickness),
            Node::Translate { v, child } => {
                let a = child.aabb();
                let t = Vec3::from_arr(*v);
                Aabb::new(a.min + t, a.max + t)
            }
            Node::Rotate { q, child } => {
                let a = child.aabb();
                if !a.is_finite() {
                    return Aabb::huge();
                }
                let q = Quat(*q);
                let pts: Vec<Vec3> = a.corners().iter().map(|c| q.rotate(*c)).collect();
                Aabb::from_points(&pts)
            }
            Node::Scale { s, child } => {
                let a = child.aabb();
                if !a.is_finite() {
                    return Aabb::huge();
                }
                let s = Vec3::from_arr(*s);
                let pts: Vec<Vec3> = a.corners().iter().map(|c| c.mul(s)).collect();
                Aabb::from_points(&pts)
            }
            Node::RadialRepeat { n, child } => {
                if *n <= 1 {
                    return child.aabb();
                }
                let a = child.aabb();
                if !a.is_finite() {
                    return Aabb::huge();
                }
                let r = a
                    .corners()
                    .iter()
                    .map(|c| v2len(c.x, c.z))
                    .fold(0.0f32, f32::max);
                Aabb::new(v3(-r, a.min.y, -r), v3(r, a.max.y, r))
            }
            Node::GridRepeat { cell, count, child } => {
                let a = child.aabb();
                if !a.is_finite() {
                    return Aabb::huge();
                }
                let e = v3(
                    cell[0].abs() * (count[0].saturating_sub(1)) as f32 * 0.5,
                    cell[1].abs() * (count[1].saturating_sub(1)) as f32 * 0.5,
                    cell[2].abs() * (count[2].saturating_sub(1)) as f32 * 0.5,
                );
                Aabb::new(a.min - e, a.max + e)
            }
            Node::Mirror { axis, child } => {
                let a = child.aabb();
                if !a.is_finite() {
                    return Aabb::huge();
                }
                let m = match axis {
                    Axis::X => Aabb::new(
                        v3(-a.max.x, a.min.y, a.min.z),
                        v3(-a.min.x, a.max.y, a.max.z),
                    ),
                    Axis::Y => Aabb::new(
                        v3(a.min.x, -a.max.y, a.min.z),
                        v3(a.max.x, -a.min.y, a.max.z),
                    ),
                    Axis::Z => Aabb::new(
                        v3(a.min.x, a.min.y, -a.max.z),
                        v3(a.max.x, a.max.y, -a.min.z),
                    ),
                };
                a.union(m)
            }
        }
    }
}

// --- builder helpers used by the scene assembly ---

pub fn sphere(r: f32) -> Node {
    Node::Prim { prim: Prim::Sphere { r } }
}
pub fn cuboid(half: [f32; 3], round: f32) -> Node {
    Node::Prim { prim: Prim::Box { half, round } }
}
pub fn cylinder(r: f32, h: f32) -> Node {
    Node::Prim { prim: Prim::Cylinder { r, h } }
}
pub fn cone(r1: f32, r2: f32, h: f32) -> Node {
    Node::Prim { prim: Prim::Cone { r1, r2, h } }
}
pub fn capsule(a: [f32; 3], b: [f32; 3], r: f32) -> Node {
    Node::Prim { prim: Prim::Capsule { a, b, r } }
}
pub fn union(children: Vec<Node>) -> Node {
    Node::Union { children }
}
pub fn smooth_union(k: f32, children: Vec<Node>) -> Node {
    Node::SmoothUnion { k, children }
}
pub fn translate(v: [f32; 3], child: Node) -> Node {
    Node::Translate { v, child: Box::new(child) }
}
pub fn rotate(q: Quat, child: Node) -> Node {
    Node::Rotate { q: q.0, child: Box::new(child) }
}
pub fn rotate_y(angle: f32, child: Node) -> Node {
    rotate(Quat::from_axis_angle(Vec3::Y, angle), child)
}
pub fn scale(s: [f32; 3], child: Node) -> Node {
    Node::Scale { s, child: Box::new(child) }
}
pub fn scale_u(f: f32, child: Node) -> Node {
    scale([f, f, f], child)
}
pub fn grid_repeat(cell: [f32; 3], count: [u32; 3], child: Node) -> Node {
    Node::GridRepeat { cell, count, child: Box::new(child) }
}

// ---------------------------------------------------------------------------
// Stream D tests: transform correctness (non-uniform scale bound, rotation-
// chain parity vs the da-csg mesh backend, fillet/chamfer parity).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- deterministic pseudo-randomness (no rand crate, no time) ----------

    /// Knuth MMIX LCG; fixed seed makes every "random" chain reproducible.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        /// Uniform in [0, 1).
        fn f01(&mut self) -> f64 {
            (self.next() >> 40) as f64 / (1u64 << 24) as f64
        }
        /// Uniform in [lo, hi).
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.f01()
        }
        fn axis(&mut self) -> &'static str {
            ["x", "y", "z"][(self.next() >> 33) as usize % 3]
        }
        fn unit_dir(&mut self) -> Vec3 {
            loop {
                let v = v3(
                    self.range(-1.0, 1.0) as f32,
                    self.range(-1.0, 1.0) as f32,
                    self.range(-1.0, 1.0) as f32,
                );
                let l = v.length();
                if l > 0.2 {
                    return v * (1.0 / l);
                }
            }
        }
    }

    // --- D1: the non-uniform-scale distance bound ---------------------------

    /// The min-component factor must be a *safe* bound: taking a full sphere-
    /// trace step of length dist(p) in any direction must never cross the
    /// surface. Checked on a fixed-seed set of outside points and directions
    /// against a strongly anisotropic scale (ratio 32).
    #[test]
    fn scale_bound_never_oversteps() {
        let node = scale([8.0, 0.25, 8.0], sphere(1.0));
        let mut rng = Lcg(0xD1_5EED);
        let mut checked = 0;
        while checked < 300 {
            let p = v3(
                rng.range(-12.0, 12.0) as f32,
                rng.range(-3.0, 3.0) as f32,
                rng.range(-12.0, 12.0) as f32,
            );
            let d = node.dist(p);
            if d <= 1e-3 {
                continue; // want strictly-outside starting points
            }
            let dir = rng.unit_dir();
            // Sample the whole step segment: it must stay outside (>= -eps).
            for k in 0..=32 {
                let t = d * k as f32 / 32.0;
                let dq = node.dist(p + dir * t);
                assert!(
                    dq >= -1e-4,
                    "overstep: start {p:?} d={d} dir {dir:?} t={t} -> {dq}"
                );
            }
            checked += 1;
        }
    }

    /// Worst-case cost of the conservative bound, pinned as a number: a ray
    /// approaching the ellipsoid scale([8, 0.25, 8], sphere(1)) along its
    /// most-stretched axis shrinks each step to remaining/ratio, so the march
    /// needs ~ratio * ln(r0/eps) steps (ratio 32 -> ~330). The test asserts
    /// convergence and caps the count, so a regression in the bound (either
    /// unsafe or grossly slower) fails loudly. The same ray against a uniform
    /// scale converges almost immediately (exact distance).
    #[test]
    fn scale_aniso_march_count_is_bounded() {
        let march = |node: &Node, mut t: f32| -> (bool, u32) {
            let (ro, rd) = (v3(40.0, 0.0, 0.0), v3(-1.0, 0.0, 0.0));
            for step in 0..2000 {
                let d = node.dist(ro + rd * t);
                if d < 1e-3 {
                    return (true, step);
                }
                t += d;
                if t > 100.0 {
                    break;
                }
            }
            (false, 2000)
        };

        let aniso = scale([8.0, 0.25, 8.0], sphere(1.0));
        let (hit, steps) = march(&aniso, 0.0);
        println!("D1 aniso ratio-32 march: hit={hit} steps={steps}");
        assert!(hit, "anisotropic march must still converge");
        // Theory: ~ratio * ln(32/1e-3) = 32 * 10.4 ~ 333. Cap with margin.
        assert!(steps <= 400, "worst-case step count regressed: {steps}");
        // And it *is* the conservative regime (documents the known cost).
        assert!(steps >= 100, "unexpectedly cheap ({steps}) — bound changed?");

        let uniform = scale([8.0, 8.0, 8.0], sphere(1.0));
        let (hit_u, steps_u) = march(&uniform, 0.0);
        println!("D1 uniform scale march: hit={hit_u} steps={steps_u}");
        assert!(hit_u && steps_u <= 5, "uniform scale should be ~exact: {steps_u}");
    }

    // --- mesh-side helpers for the parity tests -----------------------------

    type P3 = [f64; 3];

    fn sub3(a: P3, b: P3) -> P3 {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }
    fn dot3(a: P3, b: P3) -> f64 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }
    fn cross3(a: P3, b: P3) -> P3 {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }

    /// Moller-Trumbore, f64, counting hits strictly in front of the origin.
    fn ray_hits_tri(o: P3, d: P3, a: P3, b: P3, c: P3) -> bool {
        let (e1, e2) = (sub3(b, a), sub3(c, a));
        let pv = cross3(d, e2);
        let det = dot3(e1, pv);
        if det.abs() < 1e-12 {
            return false;
        }
        let inv = 1.0 / det;
        let s = sub3(o, a);
        let u = dot3(s, pv) * inv;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let q = cross3(s, e1);
        let v = dot3(d, q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return false;
        }
        dot3(e2, q) * inv > 1e-9
    }

    /// Compile a .vim source through the mesh backend into Y-up triangles.
    fn mesh_tris(src: &str) -> Vec<[P3; 3]> {
        let solid = da_csg::compile_vim(src)
            .unwrap_or_else(|e| panic!("mesh compile failed: {e}\n{src}"))
            .solid;
        let (pos, idx) = solid.to_mesh_yup();
        idx.chunks(3)
            .map(|t| {
                let g = |i: u32| {
                    let p = pos[i as usize];
                    [p.x as f64, p.y as f64, p.z as f64]
                };
                [g(t[0]), g(t[1]), g(t[2])]
            })
            .collect()
    }

    /// Odd fixed direction: keeps the parity ray off edges/vertices.
    const RAY_DIR: P3 = [0.531_223, 0.724_708, 0.438_916];

    fn point_in_mesh(p: Vec3, tris: &[[P3; 3]]) -> bool {
        let o = [p.x as f64, p.y as f64, p.z as f64];
        let mut crossings = 0u32;
        for t in tris {
            if ray_hits_tri(o, RAY_DIR, t[0], t[1], t[2]) {
                crossings += 1;
            }
        }
        crossings % 2 == 1
    }

    fn mesh_aabb(tris: &[[P3; 3]]) -> Aabb {
        let pts: Vec<Vec3> = tris
            .iter()
            .flatten()
            .map(|p| v3(p[0] as f32, p[1] as f32, p[2] as f32))
            .collect();
        Aabb::from_points(&pts)
    }

    /// Compile the same .vim source through the SDF backend into a Node.
    fn sdf_node(src: &str) -> Node {
        let v = da_csg::compile_sdf(src)
            .unwrap_or_else(|e| panic!("sdf compile failed: {e}\n{src}"));
        serde_json::from_value(v).expect("osgedit Node should deserialize")
    }

    /// Inside/outside sign agreement between the two backends on a fixed
    /// grid over `bb`; points within `band` of the SDF surface are skipped
    /// (mesh faceting makes the near-surface sign legitimately ambiguous).
    /// Returns (agree, counted).
    fn sign_agreement(node: &Node, tris: &[[P3; 3]], bb: Aabb, n: usize, band: f32) -> (u32, u32) {
        let (mut agree, mut counted) = (0u32, 0u32);
        for ix in 0..n {
            for iy in 0..n {
                for iz in 0..n {
                    let f = |i: usize, lo: f32, hi: f32| {
                        lo + (hi - lo) * (i as f32 + 0.5) / n as f32
                    };
                    let p = v3(
                        f(ix, bb.min.x, bb.max.x),
                        f(iy, bb.min.y, bb.max.y),
                        f(iz, bb.min.z, bb.max.z),
                    );
                    let d = node.dist(p);
                    if d.abs() < band {
                        continue;
                    }
                    counted += 1;
                    if (d < 0.0) == point_in_mesh(p, tris) {
                        agree += 1;
                    }
                }
            }
        }
        (agree, counted)
    }

    // --- D2: rotation-chain parity between backends --------------------------

    /// Fixed-seed axis/angle chains (with a translate in the middle) applied
    /// to every analytic primitive, evaluated through BOTH backends: the SDF
    /// AABB must contain the mesh AABB (and stay sane), and inside/outside
    /// signs must agree on a fixed grid away from the faceted surface.
    #[test]
    fn rotation_chain_parity_between_backends() {
        let prims = [
            "cube(2)",
            "box(1.5, 2.5, 0.8)",
            "cylinder(r = 1.2, h = 3, seg = 128)",
            "frustum(1.5, 0.6, 2.2, 128)",
            "sphere(1.3, 96)",
            "torus(1.5, 0.4, 96, 48)",
            "pyramid(2.0, 1.4, 1.8)",
            "wedge(2.0, 1.2, 1.6)",
        ];
        let mut rng = Lcg(0xD2_5EED);
        let (mut worst_agree, mut total_agree, mut total_counted) = (1.0f64, 0u64, 0u64);
        let mut worst_case = String::new();

        for prim in prims {
            for _chain in 0..2 {
                let mut src = format!("model {prim}");
                for k in 0..3 {
                    src.push_str(&format!(
                        ".rotate(\"{}\", {:.1})",
                        rng.axis(),
                        rng.range(0.0, 360.0)
                    ));
                    if k == 1 {
                        src.push_str(&format!(
                            ".move({:.2}, {:.2}, {:.2})",
                            rng.range(-2.0, 2.0),
                            rng.range(-2.0, 2.0),
                            rng.range(-2.0, 2.0)
                        ));
                    }
                }

                let tris = mesh_tris(&src);
                let node = sdf_node(&src);
                let mb = mesh_aabb(&tris);
                let sb = node.aabb();

                // The mesh inscribes the analytic surface; the SDF AABB is a
                // conservative hull of it — so mesh ⊆ sdf, within tolerance.
                for (m, s) in [
                    (mb.min.x, sb.min.x),
                    (mb.min.y, sb.min.y),
                    (mb.min.z, sb.min.z),
                ] {
                    assert!(m >= s - 0.02, "mesh AABB min escapes SDF AABB: {src}");
                }
                for (m, s) in [
                    (mb.max.x, sb.max.x),
                    (mb.max.y, sb.max.y),
                    (mb.max.z, sb.max.z),
                ] {
                    assert!(m <= s + 0.02, "mesh AABB max escapes SDF AABB: {src}");
                }
                // Conservatism stays bounded (rotating AABB corners inflates
                // by at most sqrt(3) per rotation in the chain).
                let mext = (mb.max - mb.min).max_comp();
                let sext = (sb.max - sb.min).max_comp();
                assert!(
                    sext <= mext * 5.5 + 0.2,
                    "SDF AABB uselessly loose ({sext} vs {mext}): {src}"
                );

                let (agree, counted) =
                    sign_agreement(&node, &tris, mb.expand(0.4), 13, 0.03);
                let frac = agree as f64 / counted as f64;
                total_agree += agree as u64;
                total_counted += counted as u64;
                if frac < worst_agree {
                    worst_agree = frac;
                    worst_case = src.clone();
                }
                assert!(
                    frac >= 0.999,
                    "sign agreement {frac:.4} ({agree}/{counted}) too low: {src}"
                );
            }
        }
        println!(
            "D2 parity: {total_agree}/{total_counted} grid signs agree \
             ({:.5}), worst chain {:.5} [{worst_case}]",
            total_agree as f64 / total_counted as f64,
            worst_agree
        );
    }

    // --- D3: fillet/chamfer parity between backends ---------------------------

    /// Grid-sampled volume of a Node (deterministic, cell-center counting).
    fn grid_volume(node: &Node, bb: Aabb, n: usize) -> f64 {
        let ext = bb.max - bb.min;
        let cell = (ext.x as f64 / n as f64)
            * (ext.y as f64 / n as f64)
            * (ext.z as f64 / n as f64);
        let mut inside = 0u64;
        for ix in 0..n {
            for iy in 0..n {
                for iz in 0..n {
                    let f = |i: usize, lo: f32, hi: f32| {
                        lo + (hi - lo) * (i as f32 + 0.5) / n as f32
                    };
                    let p = v3(
                        f(ix, bb.min.x, bb.max.x),
                        f(iy, bb.min.y, bb.max.y),
                        f(iz, bb.min.z, bb.max.z),
                    );
                    if node.dist(p) < 0.0 {
                        inside += 1;
                    }
                }
            }
        }
        inside as f64 * cell
    }

    /// cube(2).fillet(0.4): the SDF maps to the exact rounded box, whose
    /// volume is closed-form; the mesh beveler circumscribes the same round
    /// with tangent planes, so its volume brackets the analytic one from
    /// above (and both stay below the sharp cube).
    #[test]
    fn fillet_volume_parity_with_mesh_backend() {
        let src = "model cube(2).fillet(0.4, 24)";
        let (a, r) = (2.0f64, 0.4f64);
        let core = a - 2.0 * r;
        let v_round = core.powi(3)
            + 6.0 * core * core * r
            + 3.0 * std::f64::consts::PI * r * r * core
            + 4.0 / 3.0 * std::f64::consts::PI * r.powi(3);

        let node = sdf_node(src);
        let v_sdf = grid_volume(&node, node.aabb().expand(0.05), 201);
        println!("D3 fillet volumes: analytic {v_round:.4} sdf-grid {v_sdf:.4}");
        assert!(
            (v_sdf - v_round).abs() < 0.04,
            "SDF grid volume {v_sdf} vs analytic rounded box {v_round}"
        );

        let v_mesh = da_csg::compile_vim(src).unwrap().solid.volume();
        println!("D3 fillet mesh volume: {v_mesh:.4}");
        assert!(
            v_mesh >= v_round - 0.01 && v_mesh <= v_round + 0.25 && v_mesh < 8.0,
            "mesh fillet volume {v_mesh} should bracket analytic {v_round} from above"
        );
    }

    /// box.chamfer(d) cuts the *same 12 planes* in both backends, so the two
    /// geometries are identical up to grid resolution: volumes match and the
    /// sign grid agrees essentially everywhere.
    #[test]
    fn chamfer_parity_with_mesh_backend() {
        let src = "model box(2, 3, 4).chamfer(0.35)";
        let node = sdf_node(src);
        let tris = mesh_tris(src);

        let v_mesh = da_csg::compile_vim(src).unwrap().solid.volume();
        let v_sdf = grid_volume(&node, node.aabb().expand(0.05), 101);
        println!("D3 chamfer volumes: mesh {v_mesh:.4} sdf-grid {v_sdf:.4}");
        assert!(
            (v_sdf - v_mesh).abs() < 0.15,
            "chamfer volumes disagree: mesh {v_mesh} sdf {v_sdf}"
        );

        let (agree, counted) = sign_agreement(&node, &tris, mesh_aabb(&tris).expand(0.3), 13, 0.02);
        let frac = agree as f64 / counted as f64;
        println!("D3 chamfer sign agreement: {agree}/{counted} ({frac:.5})");
        assert!(frac >= 0.999, "chamfer sign agreement {frac}");
    }

    /// The D3 acceptance prop: dumpster.vim (chamfer on a moved box) renders
    /// the same solid through both backends.
    #[test]
    fn dumpster_prop_parity_between_backends() {
        let src = include_str!("../../../assets/props/builtin/dumpster.vim");
        let node = sdf_node(src);
        let tris = mesh_tris(src);
        let (agree, counted) = sign_agreement(&node, &tris, mesh_aabb(&tris).expand(0.3), 13, 0.02);
        let frac = agree as f64 / counted as f64;
        println!("D3 dumpster sign agreement: {agree}/{counted} ({frac:.5})");
        assert!(frac >= 0.999, "dumpster sign agreement {frac}");
    }

    /// A deterministic grid of probe points around the unit-ish scale.
    fn probes() -> Vec<Vec3> {
        let mut out = Vec::new();
        let vals = [-3.0f32, -1.7, -0.9, -0.3, 0.0, 0.4, 1.1, 2.6];
        for &x in &vals {
            for &y in &vals {
                for &z in &vals {
                    out.push(v3(x, y, z));
                }
            }
        }
        out
    }

    #[test]
    fn polygon_sdf_unit_square_is_exact() {
        // Square [-1,1]^2, CCW.
        let sq = [[-1.0f32, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        assert!((polygon_sdf(&sq, 0.0, 0.0) - (-1.0)).abs() < 1e-6);
        assert!((polygon_sdf(&sq, 2.0, 0.0) - 1.0).abs() < 1e-6);
        assert!((polygon_sdf(&sq, 2.0, 2.0) - std::f32::consts::SQRT_2).abs() < 1e-6);
        assert!((polygon_sdf(&sq, 0.5, 0.75) - (-0.25)).abs() < 1e-6);
        // Winding-agnostic: CW gives the same field.
        let cw = [[-1.0f32, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        for p in [(0.3f32, -0.8f32), (1.5, 0.2), (-2.0, 1.0)] {
            assert!((polygon_sdf(&sq, p.0, p.1) - polygon_sdf(&cw, p.0, p.1)).abs() < 1e-6);
        }
    }

    #[test]
    fn lathe_of_rect_profile_matches_cylinder() {
        // Profile (r, z): 0..r=0.8, z in [0, 2] → a Z-axis cylinder sitting on
        // z=0. Compare to the closed-form Cylinder (Y-axis, centered) by
        // swapping the axes and centering.
        let (r, h) = (0.8f32, 2.0f32);
        let lathe = Prim::Lathe { pts: vec![[0.0, 0.0], [r, 0.0], [r, h], [0.0, h]] };
        let cyl = Prim::Cylinder { r, h };
        for p in probes() {
            let expect = cyl.dist(v3(p.x, p.z - h * 0.5, p.y));
            let got = lathe.dist(p);
            assert!(
                (got - expect).abs() < 1e-5,
                "lathe vs cylinder at {p:?}: {got} vs {expect}"
            );
        }
    }

    #[test]
    fn lathe_of_triangle_profile_matches_cone() {
        // Profile (0,0)-(r,0)-(0,h): a cone, base radius r at z=0, apex at z=h.
        let (r, h) = (1.2f32, 2.4f32);
        let lathe = Prim::Lathe { pts: vec![[0.0, 0.0], [r, 0.0], [0.0, h]] };
        let cone = Prim::Cone { r1: r, r2: 0.0, h };
        for p in probes() {
            let expect = cone.dist(v3(p.x, p.z - h * 0.5, p.y));
            let got = lathe.dist(p);
            assert!(
                (got - expect).abs() < 1e-5,
                "lathe vs cone at {p:?}: {got} vs {expect}"
            );
        }
    }

    #[test]
    fn lathe_mirror_profile_still_solid() {
        // A profile living entirely at r < 0 sweeps the same solid as its
        // mirror: an annular tube, inner 1, outer 2, z in [0, 1].
        let lathe = Prim::Lathe {
            pts: vec![[-2.0, 0.0], [-1.0, 0.0], [-1.0, 1.0], [-2.0, 1.0]],
        };
        assert!(lathe.dist(v3(1.5, 0.0, 0.5)) < 0.0, "inside the swept wall");
        assert!(lathe.dist(v3(0.0, 0.0, 0.5)) > 0.0, "the bore is empty");
        assert!(lathe.dist(v3(2.5, 0.0, 0.5)) > 0.0, "outside is outside");
    }

    #[test]
    fn lathe_aabb_bounds_the_field() {
        let lathe = Prim::Lathe { pts: vec![[0.0, 0.0], [0.8, 0.0], [0.8, 2.0], [0.0, 2.0]] };
        let bb = lathe.aabb();
        assert_eq!(bb, Aabb::new(v3(-0.8, -0.8, 0.0), v3(0.8, 0.8, 2.0)));
        for p in probes() {
            if lathe.dist(p) < 0.0 {
                assert!(bb.distance(p) == 0.0, "negative SDF outside the AABB at {p:?}");
            }
        }
    }

    #[test]
    fn extrude_of_rect_matches_box() {
        // Rect w×d in (x, y), extruded h along z == Box with those half-extents.
        let (w, d, h) = (1.6f32, 0.9f32, 2.2f32);
        let ext = Prim::Extrude {
            pts: vec![
                [-w / 2.0, -d / 2.0],
                [w / 2.0, -d / 2.0],
                [w / 2.0, d / 2.0],
                [-w / 2.0, d / 2.0],
            ],
            h,
            twist_deg: 0.0,
        };
        let bx = Prim::Box { half: [w / 2.0, d / 2.0, h / 2.0], round: 0.0 };
        for p in probes() {
            let (got, expect) = (ext.dist(p), bx.dist(p));
            assert!(
                (got - expect).abs() < 1e-5,
                "extrude vs box at {p:?}: {got} vs {expect}"
            );
        }
        assert_eq!(
            ext.aabb(),
            Aabb::new(v3(-w / 2.0, -d / 2.0, -h / 2.0), v3(w / 2.0, d / 2.0, h / 2.0))
        );
    }

    #[test]
    fn extrude_twist_rotates_the_section() {
        // A long thin blade (4 × 0.2) twisted 90°: at the top the section has
        // rotated onto the Y axis. Check sign at the ends and that the twisted
        // distances never overstep the true surface (conservative marching).
        let ext = Prim::Extrude {
            pts: vec![[-2.0, -0.1], [2.0, -0.1], [2.0, 0.1], [-2.0, 0.1]],
            h: 2.0,
            twist_deg: 90.0,
        };
        // Bottom (z=-1): rotation is 0° → blade along X.
        assert!(ext.dist(v3(1.8, 0.0, -0.99)) < 0.0);
        assert!(ext.dist(v3(0.0, 1.8, -0.99)) > 0.0);
        // Top (z=+1): rotated +90° → blade along Y.
        assert!(ext.dist(v3(0.0, 1.8, 0.99)) < 0.0);
        assert!(ext.dist(v3(1.8, 0.0, 0.99)) > 0.0);
        // Conservative: stepping d from p never lands inside the solid.
        for p in probes() {
            let d = ext.dist(p);
            if d <= 0.0 {
                continue;
            }
            // March straight toward the origin-ish center by d; must not
            // overshoot into d' < -1e-4 (tiny slack for f32).
            let dir = (v3(0.0, 0.0, 0.0) - p).normalize();
            let q = p + dir * d;
            assert!(ext.dist(q) > -1e-4, "overstepped at {p:?}: d={d}, next={}", ext.dist(q));
        }
        // Twisted AABB covers the swept circle.
        let bb = ext.aabb();
        assert!(bb.min.x <= -2.0 && bb.max.y >= 2.0);
    }

    #[test]
    fn lathe_extrude_serde_round_trip() {
        let n = Node::Union {
            children: vec![
                Node::Prim { prim: Prim::Lathe { pts: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 2.0]] } },
                Node::Prim {
                    prim: Prim::Extrude { pts: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], h: 3.0, twist_deg: 0.0 },
                },
            ],
        };
        let j = serde_json::to_string(&n).unwrap();
        assert!(j.contains("\"lathe\"") && j.contains("\"extrude\""));
        let back: Node = serde_json::from_str(&j).unwrap();
        assert_eq!(n, back);
        // twist_deg is optional in the JSON (defaults to 0).
        let e: Prim = serde_json::from_str(
            r#"{ "shape": "extrude", "pts": [[0,0],[1,0],[0,1]], "h": 2.0 }"#,
        )
        .unwrap();
        assert_eq!(e, Prim::Extrude { pts: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], h: 2.0, twist_deg: 0.0 });
    }
}
