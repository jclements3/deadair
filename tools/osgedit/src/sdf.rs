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
