//! Bridges between the crates: scene graph leaves → renderer draw items,
//! thermal attaches → full thermal profiles, species name mappings.

use da_graph::prelude::*;
use da_render::draw::{DrawItem, Shape as RShape};
use da_thermal::ThermalProfile;
use std::collections::BTreeMap;

/// Precomputed mesh ids keyed by `(geode node, drawable index)`, built once
/// per zone by [`collect_meshes`]. [`leaf_to_item`] runs per leaf per frame,
/// so it must never rehash mesh geometry — it resolves ids through this map
/// instead. `BTreeMap` for deterministic iteration, matching the registry.
pub type MeshIds = BTreeMap<(NodeId, usize), u32>;

/// A zone's graph meshes: the renderer-side registry plus the per-drawable
/// id lookup the per-frame draw path uses. Both are keyed by the same
/// content hashes ([`da_render::mesh_id`]), assigned in one walk so they
/// can never disagree.
pub struct SceneMeshes {
    /// Hand to `Renderer::register_meshes` before drawing.
    pub registry: da_render::MeshRegistry,
    /// Hand to [`leaf_to_item`] each frame (O(log n) lookup, no rehash).
    pub ids: MeshIds,
}

/// Map a scene-graph shape to a renderer shape. `None` = not drawable yet.
///
/// Meshes map to `RShape::Mesh { id }` where the id is the deterministic
/// content hash [`da_render::mesh_id`] — the same id [`collect_meshes`]
/// registers under, so a converted mesh draws once the zone's registry has
/// been handed to `Renderer::register_meshes`.
///
/// Hashing is O(mesh bytes): fine at load, too hot for a per-frame path.
/// Frame code goes through [`leaf_to_item`], which resolves ids from the
/// precomputed [`MeshIds`] cache instead of calling this on meshes.
pub fn map_shape(shape: &da_graph::Shape) -> Option<RShape> {
    Some(match *shape {
        da_graph::Shape::Box { half_extents } => RShape::Box { half: half_extents },
        da_graph::Shape::Cylinder { radius, height } => RShape::Cylinder { radius, height },
        da_graph::Shape::Sphere { radius } => RShape::Sphere { radius },
        // Renderer has no capsule primitive yet; a cylinder is close enough
        // for silhouettes.
        da_graph::Shape::Capsule { radius, height } => RShape::Cylinder {
            radius,
            height: height + 2.0 * radius,
        },
        da_graph::Shape::Mesh {
            ref vertices,
            ref indices,
        } => RShape::Mesh {
            id: da_render::mesh_id(vertices, indices),
        },
    })
}

/// Collect every graph mesh in a scene: the renderer-side registry plus the
/// `(node, drawable) -> id` cache, keyed by the same content-hash ids
/// [`map_shape`] assigns. Call once at zone load; hand `.registry` to
/// `Renderer::register_meshes` and `.ids` to [`leaf_to_item`] each frame.
pub fn collect_meshes(scene: &Scene) -> SceneMeshes {
    let mut registry = da_render::MeshRegistry::new();
    let mut ids = MeshIds::new();
    for node in scene.nodes() {
        if let NodeKind::Geode(g) = node.kind() {
            for (i, d) in g.drawables.iter().enumerate() {
                if let da_graph::Shape::Mesh { vertices, indices } = &d.shape {
                    ids.insert((node.id(), i), registry.insert(vertices, indices));
                }
            }
        }
    }
    SceneMeshes { registry, ids }
}

/// Convert one culled leaf into a draw item; `temp_f` comes from the
/// thermal sim (or ambient when the node carries no profile), `mesh_ids`
/// from the zone's [`collect_meshes`] result (so meshes cost an id lookup
/// per frame, not a rehash of their whole vertex/index buffers).
pub fn leaf_to_item(
    leaf: &RenderLeaf,
    scene: &Scene,
    mesh_ids: &MeshIds,
    temp_f: f32,
) -> Option<DrawItem> {
    let node = scene.node(leaf.node)?;
    let geode = match node.kind() {
        NodeKind::Geode(g) => g,
        _ => return None,
    };
    let drawable = geode.drawables.get(leaf.drawable)?;
    let shape = match drawable.shape {
        // Cached id, O(log n) — never rehash geometry on the frame path.
        // The fallback keeps meshes added after the cache was built correct
        // (just slow) rather than invisible.
        da_graph::Shape::Mesh {
            ref vertices,
            ref indices,
        } => RShape::Mesh {
            id: mesh_ids
                .get(&(leaf.node, leaf.drawable))
                .copied()
                .unwrap_or_else(|| da_render::mesh_id(vertices, indices)),
        },
        ref other => map_shape(other)?,
    };
    // Graph cylinders are CENTERED on their transform; the renderer's unit
    // cylinder is base-anchored (mesh.rs: y in [0,1]). Drop the base by
    // half the height in local space so graph geometry sits on the ground.
    let world = match drawable.shape {
        da_graph::Shape::Cylinder { height, .. } => {
            leaf.world * glam::Mat4::from_translation(glam::Vec3::new(0.0, -height * 0.5, 0.0))
        }
        da_graph::Shape::Capsule { radius, height } => {
            leaf.world
                * glam::Mat4::from_translation(glam::Vec3::new(
                    0.0,
                    -(height * 0.5 + radius),
                    0.0,
                ))
        }
        _ => leaf.world,
    };
    let color = leaf
        .state
        .base_color
        .unwrap_or(glam::Vec4::new(0.5, 0.5, 0.5, 1.0));
    let emissive = leaf
        .state
        .emissive
        .map(|e| e.length() / 3.0f32.sqrt())
        .unwrap_or(0.0);
    Some(DrawItem {
        shape,
        world,
        albedo: [color.x, color.y, color.z],
        emissive,
        temp_f,
        glass: leaf.state.glass.unwrap_or(false),
        coat_f: 0.0,
    })
}

/// Recover a full thermal profile from the graph's slimmer `ThermalAttach`
/// by matching the nearest da-thermal preset on (thermal_mass, sky_exposure)
/// — the attach doesn't carry solar gain, the preset supplies it.
pub fn profile_from_attach(attach: &ThermalAttach) -> ThermalProfile {
    let presets = [
        ThermalProfile::metal_roof(),
        ThermalProfile::rock(),
        ThermalProfile::grass(),
        ThermalProfile::water(),
        ThermalProfile::tree(),
        ThermalProfile::building_wall(),
        ThermalProfile::glass(),
    ];
    let dist = |p: &ThermalProfile| {
        let dm = (p.thermal_mass.ln() - attach.thermal_mass.max(0.1).ln()).abs();
        let ds = (p.sky_exposure - attach.sky_exposure).abs() * 3.0;
        dm + ds
    };
    let mut best = presets[0].clone();
    let mut best_d = dist(&best);
    for p in &presets[1..] {
        let d = dist(p);
        if d < best_d {
            best_d = d;
            best = p.clone();
        }
    }
    // Keep the attach's authored numbers; the preset only contributes the
    // solar-gain estimate.
    ThermalProfile {
        thermal_mass: attach.thermal_mass,
        sky_exposure: attach.sky_exposure,
        ..best
    }
}

/// da-param species names → da-sim species.
pub fn sim_species(name: &str) -> Option<da_sim::Species> {
    use da_sim::Species::*;
    Some(match name {
        "Rat" => Rat,
        "Rabbit" => Rabbit,
        "Possum" => Possum,
        "Raccoon" => Raccoon,
        "Dog" => Dog,
        "Cat" => Cat,
        "Cow" => Cow,
        "Sheep" => Sheep,
        "Zombie" => Zombie,
        "Groundhog" => Groundhog,
        "Beaver" => Beaver,
        "JuvenileFeralHog" => JuvenileFeralHog,
        _ => return None,
    })
}

/// da-sim species → da-econ species (for the ledger).
pub fn econ_species(s: da_sim::Species) -> da_econ::Species {
    use da_sim::Species as S;
    match s {
        S::Rat => da_econ::Species::Rat,
        S::Rabbit => da_econ::Species::Rabbit,
        S::Possum => da_econ::Species::Possum,
        S::Raccoon => da_econ::Species::Raccoon,
        S::Groundhog => da_econ::Species::Groundhog,
        S::Beaver => da_econ::Species::Beaver,
        S::JuvenileFeralHog => da_econ::Species::JuvenileFeralHog,
        S::Dog => da_econ::Species::Dog,
        S::Cat => da_econ::Species::Cat,
        S::Cow => da_econ::Species::Cow,
        S::Sheep => da_econ::Species::Sheep,
        S::Zombie => da_econ::Species::Zombie,
    }
}

/// da-param hazard kinds → da-sim hazard kinds.
pub fn sim_hazard_kind(k: da_param::HazardKind) -> da_sim::HazardKind {
    use da_param::HazardKind as P;
    use da_sim::HazardKind as S;
    match k {
        P::Wire => S::Wire,
        P::Hole => S::Hole,
        P::CreekBank => S::CreekBank,
        P::Water => S::Water,
        P::Limb => S::Limb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_core::TempF;

    #[test]
    fn metal_roof_attach_recovers_solar_gain() {
        let attach = ThermalAttach {
            base_temp: TempF(68.0),
            thermal_mass: 100.0,
            sky_exposure: 1.0,
        };
        let p = profile_from_attach(&attach);
        assert!(p.initial_solar_gain_f > 15.0, "metal roof gain: {p:?}");
        assert_eq!(p.sky_exposure, 1.0);
    }

    #[test]
    fn grass_attach_maps_to_low_gain() {
        let attach = ThermalAttach {
            base_temp: TempF(68.0),
            thermal_mass: 60.0,
            sky_exposure: 0.9,
        };
        let p = profile_from_attach(&attach);
        assert!(p.initial_solar_gain_f < 5.0, "grass gain: {p:?}");
    }

    #[test]
    fn graph_mesh_maps_to_stable_id_and_collect_registers_it() {
        use glam::Vec3;
        let vertices = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let indices = vec![0, 2, 1, 0, 1, 3, 0, 3, 2, 1, 2, 3];
        let shape = da_graph::Shape::Mesh {
            vertices: vertices.clone(),
            indices: indices.clone(),
        };

        // Two independent conversions agree — the id is a content hash,
        // not a pointer or insertion order.
        let a = map_shape(&shape).expect("meshes are drawable now");
        let b = map_shape(&shape.clone()).expect("drawable");
        assert_eq!(a, b, "same mesh, same id, every time");
        let RShape::Mesh { id } = a else {
            panic!("graph mesh must convert to RShape::Mesh, got {a:?}");
        };

        // Different geometry gets a different id.
        let mut v2 = vertices.clone();
        v2[0].x = 0.5;
        let other = map_shape(&da_graph::Shape::Mesh {
            vertices: v2,
            indices: indices.clone(),
        });
        assert_ne!(Some(a), other);

        // collect_meshes finds the mesh in a scene and registers it under
        // the exact id map_shape emitted — in both the registry and the
        // per-drawable id cache the frame path reads.
        let mut scene = Scene::new();
        let geode = scene.add_geode(scene.root()).expect("geode");
        scene
            .add_drawable(geode, Drawable::new(shape))
            .expect("drawable");
        let meshes = collect_meshes(&scene);
        assert_eq!(meshes.registry.len(), 1);
        assert!(
            meshes.registry.get(id).is_some(),
            "registry id must match map_shape id {id}"
        );
        assert_eq!(
            meshes.ids.get(&(geode, 0)).copied(),
            Some(id),
            "id cache must map (geode, drawable 0) to the map_shape id"
        );

        // The per-frame path resolves the same id through the cache.
        let leaves = CullVisitor::new(glam::Vec3::ZERO).cull(&scene);
        assert_eq!(leaves.len(), 1);
        let item = leaf_to_item(&leaves[0], &scene, &meshes.ids, 68.0)
            .expect("mesh leaf converts");
        assert_eq!(item.shape, RShape::Mesh { id });
        // ...and stays correct (fallback rehash) with an empty cache.
        let item = leaf_to_item(&leaves[0], &scene, &MeshIds::new(), 68.0)
            .expect("mesh leaf converts without cache");
        assert_eq!(item.shape, RShape::Mesh { id });
    }

    #[test]
    fn species_round_trip() {
        assert_eq!(sim_species("Rat"), Some(da_sim::Species::Rat));
        assert_eq!(sim_species("Rabbit"), Some(da_sim::Species::Rabbit));
        assert_eq!(
            econ_species(da_sim::Species::Rabbit),
            da_econ::Species::Rabbit
        );
        assert_eq!(
            sim_species("JuvenileFeralHog"),
            Some(da_sim::Species::JuvenileFeralHog)
        );
        assert_eq!(
            econ_species(da_sim::Species::Beaver),
            da_econ::Species::Beaver
        );
        assert_eq!(econ_species(da_sim::Species::Cat), da_econ::Species::Cat);
    }
}
