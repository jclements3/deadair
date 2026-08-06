//! Mesh export to STL (binary + ASCII) and OBJ.
//!
//! Two layers:
//!   * Generic functions over raw triangle soups (`&[[[f32; 3]; 3]]`) so any
//!     mesh source — not just this crate's CSG kernel — can be exported.
//!   * Thin `Solid` conveniences that fan-triangulate the convex polygons first.

/// Compute the facet normal of a triangle via the cross product of two edges.
/// Falls back to a zero normal for degenerate triangles.
fn facet_normal(t: &[[f32; 3]; 3]) -> [f32; 3] {
    let (a, b, c) = (t[0], t[1], t[2]);
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 0.0 {
        [n[0] / len, n[1] / len, n[2] / len]
    } else {
        [0.0, 0.0, 0.0]
    }
}

// --- generic triangle-soup exporters --------------------------------------

/// Binary STL: 80-byte header, `u32` triangle count, then 50 bytes/triangle
/// (facet normal + 3 vertices as little-endian `f32`, plus a `u16` attribute
/// byte count of 0). Facet normals are computed from each triangle.
pub fn stl_binary_from_triangles(tris: &[[[f32; 3]; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(84 + 50 * tris.len());
    out.extend_from_slice(&[0u8; 80]); // header
    out.extend_from_slice(&(tris.len() as u32).to_le_bytes());
    for t in tris {
        let n = facet_normal(t);
        for c in n {
            out.extend_from_slice(&c.to_le_bytes());
        }
        for v in t {
            for c in v {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out.extend_from_slice(&0u16.to_le_bytes()); // attribute byte count
    }
    out
}

/// ASCII STL. Starts with `solid <name>` and ends with `endsolid <name>`, one
/// `facet normal` block per triangle.
pub fn stl_ascii_from_triangles(tris: &[[[f32; 3]; 3]], name: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("solid {name}\n"));
    for t in tris {
        let n = facet_normal(t);
        s.push_str(&format!("  facet normal {} {} {}\n", n[0], n[1], n[2]));
        s.push_str("    outer loop\n");
        for v in t {
            s.push_str(&format!("      vertex {} {} {}\n", v[0], v[1], v[2]));
        }
        s.push_str("    endloop\n");
        s.push_str("  endfacet\n");
    }
    s.push_str(&format!("endsolid {name}\n"));
    s
}

/// Wavefront OBJ. Emits one `v` line per triangle vertex (3 per triangle) and
/// one `f` line per triangle referencing them in order — simple and lossless
/// for a triangle soup.
pub fn obj_from_triangles(tris: &[[[f32; 3]; 3]], name: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("# {name}\n"));
    s.push_str(&format!("o {name}\n"));
    for t in tris {
        for v in t {
            s.push_str(&format!("v {} {} {}\n", v[0], v[1], v[2]));
        }
    }
    for i in 0..tris.len() {
        let base = i * 3 + 1; // OBJ indices are 1-based
        s.push_str(&format!("f {} {} {}\n", base, base + 1, base + 2));
    }
    s
}

// --- Solid conveniences ----------------------------------------------------

/// Fan-triangulate a solid's convex polygons into a triangle soup. Each polygon
/// with vertices `v0..vn` yields triangles `(v0, vi, vi+1)`.
pub fn triangles_of(solid: &crate::csg::Solid) -> Vec<[[f32; 3]; 3]> {
    let mut tris = Vec::with_capacity(solid.triangle_count());
    for p in &solid.polys {
        let vs = &p.vertices;
        let v0 = vs[0].pos;
        let p0 = [v0.x as f32, v0.y as f32, v0.z as f32];
        for i in 1..vs.len() - 1 {
            let a = vs[i].pos;
            let b = vs[i + 1].pos;
            tris.push([
                p0,
                [a.x as f32, a.y as f32, a.z as f32],
                [b.x as f32, b.y as f32, b.z as f32],
            ]);
        }
    }
    tris
}

pub fn stl_binary(solid: &crate::csg::Solid) -> Vec<u8> {
    stl_binary_from_triangles(&triangles_of(solid))
}

pub fn stl_ascii(solid: &crate::csg::Solid) -> String {
    stl_ascii_from_triangles(&triangles_of(solid), "solid")
}

pub fn obj(solid: &crate::csg::Solid) -> String {
    obj_from_triangles(&triangles_of(solid), "solid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::csg::Solid;

    #[test]
    fn cube_triangle_count() {
        let c = Solid::cube(2.0, 2.0, 2.0);
        let tris = triangles_of(&c);
        assert_eq!(tris.len(), 12, "cube should fan-triangulate to 12 triangles");
        assert_eq!(tris.len(), c.triangle_count());
    }

    #[test]
    fn binary_stl_layout() {
        let c = Solid::cube(2.0, 2.0, 2.0);
        let bytes = stl_binary(&c);
        assert_eq!(bytes.len(), 84 + 50 * 12);
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]);
        assert_eq!(count, 12);
    }

    #[test]
    fn ascii_stl_facets() {
        let c = Solid::cube(2.0, 2.0, 2.0);
        let s = stl_ascii(&c);
        assert!(s.starts_with("solid"));
        assert_eq!(s.matches("facet normal").count(), 12);
    }

    #[test]
    fn obj_lines() {
        let c = Solid::cube(2.0, 2.0, 2.0);
        let s = obj(&c);
        assert_eq!(s.matches("\nv ").count() + s.starts_with("v ") as usize, 36);
        assert_eq!(s.lines().filter(|l| l.starts_with("v ")).count(), 36);
        assert_eq!(s.lines().filter(|l| l.starts_with("f ")).count(), 12);
    }

    #[test]
    fn binary_count_matches_ascii_facets() {
        let c = Solid::cube(2.0, 2.0, 2.0);
        let bytes = stl_binary(&c);
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        let ascii_facets = stl_ascii(&c).matches("facet normal").count();
        assert_eq!(count, ascii_facets);
    }
}
