//! BSP-tree Constructive Solid Geometry core.
//!
//! A direct, well-tested approach (after Evan Wallace's csg.js, MIT): a solid is
//! a set of convex polygons; boolean ops are computed by clipping each solid's
//! polygons against a BSP tree of the other. All math is f64 for robustness;
//! meshes are emitted as f32 only at the very end (see `super::mesh`).
//!
//! Reliability over features: every primitive here produces *convex* polygons
//! with consistent outward winding, which is exactly what this algorithm needs.

const EPSILON: f64 = 1e-7;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct V3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

pub fn v3(x: f64, y: f64, z: f64) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
    pub fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
    pub fn mul(self, s: f64) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
    pub fn dot(self, o: V3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }
    pub fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> V3 {
        let l = self.len();
        if l < EPSILON {
            v3(0.0, 0.0, 0.0)
        } else {
            self.mul(1.0 / l)
        }
    }
    pub fn lerp(self, o: V3, t: f64) -> V3 {
        self.add(o.sub(self).mul(t))
    }
    pub fn negated(self) -> V3 {
        self.mul(-1.0)
    }
}

/// A vertex carries a position and a (smooth) normal so primitives like spheres
/// keep their shading through boolean ops.
#[derive(Clone, Copy, Debug)]
pub struct Vertex {
    pub pos: V3,
    pub normal: V3,
}

impl Vertex {
    pub fn new(pos: V3, normal: V3) -> Self {
        Vertex { pos, normal }
    }
    fn flip(&mut self) {
        self.normal = self.normal.negated();
    }
    fn interpolate(&self, other: &Vertex, t: f64) -> Vertex {
        Vertex {
            pos: self.pos.lerp(other.pos, t),
            normal: self.normal.lerp(other.normal, t),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Plane {
    normal: V3,
    w: f64,
}

impl Plane {
    fn from_points(a: V3, b: V3, c: V3) -> Plane {
        let normal = b.sub(a).cross(c.sub(a)).normalized();
        Plane {
            normal,
            w: normal.dot(a),
        }
    }
    fn flip(&mut self) {
        self.normal = self.normal.negated();
        self.w = -self.w;
    }
}

/// A convex polygon (planar, consistently wound). `part` tags which primitive
/// this polygon belongs to (an index into `Solid::parts`); it rides along
/// through splits, booleans, and transforms so each primitive keeps its
/// identity — and its own temperature — in the final solid.
#[derive(Clone, Debug)]
pub struct Polygon {
    pub vertices: Vec<Vertex>,
    pub part: u32,
    plane: Plane,
}

impl Polygon {
    pub fn new(vertices: Vec<Vertex>) -> Polygon {
        debug_assert!(vertices.len() >= 3);
        let plane = Plane::from_points(vertices[0].pos, vertices[1].pos, vertices[2].pos);
        Polygon { vertices, part: 0, plane }
    }
    /// As [`Polygon::new`] but with an explicit part tag.
    pub fn with_part(vertices: Vec<Vertex>, part: u32) -> Polygon {
        let mut p = Polygon::new(vertices);
        p.part = part;
        p
    }
    pub fn flip(&mut self) {
        self.vertices.reverse();
        for v in &mut self.vertices {
            v.flip();
        }
        self.plane.flip();
    }
}

// Polygon classification during BSP splitting.
const COPLANAR: i32 = 0;
const FRONT: i32 = 1;
const BACK: i32 = 2;
const SPANNING: i32 = 3;

impl Plane {
    /// Split `polygon` by this plane, appending the pieces to the four buckets.
    fn split_polygon(
        &self,
        polygon: &Polygon,
        coplanar_front: &mut Vec<Polygon>,
        coplanar_back: &mut Vec<Polygon>,
        front: &mut Vec<Polygon>,
        back: &mut Vec<Polygon>,
    ) {
        let mut polygon_type = 0;
        let mut types = Vec::with_capacity(polygon.vertices.len());
        for v in &polygon.vertices {
            let t = self.normal.dot(v.pos) - self.w;
            let ty = if t < -EPSILON {
                BACK
            } else if t > EPSILON {
                FRONT
            } else {
                COPLANAR
            };
            polygon_type |= ty;
            types.push(ty);
        }

        match polygon_type {
            COPLANAR => {
                if self.normal.dot(polygon.plane.normal) > 0.0 {
                    coplanar_front.push(polygon.clone());
                } else {
                    coplanar_back.push(polygon.clone());
                }
            }
            FRONT => front.push(polygon.clone()),
            BACK => back.push(polygon.clone()),
            _ => {
                // SPANNING: clip the polygon into a front piece and a back piece.
                let mut f = Vec::new();
                let mut b = Vec::new();
                let n = polygon.vertices.len();
                for i in 0..n {
                    let j = (i + 1) % n;
                    let ti = types[i];
                    let tj = types[j];
                    let vi = &polygon.vertices[i];
                    let vj = &polygon.vertices[j];
                    if ti != BACK {
                        f.push(*vi);
                    }
                    if ti != FRONT {
                        b.push(*vi);
                    }
                    if (ti | tj) == SPANNING {
                        let t = (self.w - self.normal.dot(vi.pos))
                            / self.normal.dot(vj.pos.sub(vi.pos));
                        let mid = vi.interpolate(vj, t);
                        f.push(mid);
                        b.push(mid);
                    }
                }
                if f.len() >= 3 {
                    front.push(Polygon::with_part(f, polygon.part));
                }
                if b.len() >= 3 {
                    back.push(Polygon::with_part(b, polygon.part));
                }
            }
        }
    }
}

/// A node of the BSP tree.
struct Node {
    plane: Option<Plane>,
    front: Option<Box<Node>>,
    back: Option<Box<Node>>,
    polygons: Vec<Polygon>,
}

impl Node {
    fn new() -> Node {
        Node {
            plane: None,
            front: None,
            back: None,
            polygons: Vec::new(),
        }
    }

    fn from_polygons(polygons: Vec<Polygon>) -> Node {
        let mut n = Node::new();
        n.build(polygons);
        n
    }

    fn invert(&mut self) {
        for p in &mut self.polygons {
            p.flip();
        }
        if let Some(pl) = &mut self.plane {
            pl.flip();
        }
        if let Some(f) = &mut self.front {
            f.invert();
        }
        if let Some(b) = &mut self.back {
            b.invert();
        }
        std::mem::swap(&mut self.front, &mut self.back);
    }

    fn clip_polygons(&self, polygons: Vec<Polygon>) -> Vec<Polygon> {
        let plane = match self.plane {
            Some(p) => p,
            None => return polygons,
        };
        let mut cf = Vec::new();
        let mut cb = Vec::new();
        let mut front = Vec::new();
        let mut back = Vec::new();
        for p in &polygons {
            plane.split_polygon(p, &mut cf, &mut cb, &mut front, &mut back);
        }
        front.extend(cf); // coplanar-front polygons travel with the front set
        back.extend(cb);
        let mut front = match &self.front {
            Some(f) => f.clip_polygons(front),
            None => front,
        };
        let back = match &self.back {
            Some(b) => b.clip_polygons(back),
            None => Vec::new(),
        };
        front.extend(back);
        front
    }

    fn clip_to(&mut self, other: &Node) {
        self.polygons = other.clip_polygons(std::mem::take(&mut self.polygons));
        if let Some(f) = &mut self.front {
            f.clip_to(other);
        }
        if let Some(b) = &mut self.back {
            b.clip_to(other);
        }
    }

    fn all_polygons(&self) -> Vec<Polygon> {
        let mut out = self.polygons.clone();
        if let Some(f) = &self.front {
            out.extend(f.all_polygons());
        }
        if let Some(b) = &self.back {
            out.extend(b.all_polygons());
        }
        out
    }

    fn build(&mut self, polygons: Vec<Polygon>) {
        if polygons.is_empty() {
            return;
        }
        if self.plane.is_none() {
            self.plane = Some(polygons[0].plane);
        }
        let plane = self.plane.unwrap();
        let mut cf = Vec::new();
        let mut cb = Vec::new();
        let mut front = Vec::new();
        let mut back = Vec::new();
        for p in &polygons {
            plane.split_polygon(p, &mut cf, &mut cb, &mut front, &mut back);
        }
        // Coplanar polygons (either orientation) belong to this node.
        self.polygons.extend(cf);
        self.polygons.extend(cb);
        if !front.is_empty() {
            self.front
                .get_or_insert_with(|| Box::new(Node::new()))
                .build(front);
        }
        if !back.is_empty() {
            self.back
                .get_or_insert_with(|| Box::new(Node::new()))
                .build(back);
        }
    }
}

/// `a ∪ b` — merge two solids.
pub fn union(a: Vec<Polygon>, b: Vec<Polygon>) -> Vec<Polygon> {
    let mut a = Node::from_polygons(a);
    let mut b = Node::from_polygons(b);
    a.clip_to(&b);
    b.clip_to(&a);
    b.invert();
    b.clip_to(&a);
    b.invert();
    a.build(b.all_polygons());
    a.all_polygons()
}

/// `a \ b` — subtract b from a (drilling / cutting).
pub fn difference(a: Vec<Polygon>, b: Vec<Polygon>) -> Vec<Polygon> {
    let mut a = Node::from_polygons(a);
    let mut b = Node::from_polygons(b);
    a.invert();
    a.clip_to(&b);
    b.clip_to(&a);
    b.invert();
    b.clip_to(&a);
    b.invert();
    a.build(b.all_polygons());
    a.invert();
    a.all_polygons()
}

/// `a ∩ b` — keep only the overlap.
pub fn intersection(a: Vec<Polygon>, b: Vec<Polygon>) -> Vec<Polygon> {
    let mut a = Node::from_polygons(a);
    let mut b = Node::from_polygons(b);
    a.invert();
    b.clip_to(&a);
    b.invert();
    a.clip_to(&b);
    b.clip_to(&a);
    a.build(b.all_polygons());
    a.invert();
    a.all_polygons()
}
