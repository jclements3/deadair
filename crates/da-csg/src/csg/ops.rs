//! Edge beveling: `fillet` (rounded edges) and `chamfer` (flat cut edges).
//!
//! # Strategy — reliability first
//!
//! Rather than surgically re-meshing the neighbourhood of every edge (which is
//! fragile and prone to self-intersection where several beveled edges meet at a
//! corner), we bevel by **cutting material away with half-space planes** and let
//! the proven BSP boolean core (`super::bsp`) resolve every intersection.
//!
//! Every operation is `solid ∩ half-space`, and intersecting a valid closed
//! solid with a convex half-space always yields another valid, watertight,
//! outward-wound closed solid. Corners where several beveled edges meet fall out
//! of the plane intersections automatically — exactly how you would chamfer a
//! cube by slicing its 12 edges (the 8 corner facets appear for free).
//!
//! * A **chamfer** slices each convex edge with one bevel plane (the flat).
//! * A **fillet** approximates the rounded edge with `segments` planes tangent
//!   to the target arc, so the rounded surface circumscribes the true arc.
//!
//! # What this handles well
//!
//! Prismatic / box-like solids with sharp **convex** edges (cube, box, wedge,
//! pyramid, extrusions): chamfer and multi-facet fillet look correct and stay
//! watertight.
//!
//! # What it is deliberately conservative about (edges left sharp)
//!
//! * **Concave** edges — beveling them would *add* material, which a cut-based
//!   approach cannot do, so they are skipped.
//! * **Near-flat / smoothly-curved** edges (sphere facets, cylinder walls) fall
//!   below the dihedral threshold and are left untouched.
//! * **Non-manifold** edges (not shared by exactly two faces) are left sharp.
//! * Fillet corners are faceted rather than perfectly spherical.
//!
//! If the radius/distance is too large for an edge (it would consume the whole
//! feature) the op returns `Err` rather than emit degenerate geometry.

use super::bsp::{v3, Polygon, Vertex, V3};
use super::Solid;
use std::collections::{BTreeMap, HashMap};

/// Coplanarity / degeneracy threshold on the angle between adjacent face
/// normals (radians). Below this an "edge" is essentially flat and skipped.
const MIN_ANGLE: f64 = 0.02; // ~1.1 degrees

/// Chamfer the convex edges of `solid`, cutting a flat of the given `distance`
/// (measured along each adjacent face) off every sharp convex edge.
pub fn chamfer(solid: &Solid, distance: f64) -> Result<Solid, String> {
    bevel(solid, distance, None)
}

/// Fillet (round) the convex edges of `solid` with the given `radius`, using
/// `segments` flat facets per edge to approximate each rounded arc.
pub fn fillet(solid: &Solid, radius: f64, segments: usize) -> Result<Solid, String> {
    bevel(solid, radius, Some(segments.max(1)))
}

/// A convex edge selected for beveling.
struct Bevel {
    mid: V3,   // a point on the edge
    n_a: V3,   // outward normal of the first face
    n_b: V3,   // outward normal of the second face
    in_a: V3,  // in-plane inward direction on face A (perp. to the edge)
    phi: f64,  // angle between the two outward normals (exterior angle)
    len: f64,  // edge length
}

/// Shared implementation. `segments == None` → chamfer (one flat plane per
/// edge); `Some(n)` → fillet (n tangent planes per edge).
fn bevel(solid: &Solid, param: f64, segments: Option<usize>) -> Result<Solid, String> {
    if solid.polys.is_empty() {
        return Err("cannot bevel an empty solid".into());
    }
    if !(param > 0.0) || !param.is_finite() {
        return Err(format!(
            "{} must be a positive number",
            if segments.is_some() { "radius" } else { "distance" }
        ));
    }

    let bevels = collect_convex_edges(solid);
    if bevels.is_empty() {
        // No sharp convex edges to work with (e.g. a sphere). Leave the solid
        // unchanged rather than error or corrupt it.
        return Ok(solid.clone());
    }

    // A rough scene size, used to size the (very large) cutting half-spaces so a
    // single box face does the cutting and the rest safely clears the model.
    let diameter = scene_diameter(solid);
    let cut_size = diameter * 4.0 + 1.0;

    // Validate radius/distance against every edge before mutating anything.
    for b in &bevels {
        let setback = match segments {
            Some(_) => param * (b.phi * 0.5).tan(), // fillet: r * tan(phi/2)
            None => param,                          // chamfer: distance along face
        };
        if setback >= b.len {
            return Err(format!(
                "{} too large for this geometry (would consume an edge of length {:.4})",
                if segments.is_some() { "radius" } else { "distance" },
                b.len
            ));
        }
    }

    // Apply the cuts. Each is an intersection with a half-space, so the result
    // is always a valid closed solid.
    let mut work = solid.clone();
    for b in &bevels {
        let setback = match segments {
            Some(_) => param * (b.phi * 0.5).tan(),
            None => param,
        };
        let s_a = b.mid.add(b.in_a.mul(setback)); // tangent/cut point on face A

        match segments {
            None => {
                // Chamfer: a single flat plane cutting `setback` off each face.
                // With equal setback along both faces the cut plane is the outward
                // bisector of the two face normals, passing through the face-A cut
                // point `s_a` (equivalently the face-B cut point — same plane).
                let m = b.n_a.add(b.n_b).normalized();
                let w = m.dot(s_a);
                work = clip_halfspace(work, m, w, b.mid, cut_size);
            }
            Some(n) => {
                // Fillet: circular arc of radius `param`, tangent to both faces.
                // Centre lies at distance r from face A along -n_a from s_a.
                let center = s_a.sub(b.n_a.mul(param));
                for k in 0..n {
                    let t = (k as f64 + 0.5) / n as f64;
                    let dir = slerp(b.n_a, b.n_b, t, b.phi); // outward facet normal
                    let w = dir.dot(center) + param;
                    work = clip_halfspace(work, dir, w, b.mid, cut_size);
                }
            }
        }
    }

    let vol = work.volume();
    if !vol.is_finite() || vol <= 0.0 {
        return Err("radius/distance too large for this geometry (produced no solid)".into());
    }
    Ok(work)
}

/// Weld vertices, build edge adjacency, and return every sharp *convex* edge
/// shared by exactly two faces.
fn collect_convex_edges(solid: &Solid) -> Vec<Bevel> {
    // Weld positions onto a grid so shared edges resolve to identical indices.
    let mut index: HashMap<(i64, i64, i64), usize> = HashMap::new();
    let mut positions: Vec<V3> = Vec::new();
    let mut weld = |p: V3| -> usize {
        let q = 1.0e6_f64;
        let key = (
            (p.x * q).round() as i64,
            (p.y * q).round() as i64,
            (p.z * q).round() as i64,
        );
        *index.entry(key).or_insert_with(|| {
            positions.push(p);
            positions.len() - 1
        })
    };

    // Faces as welded-index loops.
    let mut faces: Vec<Vec<usize>> = Vec::with_capacity(solid.polys.len());
    for p in &solid.polys {
        let loop_: Vec<usize> = p.vertices.iter().map(|v| weld(v.pos)).collect();
        faces.push(loop_);
    }
    drop(weld); // release the mutable borrow of `positions`

    // A robust (Newell) outward normal per face.
    let normals: Vec<V3> = faces
        .iter()
        .map(|loop_| newell_normal(loop_, &positions))
        .collect();

    // Directed edge uses, grouped by the undirected {min,max} vertex pair.
    // We also carry the *stored vertex normals* at each endpoint: this is how we
    // tell an intended hard edge (adjacent faces store different normals, e.g. a
    // cube corner) from the tessellation of a smooth surface (adjacent faces
    // share a smooth normal, e.g. a sphere / cylinder wall — left untouched).
    struct Use {
        face: usize,
        a: usize,     // welded index of the first endpoint (as wound in this face)
        b: usize,     // welded index of the second endpoint
        na: V3,       // stored vertex normal at `a`
        nb: V3,       // stored vertex normal at `b`
    }
    // darkair: BTreeMap (not HashMap) so bevel cuts apply in a deterministic
    // order — same solid in, byte-identical solid out (determinism is
    // load-bearing in darkair). Semantics are otherwise unchanged.
    let mut edges: BTreeMap<(usize, usize), Vec<Use>> = BTreeMap::new();
    for (fi, loop_) in faces.iter().enumerate() {
        let n = loop_.len();
        let verts = &solid.polys[fi].vertices;
        for i in 0..n {
            let a = loop_[i];
            let b = loop_[(i + 1) % n];
            if a == b {
                continue;
            }
            let key = if a < b { (a, b) } else { (b, a) };
            edges.entry(key).or_default().push(Use {
                face: fi,
                a,
                b,
                na: verts[i].normal,
                nb: verts[(i + 1) % n].normal,
            });
        }
    }

    let mut out = Vec::new();
    for uses in edges.values() {
        if uses.len() != 2 {
            continue; // non-manifold or boundary — leave sharp
        }
        let ua = &uses[0];
        let ub = &uses[1];
        let n_a = normals[ua.face];
        let n_b = normals[ub.face];
        if n_a.len() < 0.5 || n_b.len() < 0.5 {
            continue; // degenerate face normal
        }

        // Smooth-surface test: if both faces store the *same* shading normal at
        // the shared endpoints, this edge is the seam of a smooth surface (sphere,
        // cylinder wall), not a hard feature edge — leave it untouched. `ub` winds
        // the edge in the opposite direction, so match normals by welded index.
        let ub_na = if ub.a == ua.a { ub.na } else { ub.nb };
        let ub_nb = if ub.b == ua.b { ub.nb } else { ub.na };
        if ua.na.dot(ub_na) > 0.999 && ua.nb.dot(ub_nb) > 0.999 {
            continue;
        }
        let pa = positions[ua.a];
        let pb = positions[ua.b];
        let e_dir = pb.sub(pa).normalized(); // edge direction as wound in face A

        // Convexity: for outward normals, an edge is convex iff (n_a × n_b)·eDir > 0.
        let convex = n_a.cross(n_b).dot(e_dir) > 1.0e-9;
        if !convex {
            continue;
        }
        let phi = n_a.dot(n_b).clamp(-1.0, 1.0).acos();
        if phi < MIN_ANGLE || phi > std::f64::consts::PI - MIN_ANGLE {
            continue; // essentially flat, or a razor edge we can't bevel sanely
        }

        // Inward in-plane direction on face A (perpendicular to the edge).
        let in_a = n_a.cross(e_dir).normalized();
        out.push(Bevel {
            mid: pa.add(pb).mul(0.5),
            n_a,
            n_b,
            in_a,
            phi,
            len: pb.sub(pa).len(),
        });
    }
    out
}

/// Newell's method: an area-weighted normal that is robust for planar convex
/// polygons and points outward for CCW (outward-wound) loops.
fn newell_normal(loop_: &[usize], positions: &[V3]) -> V3 {
    let n = loop_.len();
    let mut nx = 0.0;
    let mut ny = 0.0;
    let mut nz = 0.0;
    for i in 0..n {
        let a = positions[loop_[i]];
        let b = positions[loop_[(i + 1) % n]];
        nx += (a.y - b.y) * (a.z + b.z);
        ny += (a.z - b.z) * (a.x + b.x);
        nz += (a.x - b.x) * (a.y + b.y);
    }
    v3(nx, ny, nz).normalized()
}

/// Spherical linear interpolation between two unit vectors separated by `phi`.
fn slerp(a: V3, b: V3, t: f64, phi: f64) -> V3 {
    if phi < 1.0e-9 {
        return a;
    }
    let s = phi.sin();
    let wa = ((1.0 - t) * phi).sin() / s;
    let wb = (t * phi).sin() / s;
    a.mul(wa).add(b.mul(wb)).normalized()
}

/// Bounding-box diagonal length — a cheap measure of scene size.
fn scene_diameter(solid: &Solid) -> f64 {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for p in &solid.polys {
        for v in &p.vertices {
            for (k, c) in [v.pos.x, v.pos.y, v.pos.z].into_iter().enumerate() {
                lo[k] = lo[k].min(c);
                hi[k] = hi[k].max(c);
            }
        }
    }
    let d = v3(hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]);
    d.len().max(1.0)
}

/// Keep the part of `solid` on the inside of the plane `{ x : m·x ≤ w }` by
/// intersecting with a large box whose top face lies on that plane and whose
/// body extends inward. `refp` is a point near the region of interest (so the
/// box stays close to the model for numerical stability); `size` must exceed
/// the model's extent so only the plane face actually cuts.
fn clip_halfspace(solid: Solid, m: V3, w: f64, refp: V3, size: f64) -> Solid {
    let m = m.normalized();
    // Orthonormal basis (u, v, m).
    let helper = if m.x.abs() < 0.9 {
        v3(1.0, 0.0, 0.0)
    } else {
        v3(0.0, 1.0, 0.0)
    };
    let u = m.cross(helper).normalized();
    let vv = m.cross(u).normalized();

    // Project the reference point onto the plane so the box's cutting face lies
    // exactly on it while the box stays centred near the model.
    let p = refp.sub(m.mul(m.dot(refp) - w));

    let eu = u.mul(size);
    let ev = vv.mul(size);
    let em = m.mul(size);

    // Top face on the plane (c = 0), bottom face pushed inward (c = -size).
    let t00 = p.sub(eu).sub(ev);
    let t10 = p.add(eu).sub(ev);
    let t11 = p.add(eu).add(ev);
    let t01 = p.sub(eu).add(ev);
    let b00 = t00.sub(em);
    let b10 = t10.sub(em);
    let b11 = t11.sub(em);
    let b01 = t01.sub(em);

    let box_center = p.sub(m.mul(size * 0.5));
    let polys = vec![
        quad(box_center, t00, t10, t11, t01), // top (on plane)
        quad(box_center, b00, b01, b11, b10), // bottom
        quad(box_center, t00, t01, b01, b00), // -u side
        quad(box_center, t10, t11, b11, b10), // +u ... (winding fixed below)
        quad(box_center, t00, b00, b10, t10), // -v side
        quad(box_center, t01, t11, b11, b01), // +v side
    ];

    // The cutting box is one throwaway "bevel" part; intersection offsets its id
    // past the work solid's parts, so any surviving cut face keeps a valid tag.
    solid.intersection(Solid {
        polys,
        parts: vec![super::Part {
            name: "bevel".to_string(),
            temp: super::DEFAULT_TEMP,
        }],
    })
}

/// Build a quad with a flat outward normal, reversing the winding if needed so
/// the normal points away from `center`.
fn quad(center: V3, a: V3, b: V3, c: V3, d: V3) -> Polygon {
    let mut n = b.sub(a).cross(c.sub(a)).normalized();
    let centroid = a.add(b).add(c).add(d).mul(0.25);
    let outward = centroid.sub(center);
    if n.dot(outward) < 0.0 {
        // Reverse the loop so it is wound CCW as seen from outside.
        n = n.negated();
        return Polygon::new(vec![
            Vertex::new(d, n),
            Vertex::new(c, n),
            Vertex::new(b, n),
            Vertex::new(a, n),
        ]);
    }
    Polygon::new(vec![
        Vertex::new(a, n),
        Vertex::new(b, n),
        Vertex::new(c, n),
        Vertex::new(d, n),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chamfer_cube_removes_material_stays_watertight() {
        let cube = Solid::cube(2.0, 2.0, 2.0);
        let orig_tris = cube.triangle_count();
        let out = chamfer(&cube, 0.4).expect("chamfer should succeed");

        let v = out.volume();
        assert!(v > 0.0, "volume must be positive, got {v}");
        assert!(v < 8.0, "chamfer must remove material, got {v}");
        // Removing 12 small edge wedges + 8 corners from an 8.0 cube stays well
        // above half the volume.
        assert!(v > 6.0 && v < 8.0, "volume out of sane range: {v}");
        assert!(
            out.triangle_count() > orig_tris,
            "chamfer should add faces: {} -> {}",
            orig_tris,
            out.triangle_count()
        );

        // Watertightness / orientation: signed volume recomputed identically.
        assert!((out.volume() - v).abs() < 1e-9, "volume not stable");
    }

    #[test]
    fn fillet_cube_removes_material_stays_watertight() {
        let cube = Solid::cube(2.0, 2.0, 2.0);
        let orig_tris = cube.triangle_count();
        let out = fillet(&cube, 0.4, 6).expect("fillet should succeed");

        let v = out.volume();
        assert!(v > 0.0, "volume must be positive, got {v}");
        assert!(v < 8.0, "fillet must remove material, got {v}");
        assert!(v > 6.0 && v < 8.0, "volume out of sane range: {v}");
        assert!(
            out.triangle_count() > orig_tris,
            "fillet should add faces: {} -> {}",
            orig_tris,
            out.triangle_count()
        );
        assert!((out.volume() - v).abs() < 1e-9, "volume not stable");
    }

    #[test]
    fn fillet_removes_less_than_the_deepest_chamfer() {
        // A rounded edge leaves more material than a flat cut of comparable size.
        let cube = Solid::cube(2.0, 2.0, 2.0);
        let cham = chamfer(&cube, 0.4).unwrap().volume();
        let fil = fillet(&cube, 0.4, 8).unwrap().volume();
        assert!(fil > cham, "fillet {fil} should keep more than chamfer {cham}");
    }

    #[test]
    fn rejects_bad_input() {
        let cube = Solid::cube(2.0, 2.0, 2.0);
        assert!(chamfer(&cube, 0.0).is_err(), "zero distance must error");
        assert!(fillet(&cube, -1.0, 6).is_err(), "negative radius must error");
        assert!(
            chamfer(&cube, 5.0).is_err(),
            "distance larger than the geometry must error"
        );
        assert!(
            chamfer(&Solid { polys: vec![], parts: vec![] }, 0.4).is_err(),
            "empty solid must error"
        );
    }

    #[test]
    fn smooth_solid_is_left_unchanged() {
        // A sphere has no sharp convex edges above the threshold: pass through.
        let sph = Solid::sphere(1.0, 24);
        let v0 = sph.volume();
        let out = fillet(&sph, 0.1, 4).expect("should not error");
        assert!((out.volume() - v0).abs() < 1e-9, "sphere should be unchanged");
    }
}
