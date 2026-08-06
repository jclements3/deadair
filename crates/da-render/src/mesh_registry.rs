//! Deterministic mesh id assignment and the CPU-side mesh registry.
//!
//! The draw contract references triangle meshes by `u32` id
//! ([`crate::draw::Shape::Mesh`]); the id is a **content hash** (FNV-1a over
//! the exact vertex/index bytes), never a pointer or insertion order —
//! determinism is load-bearing (same zone source + seed must produce the
//! same DrawList bytes on every run and every machine).
//!
//! Flow: the app hashes each graph mesh with [`mesh_id`] during conversion
//! (pure, no registry needed per frame), collects the `(id, MeshData)`
//! pairs into a [`MeshRegistry`] at zone load, and hands the registry to
//! [`crate::renderer::Renderer::register_meshes`] before rendering.

use crate::mesh::MeshData;
use glam::Vec3;
use std::collections::BTreeMap;

/// Stable content hash of a triangle mesh: FNV-1a (32-bit) over the vertex
/// positions (f32 little-endian bit patterns) then the indices.
///
/// A pure function of the mesh bytes — the same geometry always yields the
/// same id, across processes and machines. This is the ONLY id assignment
/// scheme: `Shape::Mesh { id }` producers and registry insertions must both
/// come through here (or [`MeshRegistry::insert`], which delegates).
pub fn mesh_id(vertices: &[Vec3], indices: &[u32]) -> u32 {
    const FNV_OFFSET: u32 = 0x811c_9dc5;
    const FNV_PRIME: u32 = 0x0100_0193;
    let mut h = FNV_OFFSET;
    let mut eat = |byte: u8| {
        h ^= byte as u32;
        h = h.wrapping_mul(FNV_PRIME);
    };
    for v in vertices {
        for f in [v.x, v.y, v.z] {
            for b in f.to_bits().to_le_bytes() {
                eat(b);
            }
        }
    }
    for i in indices {
        for b in i.to_le_bytes() {
            eat(b);
        }
    }
    h
}

/// CPU-side collection of meshes awaiting GPU registration, keyed by their
/// content-hash id. `BTreeMap` so iteration order is deterministic (sorted
/// by id), never HashMap/pointer order.
#[derive(Debug, Default)]
pub struct MeshRegistry {
    meshes: BTreeMap<u32, MeshData>,
}

impl MeshRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Hash the mesh, build its flat-shaded [`MeshData`] (once — inserting
    /// the same content again is a no-op), and return the stable id. This
    /// is the id `Shape::Mesh { id }` must carry for the mesh to draw.
    pub fn insert(&mut self, vertices: &[Vec3], indices: &[u32]) -> u32 {
        let id = mesh_id(vertices, indices);
        self.meshes
            .entry(id)
            .or_insert_with(|| MeshData::from_positions_indices(vertices, indices));
        id
    }

    pub fn get(&self, id: u32) -> Option<&MeshData> {
        self.meshes.get(&id)
    }

    /// All `(id, mesh)` pairs, in deterministic (sorted-by-id) order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &MeshData)> {
        self.meshes.iter().map(|(id, m)| (*id, m))
    }

    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tetra() -> (Vec<Vec3>, Vec<u32>) {
        (
            vec![
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1.0, -1.0, -1.0),
                Vec3::new(-1.0, 1.0, -1.0),
                Vec3::new(-1.0, -1.0, 1.0),
            ],
            vec![0, 1, 2, 0, 3, 1, 0, 2, 3, 1, 3, 2],
        )
    }

    #[test]
    fn same_content_same_id_across_registries() {
        let (v, i) = tetra();
        assert_eq!(mesh_id(&v, &i), mesh_id(&v, &i));
        let mut a = MeshRegistry::new();
        let mut b = MeshRegistry::new();
        assert_eq!(a.insert(&v, &i), b.insert(&v, &i));
        assert_eq!(a.insert(&v, &i), mesh_id(&v, &i), "insert delegates to mesh_id");
    }

    #[test]
    fn different_content_different_id() {
        let (v, i) = tetra();
        let mut v2 = v.clone();
        v2[0].x += 0.25;
        assert_ne!(mesh_id(&v, &i), mesh_id(&v2, &i));
        // Index order matters too (a different winding is a different mesh).
        let mut i2 = i.clone();
        i2.swap(0, 1);
        assert_ne!(mesh_id(&v, &i), mesh_id(&v, &i2));
    }

    #[test]
    fn insert_is_idempotent_and_iter_is_sorted() {
        let (v, i) = tetra();
        let mut v2 = v.clone();
        v2[0].x += 0.25;
        let mut reg = MeshRegistry::new();
        let id1 = reg.insert(&v, &i);
        let id2 = reg.insert(&v2, &i);
        reg.insert(&v, &i); // duplicate: no growth
        assert_eq!(reg.len(), 2);
        assert!(reg.get(id1).is_some() && reg.get(id2).is_some());
        let ids: Vec<u32> = reg.iter().map(|(id, _)| id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "iteration order is deterministic");
    }
}
