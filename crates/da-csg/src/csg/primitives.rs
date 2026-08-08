//! The standard solid primitives, as convex-polygon soups for the CSG core.
//!
//! Convention: **every primitive is centered at the origin.** Round shapes run
//! along +Z. Users position parts with `.move` / `.rotate`. Consistent centering
//! keeps booleans and transforms easy to reason about.

use super::bsp::{v3, Polygon, Vertex, V3};
use std::f64::consts::PI;

/// Flat face: one shared normal derived from the winding (used for planar faces).
fn flat(pts: Vec<V3>) -> Polygon {
    let n = pts[1].sub(pts[0]).cross(pts[2].sub(pts[0])).normalized();
    Polygon::new(pts.into_iter().map(|p| Vertex::new(p, n)).collect())
}

/// Axis-aligned block of size `w`×`d`×`h` (x,y,z), centered at origin.
pub fn cube(w: f64, d: f64, h: f64) -> Vec<Polygon> {
    let (x, y, z) = (w / 2.0, d / 2.0, h / 2.0);
    // 8 corners
    let c = [
        v3(-x, -y, -z),
        v3(x, -y, -z),
        v3(x, y, -z),
        v3(-x, y, -z),
        v3(-x, -y, z),
        v3(x, -y, z),
        v3(x, y, z),
        v3(-x, y, z),
    ];
    // 6 faces, each wound CCW as seen from outside.
    vec![
        flat(vec![c[0], c[3], c[2], c[1]]), // bottom (-z)
        flat(vec![c[4], c[5], c[6], c[7]]), // top (+z)
        flat(vec![c[0], c[1], c[5], c[4]]), // front (-y)
        flat(vec![c[2], c[3], c[7], c[6]]), // back (+y)
        flat(vec![c[1], c[2], c[6], c[5]]), // right (+x)
        flat(vec![c[0], c[4], c[7], c[3]]), // left (-x)
    ]
}

/// Ring of positions at height `z`, radius `r`.
fn ring(r: f64, z: f64, seg: usize) -> Vec<V3> {
    (0..seg)
        .map(|i| {
            let t = i as f64 / seg as f64 * 2.0 * PI;
            v3(r * t.cos(), r * t.sin(), z)
        })
        .collect()
}

/// Truncated cone: bottom radius `r1` (at −h/2), top radius `r2` (at +h/2).
/// This is the general body-of-revolution frustum; `cylinder`/`cone` defer to it.
pub fn frustum(r1: f64, r2: f64, h: f64, seg: usize) -> Vec<Polygon> {
    let zb = -h / 2.0;
    let zt = h / 2.0;
    let mut polys = Vec::new();
    let bottom = ring(r1, zb, seg);
    let top = ring(r2, zt, seg);

    // Side wall — smooth radial normals so cylinders/cones shade cleanly.
    let slope_n = |x: f64, y: f64, r: f64| {
        // Outward normal of a cone surface: radial component scaled by dr, plus
        // an axial component from the taper.
        let radial = if r > 1e-9 {
            v3(x / r, y / r, 0.0)
        } else {
            v3(0.0, 0.0, 0.0)
        };
        let axial = (r1 - r2) / h; // dz slope
        v3(radial.x, radial.y, axial).normalized()
    };
    for i in 0..seg {
        let j = (i + 1) % seg;
        let b0 = bottom[i];
        let b1 = bottom[j];
        let t0 = top[i];
        let t1 = top[j];
        let n_b0 = slope_n(b0.x, b0.y, r1);
        let n_b1 = slope_n(b1.x, b1.y, r1);
        let n_t0 = slope_n(t0.x, t0.y, r2);
        let n_t1 = slope_n(t1.x, t1.y, r2);
        if r2 < 1e-9 {
            // Degenerate top → triangle to the apex.
            polys.push(Polygon::new(vec![
                Vertex::new(b0, n_b0),
                Vertex::new(b1, n_b1),
                Vertex::new(t0, n_t0),
            ]));
        } else if r1 < 1e-9 {
            polys.push(Polygon::new(vec![
                Vertex::new(b0, n_b0),
                Vertex::new(t1, n_t1),
                Vertex::new(t0, n_t0),
            ]));
        } else {
            polys.push(Polygon::new(vec![
                Vertex::new(b0, n_b0),
                Vertex::new(b1, n_b1),
                Vertex::new(t1, n_t1),
                Vertex::new(t0, n_t0),
            ]));
        }
    }
    // Caps.
    if r1 > 1e-9 {
        let n = v3(0.0, 0.0, -1.0);
        let center = Vertex::new(v3(0.0, 0.0, zb), n);
        for i in 0..seg {
            let j = (i + 1) % seg;
            polys.push(Polygon::new(vec![
                center,
                Vertex::new(bottom[j], n),
                Vertex::new(bottom[i], n),
            ]));
        }
    }
    if r2 > 1e-9 {
        let n = v3(0.0, 0.0, 1.0);
        let center = Vertex::new(v3(0.0, 0.0, zt), n);
        for i in 0..seg {
            let j = (i + 1) % seg;
            polys.push(Polygon::new(vec![
                center,
                Vertex::new(top[i], n),
                Vertex::new(top[j], n),
            ]));
        }
    }
    polys
}

/// Right circular cylinder, radius `r`, height `h`.
pub fn cylinder(r: f64, h: f64, seg: usize) -> Vec<Polygon> {
    frustum(r, r, h, seg)
}

/// Cone: base radius `r` at −h/2 tapering to a point at +h/2.
pub fn cone(r: f64, h: f64, seg: usize) -> Vec<Polygon> {
    frustum(r, 0.0, h, seg)
}

/// UV sphere of radius `r`, centered at origin.
pub fn sphere(r: f64, seg: usize) -> Vec<Polygon> {
    let u = seg.max(3);
    let v = (seg / 2).max(2);
    let vert = |theta: f64, phi: f64| {
        let dir = v3(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
        Vertex::new(dir.mul(r), dir)
    };
    let mut polys = Vec::new();
    for i in 0..u {
        let t0 = i as f64 / u as f64 * 2.0 * PI;
        let t1 = (i + 1) as f64 / u as f64 * 2.0 * PI;
        for j in 0..v {
            let p0 = j as f64 / v as f64 * PI;
            let p1 = (j + 1) as f64 / v as f64 * PI;
            let a = vert(t0, p0);
            let b = vert(t1, p0);
            let c = vert(t1, p1);
            let d = vert(t0, p1);
            // Wound so the plane normal points outward (matches vertex normals).
            if j == 0 {
                polys.push(Polygon::new(vec![a, d, c])); // top cap triangle
            } else if j == v - 1 {
                polys.push(Polygon::new(vec![a, c, b])); // bottom cap triangle
            } else {
                polys.push(Polygon::new(vec![a, d, c, b]));
            }
        }
    }
    polys
}

/// Torus: center-circle radius `rr`, tube radius `r`, lying in the XY plane.
pub fn torus(rr: f64, r: f64, seg_major: usize, seg_minor: usize) -> Vec<Polygon> {
    let uu = seg_major.max(3);
    let vv = seg_minor.max(3);
    let vert = |u: f64, v: f64| {
        let radial = v3(u.cos(), u.sin(), 0.0);
        let center = radial.mul(rr);
        let normal = radial.mul(v.cos()).add(v3(0.0, 0.0, v.sin()));
        let pos = center.add(normal.mul(r));
        Vertex::new(pos, normal.normalized())
    };
    let mut polys = Vec::new();
    for i in 0..uu {
        let u0 = i as f64 / uu as f64 * 2.0 * PI;
        let u1 = (i + 1) as f64 / uu as f64 * 2.0 * PI;
        for j in 0..vv {
            let v0 = j as f64 / vv as f64 * 2.0 * PI;
            let v1 = (j + 1) as f64 / vv as f64 * 2.0 * PI;
            polys.push(Polygon::new(vec![
                vert(u0, v0),
                vert(u1, v0),
                vert(u1, v1),
                vert(u0, v1),
            ]));
        }
    }
    polys
}

/// Right triangular prism (wedge): width `w` (x), depth `d` (y), height `h` (z).
/// Cross-section in XZ is a right triangle; the slope faces +x/+z.
pub fn wedge(w: f64, d: f64, h: f64) -> Vec<Polygon> {
    let (x, y, z) = (w / 2.0, d / 2.0, h / 2.0);
    // Triangle profile in XZ: (-x,-z), (x,-z), (-x,z). Extruded along y.
    let a0 = v3(-x, -y, -z);
    let b0 = v3(x, -y, -z);
    let c0 = v3(-x, -y, z);
    let a1 = v3(-x, y, -z);
    let b1 = v3(x, y, -z);
    let c1 = v3(-x, y, z);
    vec![
        flat(vec![a0, b0, c0]),      // front triangle (-y)
        flat(vec![a1, c1, b1]),      // back triangle (+y)
        flat(vec![a0, a1, b1, b0]),  // bottom (-z)
        flat(vec![a0, c0, c1, a1]),  // vertical (-x)
        flat(vec![b0, b1, c1, c0]),  // hypotenuse
    ]
}

/// Rectangular pyramid: base `w`×`d` at −h/2, apex at +h/2 on the axis.
pub fn pyramid(w: f64, d: f64, h: f64) -> Vec<Polygon> {
    let (x, y, z) = (w / 2.0, d / 2.0, h / 2.0);
    let b0 = v3(-x, -y, -z);
    let b1 = v3(x, -y, -z);
    let b2 = v3(x, y, -z);
    let b3 = v3(-x, y, -z);
    let apex = v3(0.0, 0.0, z);
    vec![
        flat(vec![b0, b3, b2, b1]), // base (-z)
        flat(vec![b0, b1, apex]),   // sides
        flat(vec![b1, b2, apex]),
        flat(vec![b2, b3, apex]),
        flat(vec![b3, b0, apex]),
    ]
}
