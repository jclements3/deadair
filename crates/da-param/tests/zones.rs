//! Integration tests: the six shipped zone sources must parse, expand
//! deterministically, and carry the right thermal/material state.

use std::path::PathBuf;

use da_graph::Scene;
use da_param::{
    expand_zone, load_all_zones, load_zone_file, Biome, Feature, FriendlyBehavior, HazardKind,
    ParamError, Species, SpawnRef, SpawnTable, Volume, ZoneExpansion, ZoneSource,
};
use glam::Vec3;

fn zones_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/zones")
}

fn load(name: &str) -> ZoneSource {
    load_zone_file(zones_dir().join(name)).expect("zone file loads")
}

fn expand(name: &str) -> ZoneExpansion {
    expand_zone(&load(name)).expect("zone expands")
}

/// Names of all nodes in creation order (unnamed slots skipped).
fn names(scene: &Scene) -> Vec<String> {
    scene
        .nodes()
        .filter_map(|n| n.name().map(str::to_owned))
        .collect()
}

/// The single geode child of the first node named `name` under (and
/// including) `root`'s subtree; panics if absent.
fn geode_state_of(scene: &Scene, name: &str) -> da_graph::StateSet {
    let t = scene.find_by_name(name).expect("named node exists");
    let node = scene.node(t).expect("node resolves");
    let geode = node.children()[0];
    scene.effective_state(geode)
}

const ALL: [&str; 6] = [
    "creek_bottom.zone.ron",
    "grain_coop.zone.ron",
    "home_farm.zone.ron",
    "main_street.zone.ron",
    "orchard.zone.ron",
    "town_edge.zone.ron",
];

#[test]
fn all_six_zone_sources_parse() {
    let zones = load_all_zones(zones_dir()).expect("directory loads");
    assert_eq!(zones.len(), 6);
    let mut zone_names: Vec<&str> = zones.iter().map(|z| z.name.as_str()).collect();
    zone_names.sort_unstable();
    assert_eq!(
        zone_names,
        [
            "Creek Bottom",
            "Grain Co-op",
            "Home Farm",
            "Main Street",
            "Orchard",
            "Town Edge"
        ]
    );
}

#[test]
fn every_zone_expands_to_a_nontrivial_scene() {
    for file in ALL {
        let zone = expand(file);
        assert!(
            zone.scene.len() > 50,
            "{file}: expected > 50 nodes, got {}",
            zone.scene.len()
        );
        assert!(!zone.spawn_points.is_empty(), "{file}: no spawn points");
    }
}

#[test]
fn double_expansion_is_byte_identical_for_every_zone() {
    for file in ALL {
        let src = load(file);
        let a = expand_zone(&src).expect("first expansion");
        let b = expand_zone(&src).expect("second expansion");
        let ra = a.scene.to_ron().expect("to_ron a");
        let rb = b.scene.to_ron().expect("to_ron b");
        assert_eq!(ra, rb, "{file}: scene RON not byte-identical");
        assert_eq!(a.spawn_points, b.spawn_points, "{file}: spawn points differ");
        assert_eq!(a.patrol_routes, b.patrol_routes, "{file}: patrols differ");
        assert_eq!(
            a.hazard_volumes, b.hazard_volumes,
            "{file}: hazards differ"
        );
    }
}

#[test]
fn seed_change_keeps_structure_but_moves_jitter() {
    for file in ALL {
        let src = load(file);
        let mut reseeded = src.clone();
        reseeded.seed = src.seed + 1;
        let a = expand_zone(&src).expect("expand original");
        let b = expand_zone(&reseeded).expect("expand reseeded");
        // Structure: identical node count and identical name sequence.
        assert_eq!(a.scene.len(), b.scene.len(), "{file}: node count changed");
        assert_eq!(names(&a.scene), names(&b.scene), "{file}: names changed");
        // Jitter: the serialized scenes must not be identical.
        let ra = a.scene.to_ron().expect("to_ron a");
        let rb = b.scene.to_ron().expect("to_ron b");
        assert_ne!(ra, rb, "{file}: reseeding did not move any jitter");
    }
}

#[test]
fn metal_barn_roof_is_sky_exposed_but_walls_are_not() {
    let zone = expand("home_farm.zone.ron");
    let roof = geode_state_of(&zone.scene, "BarnRoofPanel");
    let wall = geode_state_of(&zone.scene, "BarnWall");
    let roof_t = roof.thermal.expect("roof has thermal attach");
    let wall_t = wall.thermal.expect("wall has thermal attach");
    assert!(
        roof_t.sky_exposure > 0.7,
        "metal roof sky_exposure = {}",
        roof_t.sky_exposure
    );
    assert!(
        wall_t.sky_exposure <= 0.7,
        "wall sky_exposure = {}",
        wall_t.sky_exposure
    );
    // Thin metal cools much faster than masonry.
    assert!(roof_t.thermal_mass < wall_t.thermal_mass);
}

#[test]
fn storefront_glass_pane_is_thermally_opaque_glass() {
    let zone = expand("main_street.zone.ron");
    let panes: Vec<_> = zone
        .scene
        .nodes()
        .filter(|n| n.name() == Some("StorefrontGlass"))
        .map(|n| n.id())
        .collect();
    assert_eq!(panes.len(), 4, "main street has four glass storefronts");
    for t in panes {
        let node = zone.scene.node(t).expect("pane node");
        let state = zone.scene.effective_state(node.children()[0]);
        assert_eq!(state.glass, Some(true));
        assert!(state.thermal.is_some());
    }
}

#[test]
fn spawn_table_feature_refs_resolve_in_all_six_zones() {
    for file in ALL {
        let src = load(file);
        let zone = expand_zone(&src).unwrap_or_else(|e| panic!("{file}: {e}"));
        // Every node-based table contributed exactly base_count points.
        let expected: u32 = src.spawn_tables.iter().map(|t| t.base_count).sum();
        assert_eq!(
            zone.spawn_points.len() as u32,
            expected,
            "{file}: spawn point count"
        );
    }
}

#[test]
fn dangling_feature_ref_is_an_error() {
    let src = ZoneSource {
        name: "Broken".to_owned(),
        seed: 1,
        size_m: (10.0, 10.0),
        ambient_biome: Biome::Grass,
        features: Vec::new(),
        spawn_tables: vec![SpawnTable {
            species: Species::Rat,
            nodes: vec![SpawnRef::Feature("Nope".to_owned())],
            patrol: Vec::new(),
            base_count: 1,
            elevated: false,
        }],
        friendlies: Vec::new(),
        hazards: Vec::new(),
        zombie_weight: 0.0,
        connections: Vec::new(),
        contracts_hint: Vec::new(),
        vim_sources: Default::default(),
    };
    match expand_zone(&src) {
        Err(ParamError::UnresolvedFeature { reference, .. }) => assert_eq!(reference, "Nope"),
        other => panic!("expected UnresolvedFeature, got {other:?}"),
    }
}

#[test]
fn fence_post_count_matches_length_over_gap() {
    let src = load("home_farm.zone.ron");
    let zone = expand_zone(&src).expect("expands");
    // First FenceLine in the source: (10,0,10) → (90,0,10), gap 3.
    let (from, to, gap) = src
        .features
        .iter()
        .find_map(|f| match f {
            Feature::FenceLine {
                from,
                to,
                post_gap_m,
            } => Some((*from, *to, *post_gap_m)),
            _ => None,
        })
        .expect("home farm has a fence");
    let len = (Vec3::new(to.0, to.1, to.2) - Vec3::new(from.0, from.1, from.2)).length();
    let expected = (len / gap).floor() as usize + 1;
    assert_eq!(expected, 27, "sanity: 80 m / 3 m spacing");
    // Count posts under the first FenceLine subgraph only.
    let fence_root = zone
        .scene
        .find_by_name("FenceLine")
        .expect("fence root exists");
    let posts = zone
        .scene
        .node(fence_root)
        .expect("fence node")
        .children()
        .iter()
        .filter(|&&c| zone.scene.node(c).and_then(|n| n.name()) == Some("FencePost"))
        .count();
    assert_eq!(posts, expected);
}

#[test]
fn streetlight_heads_are_emissive() {
    let src = load("main_street.zone.ron");
    let zone = expand_zone(&src).expect("expands");
    let (from, to, gap) = src
        .features
        .iter()
        .find_map(|f| match f {
            Feature::StreetlightRow { from, to, gap_m } => Some((*from, *to, *gap_m)),
            _ => None,
        })
        .expect("main street has streetlights");
    let len = (Vec3::new(to.0, to.1, to.2) - Vec3::new(from.0, from.1, from.2)).length();
    let expected = (len / gap).floor() as usize + 1;
    let heads: Vec<_> = zone
        .scene
        .nodes()
        .filter(|n| n.name() == Some("StreetlightHead"))
        .map(|n| n.id())
        .collect();
    assert_eq!(heads.len(), expected);
    for h in heads {
        let node = zone.scene.node(h).expect("head node");
        let state = zone.scene.effective_state(node.children()[0]);
        let emissive = state.emissive.expect("head is emissive");
        assert!(emissive.length() > 0.0, "emissive color is non-zero");
    }
}

#[test]
fn migrated_builtin_features_emit_per_part_meshes_with_distinct_states() {
    // The Silo generator now expands the assets/props/builtin/silo.vim
    // template: one Shape::Mesh node per part tag, each with its own
    // material/thermal StateSet (multi-material contract).
    let zone = expand("grain_coop.zone.ron");
    let silo = zone.scene.find_by_name("Silo").expect("grain co-op has silos");
    let child_names: Vec<String> = zone
        .scene
        .node(silo)
        .expect("silo node")
        .children()
        .iter()
        .filter_map(|&c| zone.scene.node(c).and_then(|n| n.name()).map(str::to_owned))
        .collect();
    assert_eq!(
        child_names,
        ["SiloBarrel", "SiloDome", "SiloChute"],
        "one node per .vim part, in part order"
    );
    for name in ["SiloBarrel", "SiloDome", "SiloChute"] {
        let t = zone.scene.find_by_name(name).expect("part node");
        let geode = zone.scene.node(t).expect("node").children()[0];
        let node = zone.scene.node(geode).expect("geode");
        let da_graph::NodeKind::Geode(g) = node.kind() else {
            panic!("{name}: expected geode");
        };
        assert!(
            matches!(g.drawables[0].shape, da_graph::Shape::Mesh { .. }),
            "{name}: builtin geometry must be a compiled mesh"
        );
    }
    // Distinct per-part states on one object: the dome reads as thin
    // sky-facing metal, and its tint differs from the barrel's sheet metal.
    let dome = geode_state_of(&zone.scene, "SiloDome");
    let barrel = geode_state_of(&zone.scene, "SiloBarrel");
    assert!(dome.thermal.expect("dome thermal").sky_exposure > 0.7);
    assert_ne!(dome.base_color, barrel.base_color);

    // Streetlight lamp unit (main street): the head part keeps its emissive
    // lamp-glass state while the pole part stays plain metal — genuinely
    // different thermal attaches on parts of the same compiled solid.
    let zone = expand("main_street.zone.ron");
    let head = geode_state_of(&zone.scene, "StreetlightHead");
    let pole = geode_state_of(&zone.scene, "StreetlightPole");
    assert!(head.emissive.expect("lamp head is emissive").length() > 0.0);
    assert!(pole.emissive.is_none(), "pole must not glow");
    assert_ne!(
        head.thermal.expect("head thermal").thermal_mass,
        pole.thermal.expect("pole thermal").thermal_mass,
        "head (glass) and pole (metal) carry different thermal presets"
    );
    // The mast beacon likewise survives as its own emissive part.
    let beacon = geode_state_of(&zone.scene, "MastBeacon");
    assert!(beacon.emissive.expect("beacon emissive").length() > 0.0);
}

#[test]
fn orchard_possums_spawn_elevated_at_canopy_height() {
    let zone = expand("orchard.zone.ron");
    let possums: Vec<_> = zone
        .spawn_points
        .iter()
        .filter(|s| s.species == "Possum")
        .collect();
    assert_eq!(possums.len(), 7);
    for p in possums {
        assert!(p.elevated, "orchard possum spawns are elevated");
        assert!(p.pos.y > 2.0, "canopy height, got y = {}", p.pos.y);
    }
    // Ground species stay on the ground.
    assert!(zone
        .spawn_points
        .iter()
        .filter(|s| s.species == "Rat")
        .all(|s| !s.elevated && s.pos.y == 0.0));
}

#[test]
fn home_farm_expands_rabbit_spawn_points_on_the_ground() {
    let zone = expand("home_farm.zone.ron");
    let rabbits: Vec<_> = zone
        .spawn_points
        .iter()
        .filter(|s| s.species == "Rabbit")
        .collect();
    assert_eq!(rabbits.len(), 6, "home farm fields carry six rabbits");
    for r in &rabbits {
        assert!(!r.elevated, "rabbits are ground game");
        assert_eq!(r.pos.y, 0.0, "ground spawn, got y = {}", r.pos.y);
    }
    // The other rabbit zones carry their authored counts too.
    let count = |file: &str| {
        expand(file)
            .spawn_points
            .iter()
            .filter(|s| s.species == "Rabbit")
            .count()
    };
    assert_eq!(count("orchard.zone.ron"), 5);
    assert_eq!(count("grain_coop.zone.ron"), 4);
}

#[test]
fn cow_pen_expands_to_fence_and_static_positions() {
    let zone = expand("home_farm.zone.ron");
    let cows = zone
        .friendly_setups
        .iter()
        .find(|f| f.species == "Cow")
        .expect("home farm has cows");
    assert_eq!(cows.count, 4);
    let FriendlyBehavior::Penned {
        corner,
        size,
        positions,
    } = &cows.behavior
    else {
        panic!("cows are penned, got {:?}", cows.behavior);
    };
    assert_eq!(*corner, Vec3::new(10.0, 0.0, 30.0));
    assert_eq!(*size, (20.0, 15.0));
    assert_eq!(positions.len(), 4);
    for p in positions {
        assert!(p.x >= corner.x && p.x <= corner.x + size.0, "inside pen x");
        assert!(p.z >= corner.z && p.z <= corner.z + size.1, "inside pen z");
    }
    // The pen fence is real scene geometry.
    let pen = zone.scene.find_by_name("Pen").expect("pen subgraph exists");
    let pen_posts = zone
        .scene
        .node(pen)
        .expect("pen node")
        .children()
        .iter()
        .filter(|&&c| zone.scene.node(c).and_then(|n| n.name()) == Some("PenPost"))
        .count();
    assert!(pen_posts > 8, "pen has a fence, got {pen_posts} posts");
}

#[test]
fn along_hazards_resolve_to_the_creek_polyline() {
    let zone = expand("creek_bottom.zone.ron");
    let bank = zone
        .hazard_volumes
        .iter()
        .find(|h| h.kind == HazardKind::CreekBank)
        .expect("creek bank hazard");
    let water = zone
        .hazard_volumes
        .iter()
        .find(|h| h.kind == HazardKind::Water)
        .expect("water hazard");
    let Volume::Polyline { points, width } = &bank.volume else {
        panic!("bank should be a polyline");
    };
    assert_eq!(points.len(), 4);
    assert_eq!(*width, 9.0, "creek width 7 m + 1 m bank each side");
    let Volume::Polyline { points, width } = &water.volume else {
        panic!("water should be a polyline");
    };
    assert_eq!(points.len(), 4);
    assert_eq!(*width, 7.0);
}

#[test]
fn passthrough_fields_survive_expansion() {
    let zone = expand("grain_coop.zone.ron");
    assert_eq!(zone.ground_biome, Biome::Gravel);
    assert!((zone.zombie_weight - 0.15).abs() < f32::EPSILON);
    assert_eq!(zone.connections.len(), 3);
    // Raccoon patrol route from the spawn table.
    let (species, route) = &zone.patrol_routes[0];
    assert_eq!(species, "Raccoon");
    assert_eq!(route.len(), 3);
}
