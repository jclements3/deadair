//! Cull output → renderer draw list.
//!
//! da-render deliberately does not walk the scene graph: the editor (like
//! the game) flattens [`da_graph::CullVisitor`] output into a
//! [`da_render::DrawList`]. Shape mapping:
//!
//! | da-graph               | da-render                                |
//! |------------------------|------------------------------------------|
//! | `Box{half_extents}`    | `Box{half}`                              |
//! | `Cylinder{r, h}`       | `Cylinder{r, h}`                         |
//! | `Sphere{r}`            | `Sphere{r}`                              |
//! | `Capsule{r, h}`        | `Cylinder{r, h + 2r}` (approximation)    |
//! | `Mesh{..}`             | `Mesh{id}` (content-hash id; see [`collect_meshes`]) |
//!
//! Materials come from the leaf's fully-merged effective [`StateSet`]:
//! `base_color` → albedo, `emissive` → max RGB component, `glass` passes
//! through, and the display temperature comes from
//! [`crate::preview::PreviewEnv`].

use da_graph::{NodeKind, RenderLeaf, Scene, Shape as GraphShape, StateSet};
use da_render::draw::{DrawItem, DrawList, Shape as RenderShape};
use std::collections::BTreeMap;

use crate::preview::{PreviewEnv, TempSource};

/// Precomputed mesh ids keyed by `(geode node, drawable index)`, built once
/// per expansion by [`collect_meshes`]. [`build_draw_list`] runs every
/// frame, so [`leaf_to_item`] resolves mesh ids through this map instead of
/// rehashing the geometry. Mirrors `darkair::convert::MeshIds`.
pub type MeshIds = BTreeMap<(da_core::NodeId, usize), u32>;

/// An expansion's graph meshes: the renderer-side registry plus the
/// per-drawable id lookup the per-frame draw path uses. Both are keyed by
/// the same content hashes ([`da_render::mesh_id`]), assigned in one walk.
pub struct SceneMeshes {
    /// Hand to `Renderer::register_meshes` after (re-)expanding.
    pub registry: da_render::MeshRegistry,
    /// Hand to [`build_draw_list`] each frame (O(log n) lookup, no rehash).
    pub ids: MeshIds,
}

/// Albedo used when no ancestor set a base color.
pub const DEFAULT_ALBEDO: [f32; 3] = [0.6, 0.6, 0.6];

/// Map a graph shape to a renderer shape. `Mesh` maps to its deterministic
/// content-hash id ([`da_render::mesh_id`] — the id [`collect_meshes`]
/// registers under); `Capsule` is approximated by a cylinder of the
/// capsule's total height.
///
/// Hashing a mesh is O(its bytes) — fine at load, too hot per frame. The
/// frame path ([`leaf_to_item`]) resolves mesh ids from the precomputed
/// [`MeshIds`] cache instead of calling this on meshes.
pub fn map_shape(shape: &GraphShape) -> Option<RenderShape> {
    match *shape {
        GraphShape::Box { half_extents } => Some(RenderShape::Box { half: half_extents }),
        GraphShape::Cylinder { radius, height } => {
            Some(RenderShape::Cylinder { radius, height })
        }
        GraphShape::Sphere { radius } => Some(RenderShape::Sphere { radius }),
        GraphShape::Capsule { radius, height } => Some(RenderShape::Cylinder {
            radius,
            height: height + 2.0 * radius,
        }),
        GraphShape::Mesh {
            ref vertices,
            ref indices,
        } => Some(RenderShape::Mesh {
            id: da_render::mesh_id(vertices, indices),
        }),
    }
}

/// Collect every graph mesh in a scene: a renderer-side registry keyed by
/// the same ids [`map_shape`] assigns, plus the `(node, drawable) -> id`
/// cache the per-frame draw path reads. Call after (re-)expanding a zone;
/// hand `.registry` to `Renderer::register_meshes` and keep `.ids` for
/// [`build_draw_list`]. Mirrors `darkair::convert::collect_meshes` so the
/// editor and the game agree.
pub fn collect_meshes(scene: &Scene) -> SceneMeshes {
    let mut registry = da_render::MeshRegistry::new();
    let mut ids = MeshIds::new();
    for node in scene.nodes() {
        if let NodeKind::Geode(g) = node.kind() {
            for (i, d) in g.drawables.iter().enumerate() {
                if let GraphShape::Mesh { vertices, indices } = &d.shape {
                    ids.insert((node.id(), i), registry.insert(vertices, indices));
                }
            }
        }
    }
    SceneMeshes { registry, ids }
}

/// Albedo / emissive / glass extracted from an effective state set.
pub fn material_of(state: &StateSet) -> ([f32; 3], f32, bool) {
    let albedo = state
        .base_color
        .map(|c| [c.x, c.y, c.z])
        .unwrap_or(DEFAULT_ALBEDO);
    let emissive = state.emissive.map(|e| e.max_element()).unwrap_or(0.0);
    let glass = state.glass.unwrap_or(false);
    (albedo, emissive, glass)
}

/// Convert one render leaf into a draw item. `temps` supplies the display
/// temperature (the real [`crate::preview::ThermalPreview`] in the editor);
/// `mesh_ids` the precomputed id cache from [`collect_meshes`] (so meshes
/// cost an id lookup per frame, not a rehash of their geometry).
pub fn leaf_to_item(
    scene: &Scene,
    leaf: &RenderLeaf,
    mesh_ids: &MeshIds,
    temps: &impl TempSource,
) -> Option<DrawItem> {
    let node = scene.node(leaf.node)?;
    let NodeKind::Geode(geode) = node.kind() else {
        return None;
    };
    let drawable = geode.drawables.get(leaf.drawable)?;
    let shape = match drawable.shape {
        // Cached id — never rehash geometry on the frame path. The fallback
        // keeps meshes missing from the cache correct (just slow) rather
        // than invisible.
        GraphShape::Mesh {
            ref vertices,
            ref indices,
        } => RenderShape::Mesh {
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
        GraphShape::Cylinder { height, .. } => {
            leaf.world * glam::Mat4::from_translation(glam::Vec3::new(0.0, -height * 0.5, 0.0))
        }
        GraphShape::Capsule { radius, height } => {
            leaf.world
                * glam::Mat4::from_translation(glam::Vec3::new(
                    0.0,
                    -(height * 0.5 + radius),
                    0.0,
                ))
        }
        _ => leaf.world,
    };
    let (albedo, emissive, glass) = material_of(&leaf.state);
    Some(DrawItem {
        shape,
        world,
        albedo,
        emissive,
        temp_f: temps.temp_f(leaf.node, leaf.state.thermal.as_ref()),
        glass,
        coat_f: 0.0,
    })
}

/// Build the frame's draw list from a cull result. `env` supplies the
/// ambient / sky / moonlight globals; `temps` the per-node temperatures;
/// `mesh_ids` the expansion's precomputed mesh-id cache.
pub fn build_draw_list(
    scene: &Scene,
    leaves: &[RenderLeaf],
    mesh_ids: &MeshIds,
    env: &PreviewEnv,
    temps: &impl TempSource,
) -> DrawList {
    DrawList {
        items: leaves
            .iter()
            .filter_map(|leaf| leaf_to_item(scene, leaf, mesh_ids, temps))
            .collect(),
        ambient_f: env.ambient_f,
        sky_temp_f: env.sky_temp_f(),
        moonlight: env.moonlight(),
        heat_decals: Vec::new(),
        eyeshine: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_core::{Forecast, TempF};
    use da_graph::{CullVisitor, Drawable, StateSet, ThermalAttach};
    use glam::{Vec3, Vec4};

    fn test_scene() -> (Scene, da_core::NodeId) {
        let mut scene = Scene::new();
        let root = scene.root();
        let xf = scene
            .add_transform_at(root, Vec3::new(5.0, 0.0, -2.0))
            .unwrap();
        let geode = scene.add_geode(xf).unwrap();
        scene
            .set_state(
                geode,
                StateSet::new()
                    .with_base_color(Vec4::new(0.2, 0.4, 0.8, 1.0))
                    .with_emissive(Vec3::new(0.1, 0.9, 0.3))
                    .with_glass(true),
            )
            .unwrap();
        scene
            .add_drawable(
                geode,
                Drawable::new(GraphShape::Box {
                    half_extents: Vec3::new(1.0, 2.0, 3.0),
                }),
            )
            .unwrap();
        scene
            .add_drawable(
                geode,
                Drawable::new(GraphShape::Capsule {
                    radius: 0.5,
                    height: 1.0,
                }),
            )
            .unwrap();
        scene
            .add_drawable(
                geode,
                Drawable::new(GraphShape::Mesh {
                    vertices: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                    indices: vec![0, 1, 2],
                }),
            )
            .unwrap();
        scene
            .add_drawable(geode, Drawable::new(GraphShape::Sphere { radius: 2.0 }))
            .unwrap();
        (scene, geode)
    }

    #[test]
    fn shapes_and_materials_map_including_mesh() {
        let (scene, _) = test_scene();
        let leaves = CullVisitor::new(Vec3::new(0.0, 1.6, 10.0)).cull(&scene);
        assert_eq!(leaves.len(), 4, "cull sees all four drawables");

        let meshes = collect_meshes(&scene);
        let env = PreviewEnv::new(0.0, Forecast::Overcast);
        let list = build_draw_list(&scene, &leaves, &meshes.ids, &env, &env);
        assert_eq!(list.items.len(), 4, "every drawable maps, mesh included");

        // Box maps half extents straight through.
        match list.items[0].shape {
            RenderShape::Box { half } => assert_eq!(half, Vec3::new(1.0, 2.0, 3.0)),
            ref s => panic!("expected Box, got {s:?}"),
        }
        // Capsule approximated as a full-height cylinder.
        match list.items[1].shape {
            RenderShape::Cylinder { radius, height } => {
                assert_eq!(radius, 0.5);
                assert_eq!(height, 2.0);
            }
            ref s => panic!("expected Cylinder, got {s:?}"),
        }
        // Mesh maps to its content-hash id — stable across conversions and
        // the same id collect_meshes registers under.
        let mesh_item = match list.items[2].shape {
            RenderShape::Mesh { id } => id,
            ref s => panic!("expected Mesh, got {s:?}"),
        };
        let list2 = build_draw_list(&scene, &leaves, &meshes.ids, &env, &env);
        assert_eq!(list2.items[2].shape, RenderShape::Mesh { id: mesh_item });
        // The cached-id path and the from-scratch hash agree; an empty cache
        // still yields the same id via the rehash fallback.
        let list3 = build_draw_list(&scene, &leaves, &MeshIds::new(), &env, &env);
        assert_eq!(list3.items[2].shape, RenderShape::Mesh { id: mesh_item });
        assert_eq!(meshes.registry.len(), 1);
        assert!(
            meshes.registry.get(mesh_item).is_some(),
            "registry id matches map_shape id"
        );
        match list.items[3].shape {
            RenderShape::Sphere { radius } => assert_eq!(radius, 2.0),
            ref s => panic!("expected Sphere, got {s:?}"),
        }

        for item in &list.items {
            assert_eq!(item.albedo, [0.2, 0.4, 0.8], "base_color → albedo");
            assert_eq!(item.emissive, 0.9, "emissive is the max RGB component");
            assert!(item.glass, "glass flag passes through");
            // The transform above the geode lands in the world matrix.
            // Cylinder-rendered shapes (incl. capsule→cylinder) drop by half
            // their render height: the graph shape is centered, the unit
            // mesh base-anchored.
            let y = match item.shape {
                RenderShape::Cylinder { height, .. } => -height * 0.5,
                _ => 0.0,
            };
            assert_eq!(item.world.w_axis.truncate(), Vec3::new(5.0, y, -2.0));
            // No thermal attach → reads exactly ambient.
            assert_eq!(item.temp_f, env.ambient_f);
        }
        assert_eq!(list.ambient_f, env.ambient_f);
        assert_eq!(list.sky_temp_f, env.sky_temp_f());
    }

    #[test]
    fn thermal_attach_drives_item_temperature() {
        let (mut scene, geode) = test_scene();
        let state = scene.node(geode).unwrap().state().cloned().unwrap();
        scene
            .set_state(
                geode,
                state.with_thermal(ThermalAttach {
                    base_temp: TempF(90.0),
                    thermal_mass: 50.0,
                    sky_exposure: 0.0,
                }),
            )
            .unwrap();
        let leaves = CullVisitor::new(Vec3::new(0.0, 1.6, 10.0)).cull(&scene);
        let env = PreviewEnv::new(0.0, Forecast::Overcast); // ambient 68 at dusk
        let list = build_draw_list(&scene, &leaves, &collect_meshes(&scene).ids, &env, &env);
        // Full stored heat at dusk: 68 + (90 - 68) = 90.
        assert!((list.items[0].temp_f - 90.0).abs() < 1e-3);
    }

    #[test]
    fn unset_material_falls_back_to_defaults() {
        let mut scene = Scene::new();
        let geode = scene.add_geode(scene.root()).unwrap();
        scene
            .add_drawable(geode, Drawable::new(GraphShape::Sphere { radius: 1.0 }))
            .unwrap();
        let leaves = CullVisitor::new(Vec3::ZERO).cull(&scene);
        let env = PreviewEnv::new(0.3, Forecast::Clear);
        let list = build_draw_list(&scene, &leaves, &MeshIds::new(), &env, &env);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].albedo, DEFAULT_ALBEDO);
        assert_eq!(list.items[0].emissive, 0.0);
        assert!(!list.items[0].glass);
    }
}
