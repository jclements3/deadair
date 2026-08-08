//! Constructive Solid Geometry: primitives, sketches, boolean ops, transforms,
//! and export to a renderable mesh. This is the reliable core the Nim-style DSL
//! evaluates onto.

pub mod bsp;
pub mod ops;
pub mod primitives;
pub mod sketch;

use bsp::{v3, Polygon, Vertex, V3};
use glam::Vec3;

/// A triangulated CPU mesh: fan-triangulated positions with smooth per-vertex
/// normals and `u32` triangle indices. Stands in for vali's `three_d::CpuMesh`
/// at the export boundary (the only external type the vendored code used).
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

/// Default part temperature (K) — cold-soaked ambient until aeroheating kicks in.
pub const DEFAULT_TEMP: f32 = 300.0;

/// One distinguishable piece of a solid: the primitive it came from, and the
/// temperature it currently glows at. Booleans concatenate these tables; the
/// polygons carry a `part` index into it (see [`bsp::Polygon::part`]).
#[derive(Clone, Debug)]
pub struct Part {
    pub name: String,
    pub temp: f32,
}

/// A solid = a soup of convex, consistently-wound polygons, plus a side table
/// naming each distinct part the polygons' `part` tags index into.
#[derive(Clone, Debug)]
pub struct Solid {
    pub polys: Vec<Polygon>,
    pub parts: Vec<Part>,
}

impl Solid {
    /// Wrap a fresh primitive's polygon soup as a single part. The primitive's
    /// polygons are all tagged part 0 (the default from `Polygon::new`).
    fn single(polys: Vec<Polygon>, name: &str) -> Solid {
        Solid {
            polys,
            parts: vec![Part {
                name: name.to_string(),
                temp: DEFAULT_TEMP,
            }],
        }
    }

    // --- primitives -------------------------------------------------------
    pub fn cube(w: f64, d: f64, h: f64) -> Solid {
        Solid::single(primitives::cube(w, d, h), "cube")
    }
    pub fn cylinder(r: f64, h: f64, seg: usize) -> Solid {
        Solid::single(primitives::cylinder(r, h, seg), "cylinder")
    }
    pub fn cone(r: f64, h: f64, seg: usize) -> Solid {
        Solid::single(primitives::cone(r, h, seg), "cone")
    }
    pub fn frustum(r1: f64, r2: f64, h: f64, seg: usize) -> Solid {
        Solid::single(primitives::frustum(r1, r2, h, seg), "frustum")
    }
    pub fn sphere(r: f64, seg: usize) -> Solid {
        Solid::single(primitives::sphere(r, seg), "sphere")
    }
    pub fn torus(rr: f64, r: f64, sj: usize, sn: usize) -> Solid {
        Solid::single(primitives::torus(rr, r, sj, sn), "torus")
    }
    pub fn wedge(w: f64, d: f64, h: f64) -> Solid {
        Solid::single(primitives::wedge(w, d, h), "wedge")
    }
    pub fn pyramid(w: f64, d: f64, h: f64) -> Solid {
        Solid::single(primitives::pyramid(w, d, h), "pyramid")
    }
    pub fn extrude(s: &sketch::Sketch, h: f64) -> Solid {
        Solid::single(sketch::extrude(s, h), "extrude")
    }
    /// Extrude with a progressive twist of `deg` degrees over the height. See
    /// [`sketch::extrude_twist`].
    pub fn extrude_twist(s: &sketch::Sketch, h: f64, deg: f64, steps: usize) -> Solid {
        Solid::single(sketch::extrude_twist(s, h, deg, steps), "extrude")
    }
    pub fn revolve(s: &sketch::Sketch, seg: usize) -> Solid {
        Solid::single(sketch::revolve(s, seg), "revolve")
    }
    /// Partial revolve through `deg` degrees with flat end caps (pipe elbows,
    /// sectors). See [`sketch::revolve_arc`].
    pub fn revolve_arc(s: &sketch::Sketch, seg: usize, deg: f64) -> Solid {
        Solid::single(sketch::revolve_arc(s, seg, deg), "revolve")
    }
    /// Revolve a `(radius, height)` profile like [`revolve`], but auto-orient the
    /// result outward: if the given point order winds the surface inward
    /// (negative signed volume), the profile is reversed so normals face out and
    /// the solid is boolean-usable. Intended for open lathe profiles (e.g. a
    /// bezier missile hull) whose winding is easy to get backwards.
    pub fn lathe(s: &sketch::Sketch, seg: usize) -> Solid {
        let solid = Solid::single(sketch::revolve(s, seg), "lathe");
        if solid.volume() >= 0.0 {
            solid
        } else {
            let rev = sketch::Sketch::polygon(s.points.iter().rev().cloned().collect());
            Solid::single(sketch::revolve(&rev, seg), "lathe")
        }
    }
    /// Skin a solid through 2+ cross-sections stacked along Z (centered on the
    /// origin like [`extrude`](Self::extrude)). Sections are resampled to a
    /// common vertex count, rolled apex-to-apex, and DTW twist-aligned, then
    /// skinned; `closed_caps` seals the ends for a watertight solid. See
    /// [`sketch::loft`].
    pub fn loft(sections: &[sketch::Sketch], h: f64, closed_caps: bool) -> Solid {
        Solid::single(sketch::loft(sections, h, closed_caps), "loft")
    }
    /// Sweep a cross-section along a planar `(x, z)` spine with a
    /// rotation-minimizing frame (minimal twist). `closed_caps` seals the ends
    /// for a watertight solid. See [`sketch::sweep`].
    pub fn sweep(section: &sketch::Sketch, path: &[(f64, f64)], closed_caps: bool) -> Solid {
        Solid::single(sketch::sweep(section, path, closed_caps), "sweep")
    }
    /// Sweep a cross-section along an arbitrary 3-D spine (e.g. a helix) with a
    /// rotation-minimizing frame. See [`sketch::sweep_path`].
    pub fn sweep_path(section: &sketch::Sketch, path: &[V3], closed_caps: bool) -> Solid {
        Solid::single(sketch::sweep_path(section, path, closed_caps), "sweep")
    }

    // --- booleans ---------------------------------------------------------
    // Every boolean concatenates the two parts tables: the right operand's
    // polygon part ids are offset by `self.parts.len()` so both tables' entries
    // stay addressable in the merged solid. Split fragments keep their tag (see
    // `bsp::Plane::split_polygon`), so a part survives even when clipped.
    pub fn union(self, other: Solid) -> Solid {
        let (polys_a, parts, polys_b) = self.merge_prep(other);
        Solid { polys: bsp::union(polys_a, polys_b), parts }
    }
    pub fn difference(self, other: Solid) -> Solid {
        let (polys_a, parts, polys_b) = self.merge_prep(other);
        Solid { polys: bsp::difference(polys_a, polys_b), parts }
    }
    pub fn intersection(self, other: Solid) -> Solid {
        let (polys_a, parts, polys_b) = self.merge_prep(other);
        Solid { polys: bsp::intersection(polys_a, polys_b), parts }
    }

    /// Shared boolean setup: offset the right operand's part tags past the left
    /// operand's, and return (left polys, merged parts table, right polys).
    fn merge_prep(self, other: Solid) -> (Vec<Polygon>, Vec<Part>, Vec<Polygon>) {
        let offset = self.parts.len() as u32;
        let mut polys_b = other.polys;
        for p in &mut polys_b {
            p.part += offset;
        }
        let mut parts = self.parts;
        parts.extend(other.parts);
        (self.polys, parts, polys_b)
    }

    // --- edge beveling ----------------------------------------------------
    /// Round the sharp convex edges of this solid with the given `radius`,
    /// approximating each arc with `seg` flat facets. See [`ops::fillet`].
    pub fn fillet(&self, r: f64, seg: usize) -> Result<Solid, String> {
        ops::fillet(self, r, seg)
    }
    /// Cut a flat of `distance` off the sharp convex edges. See [`ops::chamfer`].
    pub fn chamfer(&self, d: f64) -> Result<Solid, String> {
        ops::chamfer(self, d)
    }

    // --- transforms -------------------------------------------------------
    fn map(self, fpos: impl Fn(V3) -> V3, fnrm: impl Fn(V3) -> V3) -> Solid {
        let parts = self.parts;
        let polys = self
            .polys
            .into_iter()
            .map(|p| {
                let vs = p
                    .vertices
                    .iter()
                    .map(|v| Vertex::new(fpos(v.pos), fnrm(v.normal).normalized()))
                    .collect();
                // Rebuilding the polygon must not drop its part identity.
                Polygon::with_part(vs, p.part)
            })
            .collect();
        Solid { polys, parts }
    }

    pub fn translate(self, x: f64, y: f64, z: f64) -> Solid {
        self.map(move |p| v3(p.x + x, p.y + y, p.z + z), |n| n)
    }

    pub fn scale(self, sx: f64, sy: f64, sz: f64) -> Solid {
        // Normals use the inverse-transpose of a diagonal scale.
        self.map(
            move |p| v3(p.x * sx, p.y * sy, p.z * sz),
            move |n| v3(n.x / sx, n.y / sy, n.z / sz),
        )
    }

    /// Rotate `deg` degrees about `axis` (Rodrigues' rotation).
    pub fn rotate(self, axis: V3, deg: f64) -> Solid {
        let a = axis.normalized();
        let th = deg.to_radians();
        let (s, c) = (th.sin(), th.cos());
        let rot = move |p: V3| {
            // p*cos + (a×p)*sin + a*(a·p)*(1-cos)
            p.mul(c)
                .add(a.cross(p).mul(s))
                .add(a.mul(a.dot(p) * (1.0 - c)))
        };
        self.map(rot, rot)
    }

    /// Reflect across a coordinate plane (negate the chosen axes). A reflection
    /// reverses orientation, so for an odd number of negated axes we also reverse
    /// each polygon's winding to keep it outward-wound (watertight).
    pub fn reflect(self, nx: bool, ny: bool, nz: bool) -> Solid {
        let (sx, sy, sz) = (
            if nx { -1.0 } else { 1.0 },
            if ny { -1.0 } else { 1.0 },
            if nz { -1.0 } else { 1.0 },
        );
        let flip = (nx as u8 + ny as u8 + nz as u8) % 2 == 1;
        let parts = self.parts;
        let polys = self
            .polys
            .into_iter()
            .map(|p| {
                let mut vs: Vec<Vertex> = p
                    .vertices
                    .iter()
                    .map(|v| {
                        Vertex::new(
                            v3(v.pos.x * sx, v.pos.y * sy, v.pos.z * sz),
                            v3(v.normal.x * sx, v.normal.y * sy, v.normal.z * sz).normalized(),
                        )
                    })
                    .collect();
                if flip {
                    vs.reverse();
                }
                Polygon::with_part(vs, p.part)
            })
            .collect();
        Solid { polys, parts }
    }

    /// Mirror-symmetrize: union this solid with its reflection across the chosen
    /// coordinate plane (Blender's Mirror modifier). Model half a shape, then
    /// `.mirror("x")` to complete it.
    pub fn mirror(self, nx: bool, ny: bool, nz: bool) -> Solid {
        let refl = self.clone().reflect(nx, ny, nz);
        self.union(refl)
    }

    /// Linear array: union `n` copies, each offset by an added `(dx, dy, dz)`.
    pub fn array(self, n: usize, dx: f64, dy: f64, dz: f64) -> Solid {
        let n = n.max(1);
        let mut out = self.clone();
        for k in 1..n {
            let f = k as f64;
            out = out.union(self.clone().translate(dx * f, dy * f, dz * f));
        }
        out
    }

    /// Circular (polar) array: union `n` copies evenly rotated a full turn about
    /// `axis` (through the origin).
    pub fn polar(self, n: usize, axis: V3) -> Solid {
        let n = n.max(1);
        let mut out = self.clone();
        for k in 1..n {
            let deg = 360.0 * k as f64 / n as f64;
            out = out.union(self.clone().rotate(axis, deg));
        }
        out
    }

    // --- analysis ---------------------------------------------------------
    /// Signed volume via the divergence theorem (sum of tetra volumes). Positive
    /// for outward-wound closed solids — a good watertightness/orientation check.
    pub fn volume(&self) -> f64 {
        let mut vol = 0.0;
        for p in &self.polys {
            let o = p.vertices[0].pos;
            for i in 1..p.vertices.len() - 1 {
                let a = p.vertices[i].pos.sub(o);
                let b = p.vertices[i + 1].pos.sub(o);
                vol += o.dot(a.cross(b));
            }
        }
        vol / 6.0
    }

    pub fn triangle_count(&self) -> usize {
        self.polys.iter().map(|p| p.vertices.len() - 2).sum()
    }

    /// Axis-aligned bounding box as (min, max) in f32, for camera framing.
    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in &self.polys {
            for v in &p.vertices {
                for (k, c) in [v.pos.x, v.pos.y, v.pos.z].into_iter().enumerate() {
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
        }
        if !lo[0].is_finite() {
            return (Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0));
        }
        (
            Vec3::new(lo[0] as f32, lo[1] as f32, lo[2] as f32),
            Vec3::new(hi[0] as f32, hi[1] as f32, hi[2] as f32),
        )
    }

    // --- render -----------------------------------------------------------
    /// Fan-triangulate into a CPU mesh with smooth per-vertex normals.
    pub fn to_cpu_mesh(&self) -> Mesh {
        let mut positions: Vec<Vec3> = Vec::new();
        let mut normals: Vec<Vec3> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for p in &self.polys {
            let base = positions.len() as u32;
            for v in &p.vertices {
                positions.push(Vec3::new(v.pos.x as f32, v.pos.y as f32, v.pos.z as f32));
                normals.push(Vec3::new(
                    v.normal.x as f32,
                    v.normal.y as f32,
                    v.normal.z as f32,
                ));
            }
            for i in 1..p.vertices.len() as u32 - 1 {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
        Mesh { positions, normals, indices }
    }

    // --- darkair mesh bridge (addition over the vali source) ---------------
    /// Triangulated `(positions, indices)` in vali's native **Z-up** frame.
    /// Same fan triangulation as [`to_cpu_mesh`](Self::to_cpu_mesh); f64 → f32
    /// happens only here.
    pub fn to_mesh_zup(&self) -> (Vec<Vec3>, Vec<u32>) {
        let mut positions: Vec<Vec3> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for p in &self.polys {
            let base = positions.len() as u32;
            for v in &p.vertices {
                positions.push(Vec3::new(v.pos.x as f32, v.pos.y as f32, v.pos.z as f32));
            }
            for i in 1..p.vertices.len() as u32 - 1 {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
        (positions, indices)
    }

    /// Triangulated `(positions, indices)` converted to darkair's **Y-up**
    /// frame: vali Z-up `(x, y, z)` → Y-up `(x, z, -y)`. This is a proper
    /// rotation (det +1), so triangle winding is preserved and the mesh stays
    /// outward-wound. The axis swap is done in f64 and cast to f32 only at
    /// this boundary.
    pub fn to_mesh_yup(&self) -> (Vec<Vec3>, Vec<u32>) {
        let mut positions: Vec<Vec3> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        for p in &self.polys {
            let base = positions.len() as u32;
            for v in &p.vertices {
                positions.push(Vec3::new(v.pos.x as f32, v.pos.z as f32, (-v.pos.y) as f32));
            }
            for i in 1..p.vertices.len() as u32 - 1 {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
        (positions, indices)
    }

    /// Like [`to_mesh_yup`](Self::to_mesh_yup), but split into one mesh per
    /// part **name**: parts sharing a name (e.g. the copies a `.polar` array
    /// makes) merge into one mesh. Buckets appear in first-part-id order and
    /// empty parts (clipped away by booleans) are omitted, so the result is
    /// deterministic for a given solid. Additive darkair helper — lets a
    /// caller attach a distinct material/thermal state per named part.
    pub fn to_meshes_yup_by_part(&self) -> Vec<(String, Vec<Vec3>, Vec<u32>)> {
        // Bucket index per part id, grouping ids that share a name.
        let mut buckets: Vec<(String, Vec<Vec3>, Vec<u32>)> = Vec::new();
        let mut bucket_of: Vec<usize> = Vec::with_capacity(self.parts.len());
        for part in &self.parts {
            let idx = buckets
                .iter()
                .position(|(name, _, _)| *name == part.name)
                .unwrap_or_else(|| {
                    buckets.push((part.name.clone(), Vec::new(), Vec::new()));
                    buckets.len() - 1
                });
            bucket_of.push(idx);
        }
        for p in &self.polys {
            let (_, positions, indices) = &mut buckets[bucket_of[p.part as usize]];
            let base = positions.len() as u32;
            for v in &p.vertices {
                positions.push(Vec3::new(v.pos.x as f32, v.pos.z as f32, (-v.pos.y) as f32));
            }
            for i in 1..p.vertices.len() as u32 - 1 {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
        buckets.retain(|(_, positions, _)| !positions.is_empty());
        buckets
    }

    /// Mean vertex position of each part, as `(part_id, centroid)`. Parts with
    /// no surviving polygons (e.g. entirely clipped away by a boolean) are
    /// omitted. Used to sample a per-part aeroheating temperature.
    pub fn part_centroids(&self) -> Vec<(u32, Vec3)> {
        let n = self.parts.len();
        let mut sums = vec![(v3(0.0, 0.0, 0.0), 0usize); n];
        for p in &self.polys {
            let e = &mut sums[p.part as usize];
            for v in &p.vertices {
                e.0 = e.0.add(v.pos);
                e.1 += 1;
            }
        }
        sums.into_iter()
            .enumerate()
            .filter(|(_, (_, c))| *c > 0)
            .map(|(id, (s, c))| {
                let m = s.mul(1.0 / c as f64);
                (id as u32, Vec3::new(m.x as f32, m.y as f32, m.z as f32))
            })
            .collect()
    }

    /// Like [`to_cpu_mesh`](Self::to_cpu_mesh) but also returns a per-VERTEX
    /// part id (parallel to the mesh positions), so a caller can color each
    /// vertex by the temperature of the part it belongs to.
    pub fn to_cpu_mesh_parted(&self) -> (Mesh, Vec<u32>) {
        let mut positions: Vec<Vec3> = Vec::new();
        let mut normals: Vec<Vec3> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut vparts: Vec<u32> = Vec::new();
        for p in &self.polys {
            let base = positions.len() as u32;
            for v in &p.vertices {
                positions.push(Vec3::new(v.pos.x as f32, v.pos.y as f32, v.pos.z as f32));
                normals.push(Vec3::new(
                    v.normal.x as f32,
                    v.normal.y as f32,
                    v.normal.z as f32,
                ));
                vparts.push(p.part);
            }
            for i in 1..p.vertices.len() as u32 - 1 {
                indices.extend_from_slice(&[base, base + i, base + i + 1]);
            }
        }
        let mesh = Mesh { positions, normals, indices };
        (mesh, vparts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    // Volumes should match analytic formulas within tessellation error.
    #[test]
    fn primitive_volumes() {
        assert!((Solid::cube(2.0, 2.0, 2.0).volume() - 8.0).abs() < 1e-6);

        let cyl = Solid::cylinder(1.0, 2.0, 256).volume();
        assert!((cyl - PI * 1.0 * 2.0).abs() < 0.02, "cyl vol {cyl}");

        let sph = Solid::sphere(1.0, 128).volume();
        assert!((sph - 4.0 / 3.0 * PI).abs() < 0.02, "sphere vol {sph}");

        let con = Solid::cone(1.0, 3.0, 256).volume();
        assert!((con - PI * 1.0 * 3.0 / 3.0).abs() < 0.02, "cone vol {con}");
    }

    #[test]
    fn boolean_difference_removes_volume() {
        // Drill a unit-radius hole through a 4×4×4 block.
        let block = Solid::cube(4.0, 4.0, 4.0);
        let drill = Solid::cylinder(1.0, 6.0, 128);
        let holed = block.difference(drill);
        let expected = 64.0 - PI * 1.0 * 4.0;
        assert!(
            (holed.volume() - expected).abs() < 0.1,
            "holed vol {} vs {}",
            holed.volume(),
            expected
        );
    }

    #[test]
    fn union_of_disjoint_adds_volume() {
        let a = Solid::cube(2.0, 2.0, 2.0);
        let b = Solid::cube(2.0, 2.0, 2.0).translate(10.0, 0.0, 0.0);
        let u = a.union(b);
        assert!((u.volume() - 16.0).abs() < 1e-4, "union vol {}", u.volume());
    }

    #[test]
    fn intersection_of_offset_cubes() {
        // Two unit cubes offset by 1 in x overlap in a 1×2×2 = 4 volume.
        let a = Solid::cube(2.0, 2.0, 2.0);
        let b = Solid::cube(2.0, 2.0, 2.0).translate(1.0, 0.0, 0.0);
        let i = a.intersection(b);
        assert!((i.volume() - 4.0).abs() < 1e-4, "isect vol {}", i.volume());
    }

    // --- part identity ----------------------------------------------------
    /// Collect the distinct part ids actually present on a solid's polygons.
    fn distinct_parts(s: &Solid) -> std::collections::BTreeSet<u32> {
        s.polys.iter().map(|p| p.part).collect()
    }

    #[test]
    fn primitive_has_one_part() {
        let c = Solid::cube(2.0, 2.0, 2.0);
        assert_eq!(c.parts.len(), 1);
        assert_eq!(c.parts[0].name, "cube");
        assert_eq!(distinct_parts(&c), [0].into_iter().collect());
        assert!((c.parts[0].temp - DEFAULT_TEMP).abs() < 1e-6);
    }

    #[test]
    fn boolean_yields_distinct_surviving_parts() {
        // A union of two disjoint primitives keeps both identities: two entries
        // in the parts table and both ids present on the resulting polygons.
        let a = Solid::cube(2.0, 2.0, 2.0);
        let b = Solid::cylinder(0.6, 2.0, 32).translate(10.0, 0.0, 0.0);
        let u = a.union(b);
        assert_eq!(u.parts.len(), 2, "parts tables must concatenate");
        assert_eq!(u.parts[0].name, "cube");
        assert_eq!(u.parts[1].name, "cylinder");
        let ids = distinct_parts(&u);
        assert!(ids.len() >= 2, "expected >=2 surviving part ids, got {ids:?}");
        assert!(ids.contains(&0) && ids.contains(&1), "both parts must survive: {ids:?}");
    }

    #[test]
    fn wedge_is_outward_wound_and_unions_cleanly() {
        // The wedge was once emitted inside-out (all face normals pointing
        // into the solid); a union with it then classified the other operand
        // as interior and discarded it. Signed volume is the oracle: positive
        // means outward winding. wedge(w,d,h) is half a w*d*h box.
        let w = Solid::wedge(2.0, 3.0, 1.0);
        let vol = w.volume();
        assert!((vol - 3.0).abs() < 1e-9, "wedge volume must be +w*d*h/2, got {vol}");
        // Union with a disjoint box must keep both operands.
        let b = Solid::cube(1.0, 1.0, 1.0).translate(10.0, 0.0, 0.0);
        let u = w.union(b);
        let ids = distinct_parts(&u);
        assert!(ids.contains(&0) && ids.contains(&1), "wedge union ate an operand: {ids:?}");
    }

    #[test]
    fn difference_keeps_right_operand_offset() {
        // Drilling a hole leaves the drill's cut wall tagged with the offset id,
        // so the bore surface can glow differently from the block.
        let block = Solid::cube(4.0, 4.0, 4.0);
        let drill = Solid::cylinder(1.0, 6.0, 48);
        let holed = block.difference(drill);
        assert_eq!(holed.parts.len(), 2);
        let ids = distinct_parts(&holed);
        assert!(ids.contains(&0), "block surface must remain (part 0)");
        assert!(ids.contains(&1), "bore wall must carry the drill's offset id (part 1)");
    }

    #[test]
    fn transform_preserves_part_ids() {
        let u = Solid::cube(2.0, 2.0, 2.0)
            .union(Solid::cube(2.0, 2.0, 2.0).translate(10.0, 0.0, 0.0));
        let before = distinct_parts(&u);
        let moved = u.translate(1.0, 2.0, 3.0).rotate(v3(0.0, 0.0, 1.0), 30.0);
        assert_eq!(before, distinct_parts(&moved), "transforms must not disturb part ids");
        assert_eq!(moved.parts.len(), 2, "parts table must survive transforms");
    }

    #[test]
    fn parted_mesh_has_one_part_per_vertex() {
        let solid = Solid::cube(2.0, 2.0, 2.0)
            .union(Solid::cube(2.0, 2.0, 2.0).translate(10.0, 0.0, 0.0));
        let (mesh, vparts) = solid.to_cpu_mesh_parted();
        let np = mesh.positions.len();
        assert_eq!(np, vparts.len(), "one part id per vertex/position");
        // Every emitted id must be a valid index into the parts table.
        assert!(vparts.iter().all(|&id| (id as usize) < solid.parts.len()));
        // Both parts should color at least one vertex.
        assert!(vparts.contains(&0) && vparts.contains(&1));
    }

    #[test]
    fn part_centroids_separate_disjoint_parts() {
        // Two cubes 10 apart in x: their part centroids must land near each hub.
        let solid = Solid::cube(2.0, 2.0, 2.0)
            .union(Solid::cube(2.0, 2.0, 2.0).translate(10.0, 0.0, 0.0));
        let cs = solid.part_centroids();
        assert_eq!(cs.len(), 2);
        let get = |id: u32| cs.iter().find(|(i, _)| *i == id).unwrap().1;
        assert!(get(0).x.abs() < 0.5, "part 0 centroid near x=0");
        assert!((get(1).x - 10.0).abs() < 0.5, "part 1 centroid near x=10");
    }

    #[test]
    fn fillet_does_not_lose_the_base_part() {
        // Beveling adds throwaway cut parts but must keep the original part 0.
        let out = Solid::cube(2.0, 2.0, 2.0).fillet(0.3, 4).expect("fillet ok");
        assert!(distinct_parts(&out).contains(&0), "work solid's part must survive");
    }

    #[test]
    fn revolve_makes_a_torus() {
        // Revolving a small circle offset from the axis approximates a torus:
        // V = 2π²·R·r².
        let circle = sketch::Sketch::circle(0.5, 64).translate2d(2.0, 0.0);
        let sol = Solid::revolve(&circle, 128);
        let expected = 2.0 * PI * PI * 2.0 * 0.5 * 0.5;
        assert!(
            (sol.volume() - expected).abs() < 0.05,
            "revolved torus vol {} vs {}",
            sol.volume(),
            expected
        );
    }

    #[test]
    fn loft_between_equal_circles_is_a_cylinder() {
        // Two identical circle sections skin to a cylinder: V = π r² h.
        let c = sketch::Sketch::circle(3.0, 256);
        let sol = Solid::loft(&[c.clone(), c], 10.0, true);
        let want = PI * 9.0 * 10.0;
        assert!(
            (sol.volume() - want).abs() < want * 0.005,
            "loft cylinder vol {} vs {}",
            sol.volume(),
            want
        );
    }

    #[test]
    fn loft_between_unequal_circles_is_a_frustum() {
        // Circles of radius 2 and 4 over height 9: V = (π h / 3)(r1² + r1 r2 + r2²).
        let sol = Solid::loft(
            &[sketch::Sketch::circle(2.0, 256), sketch::Sketch::circle(4.0, 256)],
            9.0,
            true,
        );
        let want = PI * 9.0 / 3.0 * (4.0 + 8.0 + 16.0);
        assert!(
            (sol.volume() - want).abs() < want * 0.01,
            "loft frustum vol {} vs {}",
            sol.volume(),
            want
        );
    }

    #[test]
    fn three_section_loft_is_watertight_and_matches_stacked_frustums() {
        // circle(2) -> circle(3) -> circle(2) over total height 10 is two
        // back-to-back frustums of height 5 each. Watertight => positive volume.
        let sol = Solid::loft(
            &[
                sketch::Sketch::circle(2.0, 256),
                sketch::Sketch::circle(3.0, 256),
                sketch::Sketch::circle(2.0, 256),
            ],
            10.0,
            true,
        );
        let frustum = PI * 5.0 / 3.0 * (4.0 + 6.0 + 9.0);
        let want = 2.0 * frustum;
        assert!(sol.volume() > 0.0, "3-section loft must be positive");
        assert!(
            (sol.volume() - want).abs() < want * 0.02,
            "3-section loft vol {} vs {}",
            sol.volume(),
            want
        );
    }

    #[test]
    fn sweep_straight_path_is_a_cylinder() {
        // A circle swept along a straight vertical (x, z) spine of length 10 is a
        // cylinder: V = π r² L.
        let path: Vec<(f64, f64)> = (0..=10).map(|i| (0.0, -5.0 + i as f64)).collect();
        let sol = Solid::sweep(&sketch::Sketch::circle(2.0, 128), &path, true);
        let want = PI * 4.0 * 10.0;
        assert!(
            (sol.volume() - want).abs() < want * 0.01,
            "swept cylinder vol {} vs {}",
            sol.volume(),
            want
        );
    }

    #[test]
    fn sweep_bent_path_is_watertight() {
        // A quarter-circle-ish bent spine: the tube must close (positive volume).
        let path: Vec<(f64, f64)> = (0..=20)
            .map(|i| {
                let a = i as f64 / 20.0 * std::f64::consts::FRAC_PI_2;
                (10.0 * (1.0 - a.cos()), 10.0 * a.sin())
            })
            .collect();
        let sol = Solid::sweep(&sketch::Sketch::circle(1.0, 64), &path, true);
        assert!(sol.volume() > 0.0, "bent sweep must be positive, got {}", sol.volume());
    }
}
