//! CPU-side primitive mesh generation for the geometry pass.
//!
//! Everything the parametric generators emit is boxes, cylinders, and
//! spheres (plus ground patches); we bake one unit mesh per primitive and
//! scale per-instance.

use bytemuck::{Pod, Zeroable};

/// Vertex layout shared by all pipelines: position + normal.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
}

#[derive(Debug, Clone, Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Build a flat-shaded mesh from raw triangle-soup positions + indices
    /// (the payload of `da_graph::Shape::Mesh`). Each triangle gets three
    /// duplicated vertices carrying the face normal — the same faceted look
    /// as [`unit_box`], which is right for CSG solids (hard edges must stay
    /// hard; smoothing across a boolean seam is the classic artifact).
    ///
    /// Robustness: indices out of range and degenerate (zero-area)
    /// triangles are skipped. Pure function of its inputs — deterministic.
    pub fn from_positions_indices(positions: &[glam::Vec3], indices: &[u32]) -> MeshData {
        let mut m = MeshData::default();
        for tri in indices.chunks_exact(3) {
            let (ia, ib, ic) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (Some(a), Some(b), Some(c)) =
                (positions.get(ia), positions.get(ib), positions.get(ic))
            else {
                continue;
            };
            let n = (*b - *a).cross(*c - *a);
            if n.length_squared() <= 1e-12 {
                continue;
            }
            let normal = n.normalize().to_array();
            let base = m.vertices.len() as u32;
            for p in [a, b, c] {
                m.vertices.push(Vertex { pos: p.to_array(), normal });
            }
            m.indices.extend_from_slice(&[base, base + 1, base + 2]);
        }
        m
    }

    fn push_quad(&mut self, corners: [[f32; 3]; 4], normal: [f32; 3]) {
        let base = self.vertices.len() as u32;
        for pos in corners {
            self.vertices.push(Vertex { pos, normal });
        }
        self.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

/// Unit cube: [-1, 1]³ (scaled by half-extents per instance).
pub fn unit_box() -> MeshData {
    let mut m = MeshData::default();
    // +X, -X, +Y, -Y, +Z, -Z
    m.push_quad(
        [[1., -1., -1.], [1., 1., -1.], [1., 1., 1.], [1., -1., 1.]],
        [1., 0., 0.],
    );
    m.push_quad(
        [[-1., -1., 1.], [-1., 1., 1.], [-1., 1., -1.], [-1., -1., -1.]],
        [-1., 0., 0.],
    );
    m.push_quad(
        [[-1., 1., -1.], [-1., 1., 1.], [1., 1., 1.], [1., 1., -1.]],
        [0., 1., 0.],
    );
    m.push_quad(
        [[-1., -1., 1.], [-1., -1., -1.], [1., -1., -1.], [1., -1., 1.]],
        [0., -1., 0.],
    );
    m.push_quad(
        [[-1., -1., 1.], [1., -1., 1.], [1., 1., 1.], [-1., 1., 1.]],
        [0., 0., 1.],
    );
    m.push_quad(
        [[1., -1., -1.], [-1., -1., -1.], [-1., 1., -1.], [1., 1., -1.]],
        [0., 0., -1.],
    );
    m
}

/// Unit Y-axis cylinder: radius 1, y ∈ [0, 1] (scaled radius/height per instance).
pub fn unit_cylinder(segments: u32) -> MeshData {
    let mut m = MeshData::default();
    let n = segments.max(3);
    // Side wall.
    for i in 0..n {
        let (a0, a1) = (
            std::f32::consts::TAU * i as f32 / n as f32,
            std::f32::consts::TAU * (i + 1) as f32 / n as f32,
        );
        let (x0, z0, x1, z1) = (a0.cos(), a0.sin(), a1.cos(), a1.sin());
        let base = m.vertices.len() as u32;
        m.vertices.push(Vertex { pos: [x0, 0., z0], normal: [x0, 0., z0] });
        m.vertices.push(Vertex { pos: [x1, 0., z1], normal: [x1, 0., z1] });
        m.vertices.push(Vertex { pos: [x1, 1., z1], normal: [x1, 0., z1] });
        m.vertices.push(Vertex { pos: [x0, 1., z0], normal: [x0, 0., z0] });
        m.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    // Caps (fan).
    for (y, ny) in [(0.0f32, -1.0f32), (1.0, 1.0)] {
        let center = m.vertices.len() as u32;
        m.vertices.push(Vertex { pos: [0., y, 0.], normal: [0., ny, 0.] });
        let ring0 = m.vertices.len() as u32;
        for i in 0..n {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            m.vertices.push(Vertex { pos: [a.cos(), y, a.sin()], normal: [0., ny, 0.] });
        }
        for i in 0..n {
            let (i0, i1) = (ring0 + i, ring0 + (i + 1) % n);
            if ny > 0.0 {
                m.indices.extend_from_slice(&[center, i0, i1]);
            } else {
                m.indices.extend_from_slice(&[center, i1, i0]);
            }
        }
    }
    m
}

/// Unit UV sphere, radius 1.
pub fn unit_sphere(stacks: u32, slices: u32) -> MeshData {
    let mut m = MeshData::default();
    let (st, sl) = (stacks.max(3), slices.max(3));
    for i in 0..=st {
        let v = std::f32::consts::PI * i as f32 / st as f32;
        for j in 0..=sl {
            let u = std::f32::consts::TAU * j as f32 / sl as f32;
            let p = [v.sin() * u.cos(), v.cos(), v.sin() * u.sin()];
            m.vertices.push(Vertex { pos: p, normal: p });
        }
    }
    let row = sl + 1;
    for i in 0..st {
        for j in 0..sl {
            let a = i * row + j;
            let b = a + row;
            m.indices
                .extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    m
}

/// Ground patch: unit quad on y=0, [-1,1]², scaled per instance. Subdivided
/// so vertex-level effects have something to chew on later.
pub fn ground_patch(divs: u32) -> MeshData {
    let mut m = MeshData::default();
    let n = divs.max(1);
    for i in 0..=n {
        for j in 0..=n {
            let (x, z) = (
                -1.0 + 2.0 * i as f32 / n as f32,
                -1.0 + 2.0 * j as f32 / n as f32,
            );
            m.vertices.push(Vertex { pos: [x, 0., z], normal: [0., 1., 0.] });
        }
    }
    let row = n + 1;
    for i in 0..n {
        for j in 0..n {
            let a = i * row + j;
            let b = a + row;
            m.indices
                .extend_from_slice(&[a, a + 1, b, b, a + 1, b + 1]);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(m: &MeshData) {
        assert!(!m.vertices.is_empty());
        assert_eq!(m.indices.len() % 3, 0);
        for &i in &m.indices {
            assert!((i as usize) < m.vertices.len());
        }
    }

    #[test]
    fn primitives_are_well_formed() {
        check(&unit_box());
        check(&unit_cylinder(16));
        check(&unit_sphere(12, 18));
        check(&ground_patch(8));
        assert_eq!(unit_box().indices.len(), 36);
    }

    /// Regular tetrahedron centered on the origin, outward winding.
    fn tetra() -> (Vec<glam::Vec3>, Vec<u32>) {
        let v = vec![
            glam::Vec3::new(1.0, 1.0, 1.0),
            glam::Vec3::new(1.0, -1.0, -1.0),
            glam::Vec3::new(-1.0, 1.0, -1.0),
            glam::Vec3::new(-1.0, -1.0, 1.0),
        ];
        let i = vec![0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2];
        (v, i)
    }

    #[test]
    fn from_positions_indices_flat_shades_with_outward_unit_normals() {
        let (v, i) = tetra();
        let m = MeshData::from_positions_indices(&v, &i);
        check(&m);
        // Flat shading: 3 duplicated vertices per triangle, sequential indices.
        assert_eq!(m.vertices.len(), 12);
        assert_eq!(m.indices, (0..12).collect::<Vec<u32>>());
        for tri in m.vertices.chunks_exact(3) {
            let n = glam::Vec3::from_array(tri[0].normal);
            // Unit length, shared by all three verts of the face.
            assert!((n.length() - 1.0).abs() < 1e-5, "unit normal: {n:?}");
            assert_eq!(tri[0].normal, tri[1].normal);
            assert_eq!(tri[1].normal, tri[2].normal);
            // Outward: the solid is centered on the origin, so the face
            // normal must point away from it (this is also what guarantees
            // the winding survives backface culling).
            let centroid = (glam::Vec3::from_array(tri[0].pos)
                + glam::Vec3::from_array(tri[1].pos)
                + glam::Vec3::from_array(tri[2].pos))
                / 3.0;
            assert!(n.dot(centroid) > 0.0, "outward normal: {n:?} at {centroid:?}");
        }
    }

    #[test]
    fn from_positions_indices_skips_degenerate_and_out_of_range() {
        let (v, mut i) = tetra();
        i.extend_from_slice(&[0, 0, 1]); // zero-area sliver
        i.extend_from_slice(&[0, 1, 99]); // index out of range
        i.push(2); // trailing non-triangle remainder
        let m = MeshData::from_positions_indices(&v, &i);
        assert_eq!(m.vertices.len(), 12, "only the 4 real faces survive");
        assert_eq!(m.indices.len(), 12);
    }

    #[test]
    fn sphere_vertices_on_unit_radius() {
        for v in unit_sphere(8, 12).vertices {
            let r = (v.pos[0].powi(2) + v.pos[1].powi(2) + v.pos[2].powi(2)).sqrt();
            assert!((r - 1.0).abs() < 1e-4);
        }
    }
}
