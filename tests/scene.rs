//! Integration tests for scene serialisation and the scene editor.

use deadair::{
    editor::SceneEditor,
    scene::{NodeKind, Scene},
    world::World,
};

// ── Scene serialisation ───────────────────────────────────────────────────────

#[test]
fn abandoned_farm_round_trips_through_json() {
    let original = Scene::abandoned_farm();
    let json = original.to_json().expect("should serialise");
    let restored = Scene::from_json(&json).expect("should deserialise");

    assert_eq!(original.name, restored.name);
    assert_eq!(original.nodes.len(), restored.nodes.len());
    assert!((original.ambient_temp_c - restored.ambient_temp_c).abs() < 0.001);
}

#[test]
fn abandoned_farm_has_correct_node_counts() {
    let s = Scene::abandoned_farm();
    let zombies = s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Zombie { .. })).count();
    let spawns  = s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::HunterSpawn)).count();
    let terrain = s.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Terrain { .. })).count();
    assert_eq!(zombies, 3, "Expected 3 zombies");
    assert_eq!(spawns,  1, "Expected 1 hunter spawn");
    assert_eq!(terrain, 1, "Expected 1 terrain node");
}

#[test]
fn world_from_scene_spawns_correct_entities() {
    let scene = Scene::abandoned_farm();
    let world = World::from_scene(&scene);
    assert_eq!(world.zombie_count(), 3, "Expected 3 zombies");
    assert_eq!(world.hunters().count(), 1, "Expected 1 hunter");
}

#[test]
fn zombie_temperature_is_near_ambient() {
    let scene = Scene::abandoned_farm(); // ambient = 5 °C
    let world = World::from_scene(&scene);
    for zombie in world.zombies() {
        let delta_t = (zombie.temperature_c - scene.ambient_temp_c).abs();
        assert!(delta_t < 1.0,
            "Zombie ΔT should be < 1 °C above ambient (cold!), got {delta_t:.2} °C");
    }
}

// ── Scene editor ──────────────────────────────────────────────────────────────

#[test]
fn editor_add_zombie_increases_node_count() {
    let mut editor = SceneEditor::with_default_scene();
    let before = editor.scene.nodes.len();
    editor.add_zombie(60.0, 60.0, 0.9);
    assert_eq!(editor.scene.nodes.len(), before + 1);
    assert!(editor.dirty);
}

#[test]
fn editor_remove_returns_true_for_existing_id() {
    let mut editor = SceneEditor::with_default_scene();
    let found = editor.remove("zombie_0");
    assert!(found, "zombie_0 should exist in the abandoned farm scene");
    assert!(editor.dirty);
    // Node should no longer appear
    assert!(!editor.scene.nodes.iter().any(|n| n.id.as_deref() == Some("zombie_0")));
}

#[test]
fn editor_remove_returns_false_for_missing_id() {
    let mut editor = SceneEditor::with_default_scene();
    let found = editor.remove("does_not_exist");
    assert!(!found);
    assert!(!editor.dirty);
}

#[test]
fn editor_move_repositions_node() {
    let mut editor = SceneEditor::with_default_scene();
    let ok = editor.move_node("zombie_0", 10.0, 20.0, 0.0);
    assert!(ok);
    let node = editor.scene.nodes.iter().find(|n| n.id.as_deref() == Some("zombie_0")).unwrap();
    let pos = node.translate.unwrap();
    assert!((pos.x - 10.0).abs() < 0.001);
    assert!((pos.y - 20.0).abs() < 0.001);
}

#[test]
fn editor_save_and_load_preserves_scene() {
    let mut editor = SceneEditor::with_default_scene();
    editor.add_zombie(77.0, 88.0, 1.0);

    let json = editor.save_to_json().expect("should serialise");

    let mut fresh = SceneEditor::new(Scene::new("empty", 0.0));
    fresh.load_from_json(&json).expect("should deserialise");

    assert_eq!(editor.scene.nodes.len(), fresh.scene.nodes.len());
    assert_eq!(editor.scene.name, fresh.scene.name);
    assert!(!fresh.dirty);
}

#[test]
fn editor_render_map_is_non_empty() {
    let editor = SceneEditor::with_default_scene();
    let map = editor.render_map();
    assert!(map.contains('Z'), "Map should contain zombie marker 'Z'");
    assert!(map.contains('H'), "Map should contain hunter spawn marker 'H'");
}

#[test]
fn editor_list_nodes_shows_all_nodes() {
    let editor = SceneEditor::with_default_scene();
    let listing = editor.list_nodes();
    assert!(listing.contains("zombie_0"));
    assert!(listing.contains("hunter_spawn"));
    assert!(listing.contains("barn"));
}
