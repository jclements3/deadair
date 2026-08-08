//! E2 (Stream E referee) — silhouette parity between the two backends.
//!
//! The same `.vim` script is compiled twice: through `da_csg::compile_sdf`
//! into this tool's analytic `Node` tree (evaluated with the *real*
//! `Node::dist`, so exporter emission, transform conventions, and the SDF
//! semantics are all on trial), and through `da_csg::compile_vim` into the
//! BSP triangle mesh. Both are imaged from the same camera — the SDF by
//! deterministic sphere tracing, the mesh by perspective rasterization —
//! and the two binary coverage masks must agree to IoU > 0.995. Any axis,
//! handedness, anchor, or Z-up/Y-up convention bug in either backend drags
//! the IoU toward zero long before it gets near the threshold.
//!
//! Everything here is deterministic: pixel-center rays, no jitter, no RNG.

use crate::sdf::{v3, Node, Vec3};

const RES: usize = 256;
const IOU_MIN: f64 = 0.995;
const FOV_DEG: f32 = 35.0;

struct Cam {
    eye: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
    /// tan(fov/2), square frame (same convention as render.rs).
    half: f32,
}

/// Frame the node's AABB from a fixed three-quarter direction.
fn camera(node: &Node) -> Cam {
    let a = node.aabb();
    let c = (a.min + a.max) * 0.5;
    let ext = (a.max - a.min).max_comp().max(1.0);
    let dir = v3(0.62, 0.34, 0.72).normalize();
    let eye = c + dir * (ext * 2.6);
    let fwd = (c - eye).normalize();
    let right = fwd.cross(v3(0.0, 1.0, 0.0)).normalize();
    let up = right.cross(fwd);
    Cam { eye, right, up, fwd, half: (FOV_DEG.to_radians() * 0.5).tan() }
}

/// Sphere-trace one ray; `true` if it hits the surface within `t_max`.
fn trace(node: &Node, ro: Vec3, rd: Vec3, t_max: f32) -> bool {
    let mut t = 0.0f32;
    for _ in 0..512 {
        if t >= t_max {
            return false;
        }
        let d = node.dist(ro + rd * t);
        if d < 1e-3 {
            return true;
        }
        t += d * 0.9;
    }
    false
}

/// SDF coverage mask at pixel centers.
fn sdf_mask(node: &Node, cam: &Cam) -> Vec<bool> {
    let a = node.aabb();
    let c = (a.min + a.max) * 0.5;
    let t_max = (c - cam.eye).length() + (a.max - a.min).length() + 1.0;
    let mut mask = vec![false; RES * RES];
    for y in 0..RES {
        for x in 0..RES {
            let u = ((x as f32 + 0.5) / RES as f32) * 2.0 - 1.0;
            let v = 1.0 - ((y as f32 + 0.5) / RES as f32) * 2.0;
            let rd = (cam.fwd + cam.right * (u * cam.half) + cam.up * (v * cam.half))
                .normalize();
            mask[y * RES + x] = trace(node, cam.eye, rd, t_max);
        }
    }
    mask
}

/// Mesh coverage mask: perspective-project each triangle (script Z-up
/// converted to the world's Y-up exactly like the exporter's root rotation:
/// (x, y, z) -> (x, z, -y)) and fill pixel centers inside it.
fn mesh_mask(tris: &[[[f32; 3]; 3]], cam: &Cam) -> Vec<bool> {
    let mut mask = vec![false; RES * RES];
    'tri: for t in tris {
        let mut s = [[0.0f32; 2]; 3];
        for i in 0..3 {
            let p = v3(t[i][0], t[i][2], -t[i][1]) - cam.eye;
            let cz = p.dot(cam.fwd);
            if cz < 1e-3 {
                continue 'tri; // behind the camera — not framed here anyway
            }
            let u = p.dot(cam.right) / (cz * cam.half);
            let v = p.dot(cam.up) / (cz * cam.half);
            // continuous pixel coords: pixel-center (ix, iy) sits at (ix, iy)
            s[i] = [
                (u + 1.0) * 0.5 * RES as f32 - 0.5,
                (1.0 - v) * 0.5 * RES as f32 - 0.5,
            ];
        }
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1])
            - (s[1][1] - s[0][1]) * (s[2][0] - s[0][0]);
        if area.abs() < 1e-9 {
            continue;
        }
        let xs = [s[0][0], s[1][0], s[2][0]];
        let ys = [s[0][1], s[1][1], s[2][1]];
        let x0 = xs.iter().fold(f32::MAX, |a, &b| a.min(b)).floor().max(0.0) as usize;
        let x1 = xs.iter().fold(f32::MIN, |a, &b| a.max(b)).ceil().min(RES as f32 - 1.0) as usize;
        let y0 = ys.iter().fold(f32::MAX, |a, &b| a.min(b)).floor().max(0.0) as usize;
        let y1 = ys.iter().fold(f32::MIN, |a, &b| a.max(b)).ceil().min(RES as f32 - 1.0) as usize;
        let sign = area.signum();
        for py in y0..=y1 {
            for px in x0..=x1 {
                let (fx, fy) = (px as f32, py as f32);
                let mut inside = true;
                for e in 0..3 {
                    let (ax, ay) = (s[e][0], s[e][1]);
                    let (bx, by) = (s[(e + 1) % 3][0], s[(e + 1) % 3][1]);
                    let w = (bx - ax) * (fy - ay) - (by - ay) * (fx - ax);
                    if w * sign < -1e-6 {
                        inside = false;
                        break;
                    }
                }
                if inside {
                    mask[py * RES + px] = true;
                }
            }
        }
    }
    mask
}

fn iou(a: &[bool], b: &[bool]) -> f64 {
    let (mut inter, mut union) = (0u64, 0u64);
    for (x, y) in a.iter().zip(b) {
        inter += (*x && *y) as u64;
        union += (*x || *y) as u64;
    }
    if union == 0 {
        return 0.0;
    }
    inter as f64 / union as f64
}

/// Compile `src` through both backends, image both from the same camera,
/// and assert silhouette agreement.
fn assert_silhouette_parity(name: &str, src: &str) {
    let json = da_csg::compile_sdf(src)
        .unwrap_or_else(|e| panic!("{name}: SDF export failed: {e}"));
    let node: Node = serde_json::from_value(json)
        .unwrap_or_else(|e| panic!("{name}: exported JSON is not a valid osgedit Node: {e}"));
    let compiled = da_csg::compile_vim(src)
        .unwrap_or_else(|e| panic!("{name}: mesh compile failed: {e}"));
    let tris = da_csg::triangles_of(&compiled.solid);
    assert!(!tris.is_empty(), "{name}: mesh backend produced no triangles");

    let cam = camera(&node);
    let sdf = sdf_mask(&node, &cam);
    let mesh = mesh_mask(&tris, &cam);

    let cover_sdf = sdf.iter().filter(|&&b| b).count();
    let cover_mesh = mesh.iter().filter(|&&b| b).count();
    let floor = RES * RES / 100; // the model must actually be in frame
    assert!(cover_sdf > floor, "{name}: SDF silhouette near-empty ({cover_sdf} px)");
    assert!(cover_mesh > floor, "{name}: mesh silhouette near-empty ({cover_mesh} px)");

    let score = iou(&sdf, &mesh);
    eprintln!(
        "{name}: silhouette IoU {score:.5} (sdf {cover_sdf} px, mesh {cover_mesh} px, \
         {} triangles)",
        tris.len()
    );
    assert!(
        score > IOU_MIN,
        "{name}: silhouette IoU {score:.4} <= {IOU_MIN} \
         (sdf {cover_sdf} px, mesh {cover_mesh} px) — backend divergence"
    );
}

#[test]
fn e2_silhouette_parity_water_tower() {
    // Round parts + polar array: the curvature stress case.
    assert_silhouette_parity(
        "water_tower",
        include_str!("../../../assets/props/water_tower.vim"),
    );
}

#[test]
fn e2_silhouette_parity_warehouse() {
    // Box hall, cylinder∩box barrel vault, subtracted doors, array + mirror.
    assert_silhouette_parity(
        "warehouse",
        include_str!("../../../assets/props/warehouse.vim"),
    );
}

#[test]
fn e2_silhouette_parity_church() {
    // Wedge roof, pyramid spire, rotated-cylinder arch cut, arrays, mirror.
    // (The fixed copy at assets/props/ — the crate-side one fails to lex.)
    assert_silhouette_parity(
        "church",
        include_str!("../../../assets/props/church.vim"),
    );
}
