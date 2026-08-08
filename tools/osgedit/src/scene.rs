//! World assembly: the golfer vignette, the farm, the town and the mountains,
//! composed from the conference-demo CSG model library (models/*.json) plus a
//! few inline CSG props (club, ball, flag, silo, crop rows, fences).

use crate::sdf::*;
use crate::volumetric::{Volumetric, VolumetricSpec};
use serde::Deserialize;
use std::path::Path;

// ---------------------------------------------------------------------------
// Model files
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FileTransform {
    #[serde(default = "zero3")]
    pos: [f32; 3],
    #[serde(default = "ident_q")]
    rot: [f32; 4],
    #[serde(default = "one3")]
    scale: [f32; 3],
}
fn zero3() -> [f32; 3] {
    [0.0; 3]
}
fn one3() -> [f32; 3] {
    [1.0; 3]
}
fn ident_q() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

#[derive(Deserialize)]
struct ModelFile {
    #[serde(default)]
    #[allow(dead_code)]
    id: String,
    transform: Option<FileTransform>,
    /// The solid CSG tree. Optional: a model may be volumetrics only, in
    /// which case there is simply nothing for pass 1 to draw.
    csg: Option<Node>,
    /// Density fields composited over the solid render -- a *sibling* of
    /// `csg`, never nested inside it (see `crate::volumetric`).
    #[serde(default)]
    volumetrics: Vec<VolumetricSpec>,
}

impl ModelFile {
    fn node(&self) -> Node {
        let csg = self.csg.clone().unwrap_or(Node::Union { children: Vec::new() });
        match &self.transform {
            Some(t) => translate(t.pos, rotate(Quat(t.rot), scale(t.scale, csg))),
            None => csg,
        }
    }

    /// The volumetrics, with the file's own `transform` folded in. The AABB
    /// of transformed corners is conservative but always contains the field,
    /// which is all pass 2's slab test needs.
    fn vols(&self) -> Vec<Volumetric> {
        let outer = match &self.transform {
            Some(t) => file_transform_matrix(t),
            None => IDENT4,
        };
        self.volumetrics
            .iter()
            .map(|s| {
                let mut s = s.clone();
                if outer != IDENT4 {
                    s.xform = mat4_mul(&outer, &s.xform);
                    let (mn, mx) = transformed_bounds(&outer, &s.bounds.min, &s.bounds.max);
                    s.bounds.min = mn;
                    s.bounds.max = mx;
                }
                Volumetric::load(&s).unwrap_or_else(|e| panic!("{e}"))
            })
            .collect()
    }
}

const IDENT4: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, //
    0.0, 1.0, 0.0, 0.0, //
    0.0, 0.0, 1.0, 0.0, //
    0.0, 0.0, 0.0, 1.0,
];

fn mat4_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0f32; 16];
    for r in 0..4 {
        for c in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[r * 4 + k] * b[k * 4 + c];
            }
            o[r * 4 + c] = s;
        }
    }
    o
}

/// `translate(pos) * rotate(rot) * scale(scale)`, matching `ModelFile::node`.
fn file_transform_matrix(t: &FileTransform) -> [f32; 16] {
    let q = Quat(t.rot);
    let (cx, cy, cz) = (q.rotate(v3(1.0, 0.0, 0.0)), q.rotate(v3(0.0, 1.0, 0.0)), q.rotate(v3(0.0, 0.0, 1.0)));
    [
        cx.x * t.scale[0], cy.x * t.scale[1], cz.x * t.scale[2], t.pos[0], //
        cx.y * t.scale[0], cy.y * t.scale[1], cz.y * t.scale[2], t.pos[1], //
        cx.z * t.scale[0], cy.z * t.scale[1], cz.z * t.scale[2], t.pos[2], //
        0.0, 0.0, 0.0, 1.0,
    ]
}

fn transformed_bounds(m: &[f32; 16], mn: &[f32; 3], mx: &[f32; 3]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for i in 0..8 {
        let c = [
            if i & 1 == 0 { mn[0] } else { mx[0] },
            if i & 2 == 0 { mn[1] } else { mx[1] },
            if i & 4 == 0 { mn[2] } else { mx[2] },
        ];
        for k in 0..3 {
            let v = m[k * 4] * c[0] + m[k * 4 + 1] * c[1] + m[k * 4 + 2] * c[2] + m[k * 4 + 3];
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    (lo, hi)
}

fn read_model_file(file: &Path) -> ModelFile {
    let text = std::fs::read_to_string(file)
        .unwrap_or_else(|e| panic!("cannot read model {}: {e}", file.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("cannot parse model {}: {e}", file.display()))
}

pub fn load_model(dir: &Path, name: &str) -> Node {
    read_model_file(&dir.join(format!("{name}.json"))).node()
}

/// Load one model file as a standalone single-object scene (`--model`):
/// the prop on the ground plane, neutral material — for inspecting `.vim`
/// exports at full quality without the conference world around them.
pub fn single(file: &Path) -> Scene {
    let f = read_model_file(file);
    let node = f.node();
    // A volumetrics-only model has no surface to shade; skip the empty
    // object rather than feed the marcher a degenerate tree.
    let objects = if matches!(&node, Node::Union { children } if children.is_empty()) {
        Vec::new()
    } else {
        vec![obj("model", node, Mat::Flat(v3(0.62, 0.60, 0.56)), 0.18)]
    };
    Scene { objects, volumetrics: f.vols() }
}

// ---------------------------------------------------------------------------
// Materials / objects
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Mat {
    /// Single albedo.
    Flat(Vec3),
    /// Blend low->high albedo over a world-space Y band (tree trunks vs
    /// canopy, rock vs snow caps).
    HeightBlend { low: Vec3, high: Vec3, y0: f32, y1: f32 },
}

pub struct Object {
    pub name: &'static str,
    pub node: Node,
    pub mat: Mat,
    pub spec: f32,
    pub aabb: Aabb,
}

pub struct Scene {
    pub objects: Vec<Object>,
    /// Density fields composited after the solids (pass 2). Empty for every
    /// scene that predates volumetrics -- and the renderer short-circuits on
    /// empty, so those scenes render bit-identically to before.
    pub volumetrics: Vec<Volumetric>,
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Mat {
    pub fn albedo(&self, p: Vec3) -> Vec3 {
        match *self {
            Mat::Flat(c) => c,
            Mat::HeightBlend { low, high, y0, y1 } => low.lerp(high, smoothstep(y0, y1, p.y)),
        }
    }
}

// ---------------------------------------------------------------------------
// Placement helpers
// ---------------------------------------------------------------------------

/// Uniform-scale a model to a target height and set it on the ground (y=0)
/// at (x, z), rotated `yaw` radians about Y.
fn place_h(model: &Node, height: f32, yaw: f32, x: f32, z: f32) -> Node {
    let a = model.aabb();
    let s = height / (a.max.y - a.min.y).max(1e-6);
    let scaled = rotate_y(yaw, scale_u(s, model.clone()));
    let sa = scaled.aabb();
    translate([x, -sa.min.y, z], scaled)
}

fn obj(name: &'static str, node: Node, mat: Mat, spec: f32) -> Object {
    let aabb = node.aabb();
    Object { name, node, mat, spec, aabb }
}

// palette
const GRASS_DK: Vec3 = v3(0.10, 0.34, 0.10);
const TRUNK: Vec3 = v3(0.28, 0.19, 0.11);
const CANOPY: Vec3 = v3(0.12, 0.38, 0.12);
const BARN_RED: Vec3 = v3(0.55, 0.12, 0.09);
const SILO_GRAY: Vec3 = v3(0.68, 0.68, 0.66);
const WOOD: Vec3 = v3(0.45, 0.33, 0.20);
const ROCK: Vec3 = v3(0.19, 0.155, 0.125);
const SNOW: Vec3 = v3(0.92, 0.93, 0.96);

// ---------------------------------------------------------------------------
// Scene build
// ---------------------------------------------------------------------------

pub fn build(models_dir: &Path) -> Scene {
    let figure = load_model(models_dir, "figure");
    let tree = load_model(models_dir, "tree");
    let building = load_model(models_dir, "building");
    let buildings = load_model(models_dir, "buildings");
    let vehicle = load_model(models_dir, "vehicle");
    let lattice = load_model(models_dir, "lattice");
    let plane_mdl = load_model(models_dir, "plane");
    let quad = load_model(models_dir, "quad");
    let mountain = load_model(models_dir, "mountain");
    let hills = load_model(models_dir, "hills");

    let mut o: Vec<Object> = Vec::new();

    // ---- golfer vignette (near the origin) ----
    // tee green: a wide flat disc
    o.push(obj(
        "green",
        translate([0.0, -0.06, 0.0], cylinder(7.0, 0.16)),
        Mat::Flat(v3(0.13, 0.45, 0.14)),
        0.0,
    ));
    // the golfer: library figure, human height, facing down-range (+x)
    o.push(obj(
        "golfer",
        place_h(&figure, 1.8, -0.35, 0.0, 0.0),
        Mat::Flat(v3(0.75, 0.72, 0.66)),
        0.15,
    ));
    // club: thin capsule from hand height to the ball
    o.push(obj(
        "club",
        union(vec![
            capsule([0.18, 1.05, 0.12], [0.62, 0.07, 0.34], 0.017),
            // club head
            translate(
                [0.66, 0.05, 0.36],
                cuboid([0.055, 0.03, 0.028], 0.02),
            ),
        ]),
        Mat::Flat(v3(0.25, 0.26, 0.28)),
        0.5,
    ));
    // ball on a tee
    o.push(obj(
        "ball",
        translate([0.78, 0.05, 0.42], sphere(0.035)),
        Mat::Flat(v3(0.95, 0.95, 0.95)),
        0.6,
    ));
    // pin + flag ~11 m down-range
    o.push(obj(
        "flag",
        union(vec![
            translate([11.0, 1.15, -3.0], cylinder(0.035, 2.3)),
            translate([11.32, 2.12, -3.0], cuboid([0.32, 0.15, 0.012], 0.0)),
        ]),
        Mat::Flat(v3(0.80, 0.10, 0.10)),
        0.2,
    ));
    // a pair of trees framing the tee
    for (i, (x, z, h, yaw)) in [
        (-3.5f32, -4.0f32, 5.0f32, 0.4f32),
        (-5.5, 7.5, 4.2, 2.1),
        (13.5, 2.5, 4.6, 1.2),
    ]
    .iter()
    .enumerate()
    {
        let n = place_h(&tree, *h, *yaw, *x, *z);
        let a = n.aabb();
        o.push(obj(
            if i == 0 { "tee_tree_a" } else if i == 1 { "tee_tree_b" } else { "tee_tree_c" },
            n,
            Mat::HeightBlend {
                low: TRUNK,
                high: CANOPY,
                y0: a.max.y * 0.18,
                y1: a.max.y * 0.42,
            },
            0.0,
        ));
    }

    // ---- the farm (around x = 60) ----
    // barn: library building scaled up, barn red
    o.push(obj(
        "barn",
        place_h(&building, 8.5, 0.55, 60.0, -10.0),
        Mat::Flat(BARN_RED),
        0.05,
    ));
    // silo: inline CSG — cylinder + dome cap
    o.push(obj(
        "silo",
        smooth_union(
            0.3,
            vec![
                translate([66.5, 4.5, -13.5], cylinder(2.0, 9.0)),
                translate([66.5, 9.0, -13.5], sphere(2.0)),
            ],
        ),
        Mat::Flat(SILO_GRAY),
        0.3,
    ));
    // tractor: library vehicle
    o.push(obj(
        "tractor",
        place_h(&vehicle, 2.6, -0.5, 54.0, -2.0),
        Mat::Flat(v3(0.10, 0.35, 0.12)),
        0.25,
    ));
    // crop rows: a grid-repeated low ridge — CSG repetition on show
    o.push(obj(
        "crops",
        translate(
            [58.0, 0.0, 14.0],
            rotate_y(
                0.1,
                grid_repeat(
                    [0.0, 0.0, 1.9],
                    [1, 1, 12],
                    cuboid([16.0, 0.30, 0.55], 0.25),
                ),
            ),
        ),
        Mat::Flat(v3(0.32, 0.40, 0.13)),
        0.0,
    ));
    // fence along the road: repeated posts + a rail
    o.push(obj(
        "fence",
        translate(
            [56.0, 0.0, 5.4],
            union(vec![
                grid_repeat([3.0, 0.0, 0.0], [13, 1, 1], cuboid([0.07, 0.65, 0.07], 0.01)),
                translate([0.0, 1.05, 0.0], cuboid([19.5, 0.05, 0.04], 0.01)),
            ]),
        ),
        Mat::Flat(WOOD),
        0.0,
    ));
    // orchard row behind the barn
    for (i, x) in [48.0f32, 53.0, 58.0, 63.0, 68.0].iter().enumerate() {
        let n = place_h(&tree, 4.5, i as f32 * 1.3, *x, -20.0);
        let a = n.aabb();
        o.push(obj(
            "orchard",
            n,
            Mat::HeightBlend {
                low: TRUNK,
                high: CANOPY,
                y0: a.max.y * 0.18,
                y1: a.max.y * 0.42,
            },
            0.0,
        ));
        let _ = i;
    }

    // ---- the town (around x = 140) ----
    // main street cluster: the library "buildings" block, twice, plus singles
    o.push(obj(
        "town_block_a",
        place_h(&buildings, 12.0, 0.0, 138.0, -12.0),
        Mat::Flat(v3(0.60, 0.55, 0.48)),
        0.05,
    ));
    o.push(obj(
        "town_block_b",
        place_h(&buildings, 9.0, 3.14159, 152.0, 12.0),
        Mat::Flat(v3(0.52, 0.48, 0.45)),
        0.05,
    ));
    o.push(obj(
        "town_hall",
        place_h(&building, 10.0, -1.571, 128.0, 11.0),
        Mat::Flat(v3(0.72, 0.66, 0.55)),
        0.05,
    ));
    o.push(obj(
        "shop",
        place_h(&building, 6.0, 1.571, 146.0, -9.0),
        Mat::Flat(v3(0.35, 0.42, 0.55)),
        0.05,
    ));
    // radio mast on the edge of town: the lattice model stretched into a
    // thin tower (non-uniform scale is part of the CSG show)
    let mast = {
        let a = lattice.aabb();
        let s = 16.0 / (a.max.y - a.min.y).max(1e-6);
        let n = rotate_y(0.3, scale([s * 0.22, s, s * 0.22], lattice.clone()));
        let na = n.aabb();
        translate([160.0, -na.min.y, -16.0], n)
    };
    o.push(obj("radio_mast", mast, Mat::Flat(v3(0.55, 0.30, 0.20)), 0.3));
    // parked cars
    o.push(obj(
        "car_a",
        place_h(&vehicle, 1.6, 0.0, 133.0, 4.2),
        Mat::Flat(v3(0.55, 0.08, 0.08)),
        0.5,
    ));
    o.push(obj(
        "car_b",
        place_h(&vehicle, 1.6, 3.14159, 143.0, 7.8),
        Mat::Flat(v3(0.12, 0.20, 0.45)),
        0.5,
    ));
    // street trees
    for (i, (x, z)) in [(126.0f32, -4.0f32), (135.0, 10.0), (150.0, -5.0)].iter().enumerate() {
        let n = place_h(&tree, 3.8, i as f32 * 0.9, *x, *z);
        let a = n.aabb();
        o.push(obj(
            "street_tree",
            n,
            Mat::HeightBlend {
                low: TRUNK,
                high: CANOPY,
                y0: a.max.y * 0.18,
                y1: a.max.y * 0.42,
            },
            0.0,
        ));
    }
    // a light aircraft over town and a quadcopter — the rest of the library
    o.push(obj(
        "aircraft",
        translate(
            [148.0, 26.0, 14.0],
            rotate_y(2.6, scale_u(3.2, plane_mdl.clone())),
        ),
        Mat::Flat(v3(0.85, 0.85, 0.88)),
        0.4,
    ));
    o.push(obj(
        "quadcopter",
        translate([131.0, 9.5, -2.0], rotate_y(0.8, scale_u(1.4, quad.clone()))),
        Mat::Flat(v3(0.20, 0.20, 0.22)),
        0.4,
    ));

    // ---- the mountains (x = 240 and beyond) ----
    // the gallery "hills" diorama doesn't scale up (it has its own plinth
    // and tree), so the range is layered mountain instances instead; each
    // is sunk slightly to bury the plane-cut skirt at its base.
    for (name, h, yaw, x, z) in [
        ("mountain_a", 70.0f32, 0.0f32, 290.0f32, -35.0f32),
        ("mountain_b", 95.0, 1.2, 335.0, 25.0),
        ("mountain_c", 80.0, 2.3, 310.0, 90.0),
        ("mountain_d", 60.0, 3.6, 278.0, -95.0),
        ("mountain_e", 40.0, 0.7, 248.0, 58.0),
        ("mountain_f", 46.0, 1.9, 255.0, -55.0),
    ] {
        let n = translate([0.0, -h * 0.10, 0.0], place_h(&mountain, h, yaw, x, z));
        o.push(obj(
            name,
            n,
            Mat::HeightBlend { low: ROCK, high: SNOW, y0: h * 0.52, y1: h * 0.76 },
            0.0,
        ));
    }

    let _ = (GRASS_DK, hills);
    Scene { objects: o, volumetrics: Vec::new() }
}
