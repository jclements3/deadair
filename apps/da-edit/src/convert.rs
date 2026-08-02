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
//! | `Mesh{..}`             | skipped (no mesh registry in the editor) |
//!
//! Materials come from the leaf's fully-merged effective [`StateSet`]:
//! `base_color` → albedo, `emissive` → max RGB component, `glass` passes
//! through, and the display temperature comes from
//! [`crate::preview::PreviewEnv`].

use da_graph::{NodeKind, RenderLeaf, Scene, Shape as GraphShape, StateSet};
use da_render::draw::{DrawItem, DrawList, Shape as RenderShape};

use crate::preview::PreviewEnv;

/// Albedo used when no ancestor set a base color.
pub const DEFAULT_ALBEDO: [f32; 3] = [0.6, 0.6, 0.6];

/// Map a graph shape to a renderer shape. `Mesh` returns `None` (skipped);
/// `Capsule` is approximated by a cylinder of the capsule's total height.
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
        GraphShape::Mesh { .. } => None,
    }
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

/// Convert one render leaf into a draw item, or `None` for shapes the
/// editor cannot draw (meshes).
pub fn leaf_to_item(scene: &Scene, leaf: &RenderLeaf, env: &PreviewEnv) -> Option<DrawItem> {
    let node = scene.node(leaf.node)?;
    let NodeKind::Geode(geode) = node.kind() else {
        return None;
    };
    let drawable = geode.drawables.get(leaf.drawable)?;
    let shape = map_shape(&drawable.shape)?;
    let (albedo, emissive, glass) = material_of(&leaf.state);
    Some(DrawItem {
        shape,
        world: leaf.world,
        albedo,
        emissive,
        temp_f: env.display_temp_f(leaf.state.thermal.as_ref()),
        glass,
    })
}

/// Build the frame's draw list from a cull result.
pub fn build_draw_list(scene: &Scene, leaves: &[RenderLeaf], env: &PreviewEnv) -> DrawList {
    DrawList {
        items: leaves
            .iter()
            .filter_map(|leaf| leaf_to_item(scene, leaf, env))
            .collect(),
        ambient_f: env.ambient_f,
        sky_temp_f: env.sky_temp_f(),
        moonlight: env.moonlight(),
        heat_decals: Vec::new(),
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
    fn shapes_and_materials_map_and_mesh_is_skipped() {
        let (scene, _) = test_scene();
        let leaves = CullVisitor::new(Vec3::new(0.0, 1.6, 10.0)).cull(&scene);
        assert_eq!(leaves.len(), 4, "cull sees all four drawables");

        let env = PreviewEnv::new(0.0, Forecast::Overcast);
        let list = build_draw_list(&scene, &leaves, &env);
        assert_eq!(list.items.len(), 3, "mesh drawable is skipped");

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
        match list.items[2].shape {
            RenderShape::Sphere { radius } => assert_eq!(radius, 2.0),
            ref s => panic!("expected Sphere, got {s:?}"),
        }

        for item in &list.items {
            assert_eq!(item.albedo, [0.2, 0.4, 0.8], "base_color → albedo");
            assert_eq!(item.emissive, 0.9, "emissive is the max RGB component");
            assert!(item.glass, "glass flag passes through");
            // The transform above the geode lands in the world matrix.
            assert_eq!(item.world.w_axis.truncate(), Vec3::new(5.0, 0.0, -2.0));
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
        let list = build_draw_list(&scene, &leaves, &env);
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
        let list = build_draw_list(&scene, &leaves, &env);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].albedo, DEFAULT_ALBEDO);
        assert_eq!(list.items[0].emissive, 0.0);
        assert!(!list.items[0].glass);
    }
}
