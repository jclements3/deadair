//! # da-csg — vendored vali CSG kernel + `.vim` modeling DSL
//!
//! This crate vendors the pure-Rust BSP boolean CSG kernel (`src/csg/`), the
//! Nim-flavored `.vim` modeling language (`src/dsl/`), the ISO 128 technical
//! drawing generator (`src/drawing/`), and the STL/OBJ mesh exporters
//! (`src/export.rs`) from the sibling **vali** repo
//! (`/home/james.clements/projects/vali`), with zero windowing/GPU
//! dependencies. The only adaptations from the vali source are the mesh-export
//! boundary — vali's `three_d::CpuMesh` is replaced by [`csg::Mesh`] (glam-based),
//! plus the added Y-up bridges [`csg::Solid::to_mesh_yup`] and
//! [`csg::Solid::to_meshes_yup_by_part`] — a HashMap→BTreeMap swap in
//! `drawing::build_edges` so SVG element order is deterministic, and one DSL
//! addition: a `let` that binds a solid stamps the bound name onto parts still
//! carrying kernel default names (`dsl::eval::run_program`), so `.vim` part
//! tags flow from `let` vocabulary. Everything else is kept diffable against
//! the vali source; do not reformat or "improve" the vendored logic.
//!
//! The authoritative primer on the `.vim` DSL — every builtin, sketch, and
//! operation, plus the vali → darkair mapping — is
//! `VALI_LOKI_OSG_DSL_PRIMER.md` at the darkair repo root.
//!
//! Axis conventions: vali/`.vim` is **Z-up** (round primitives along +Z);
//! darkair is **Y-up**. [`Solid::to_mesh_yup`] converts Z-up `(x, y, z)` to
//! Y-up `(x, z, -y)` — a proper rotation, so triangle winding is preserved.
//!
//! Determinism: compiling the same `.vim` source is a pure function of the
//! source text — no clocks, no `rand`, no ambient state — so the same script
//! always meshes to byte-identical vertex/index buffers.

pub mod csg;
pub mod drawing;
pub mod dsl;
pub mod export;

pub use csg::sketch::Sketch;
pub use csg::{Mesh, Part, Solid};
/// ISO 128 drawing entry points: orthographic multiview / isometric / section
/// projection of a [`Solid`] with hidden-line removal, rendered to SVG.
pub use drawing::{isometric, multiview, project, section, to_svg, Drawing, LineKind, Seg2, View, ViewDir};
pub use dsl::{compile_sdf, compile_sdf_scene, sdf_model_json, Compiled, StepLine};
/// Mesh exporters: binary/ASCII STL and Wavefront OBJ from a [`Solid`] (or any
/// raw triangle soup via the `*_from_triangles` variants in [`export`]).
pub use export::{obj, stl_ascii, stl_binary, triangles_of};

/// Compile `.vim` source into a [`Solid`] plus its build-step outline.
///
/// Thin wrapper over [`dsl::compile`]: lex → parse → evaluate onto the CSG
/// kernel. On failure returns a clear, actionable error `String` (vali's
/// "reliability over features" contract — there is no silently-broken-mesh
/// outcome).
pub fn compile_vim(source: &str) -> Result<Compiled, String> {
    dsl::compile(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signed volume of a triangulated (positions, indices) mesh via the
    /// divergence theorem — positive iff the triangles are outward-wound.
    fn mesh_volume(positions: &[glam::Vec3], indices: &[u32]) -> f64 {
        let mut vol = 0.0f64;
        for t in indices.chunks(3) {
            let a = positions[t[0] as usize].as_dvec3();
            let b = positions[t[1] as usize].as_dvec3();
            let c = positions[t[2] as usize].as_dvec3();
            vol += a.dot(b.cross(c));
        }
        vol / 6.0
    }

    #[test]
    fn cylinder_meshes_yup_with_height_along_y() {
        // A vali cylinder runs along +Z; in darkair's Y-up frame its height must
        // land on Y and its circular cross-section on X/Z.
        let solid = Solid::cylinder(2.0, 10.0, 48);
        let (pos, idx) = solid.to_mesh_yup();
        assert!(!pos.is_empty() && !idx.is_empty());
        assert_eq!(idx.len() % 3, 0, "index count must be whole triangles");
        assert!(idx.iter().all(|&i| (i as usize) < pos.len()), "indices in range");

        let mut lo = glam::Vec3::splat(f32::INFINITY);
        let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
        for p in &pos {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let ext = hi - lo;
        assert!((ext.y - 10.0).abs() < 1e-4, "height along Y, got extent {ext:?}");
        assert!((ext.x - 4.0).abs() < 1e-4, "diameter along X, got extent {ext:?}");
        assert!((ext.z - 4.0).abs() < 1e-4, "diameter along Z, got extent {ext:?}");
    }

    #[test]
    fn yup_conversion_preserves_winding() {
        // (x,y,z) -> (x,z,-y) is a proper rotation: the signed mesh volume must
        // stay positive (outward-wound) and match the solid's f64 volume.
        let solid = Solid::cylinder(2.0, 10.0, 96);
        let want = solid.volume();
        let (pos, idx) = solid.to_mesh_yup();
        let v = mesh_volume(&pos, &idx);
        assert!(v > 0.0, "y-up mesh must stay outward-wound, got {v}");
        assert!((v - want).abs() < want * 1e-3, "yup vol {v} vs solid vol {want}");
        let (pz, iz) = solid.to_mesh_zup();
        let vz = mesh_volume(&pz, &iz);
        assert!((vz - v).abs() < want * 1e-3, "zup {vz} and yup {v} volumes agree");
    }

    #[test]
    fn same_vim_source_meshes_byte_identical() {
        // Determinism is load-bearing: two independent compiles of the same
        // source must produce byte-identical vertex and index buffers.
        let src = "let b = box(4, 4, 4)\nlet d = cylinder(r = 1, h = 6)\nmodel b - d";
        let a = compile_vim(src).expect("first compile");
        let b = compile_vim(src).expect("second compile");
        let (pa, ia) = a.solid.to_mesh_yup();
        let (pb, ib) = b.solid.to_mesh_yup();
        assert_eq!(ia, ib, "index buffers must be identical");
        assert_eq!(pa.len(), pb.len(), "vertex counts must be identical");
        for (va, vb) in pa.iter().zip(&pb) {
            assert_eq!(va.x.to_bits(), vb.x.to_bits());
            assert_eq!(va.y.to_bits(), vb.y.to_bits());
            assert_eq!(va.z.to_bits(), vb.z.to_bits());
        }
    }

    #[test]
    fn let_bindings_name_parts_and_split_per_part_meshes() {
        // `let` names flow onto part tags (darkair DSL adaptation): the barrel
        // and dome survive as separately-tagged parts of the union, and
        // to_meshes_yup_by_part splits them into per-part Y-up meshes.
        let src = "let barrel = cylinder(2, 8, 24).move(0, 0, 4)\n\
                   let dome = sphere(2, 12).move(0, 0, 8)\n\
                   model barrel + dome";
        let c = compile_vim(src).expect("compiles");
        let names: Vec<&str> = c.solid.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["barrel", "dome"], "let names become part tags");

        let parted = c.solid.to_meshes_yup_by_part();
        assert_eq!(parted.len(), 2, "one mesh per named part");
        assert_eq!(parted[0].0, "barrel");
        assert_eq!(parted[1].0, "dome");
        // The split meshes together carry exactly the whole solid.
        let (all_pos, all_idx) = c.solid.to_mesh_yup();
        let split_pos: usize = parted.iter().map(|(_, p, _)| p.len()).sum();
        let split_idx: usize = parted.iter().map(|(_, _, i)| i.len()).sum();
        assert_eq!(split_pos, all_pos.len());
        assert_eq!(split_idx, all_idx.len());
        let split_vol: f64 = parted
            .iter()
            .map(|(_, p, i)| mesh_volume(p, i))
            .sum();
        let whole_vol = mesh_volume(&all_pos, &all_idx);
        assert!(
            (split_vol - whole_vol).abs() < whole_vol.abs() * 1e-6,
            "split {split_vol} vs whole {whole_vol}"
        );

        // A polar array's copies share the let name and merge into one mesh.
        let legs = compile_vim(
            "let leg = cylinder(0.2, 4, 8).move(1.5, 0, 2)\nmodel leg.polar(4)",
        )
        .expect("legs compile");
        assert!(legs.solid.parts.iter().all(|p| p.name == "leg"));
        let parted = legs.solid.to_meshes_yup_by_part();
        assert_eq!(parted.len(), 1, "same-named parts merge");
        assert_eq!(parted[0].0, "leg");
    }

    #[test]
    fn compile_vim_reports_errors() {
        let err = compile_vim("model wibble(3)").unwrap_err();
        assert!(err.contains("wibble"), "message was: {err}");
        let err = compile_vim("let a = cube(2)").unwrap_err();
        assert!(err.contains("model"), "message was: {err}");
    }
}
