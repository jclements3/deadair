//! `VimProp` integration: `.vim`-authored CSG props expanding into zones
//! (compile via da-csg, mesh Y-up, one part per prop, deterministic).

use std::path::PathBuf;

use da_graph::{Drawable, NodeId, NodeKind, Scene, Shape};
use da_param::{
    expand_zone, load_zone_file, parse_zone_str, resolve_vim_sources, ParamError, ZoneSource,
};

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

/// The drawables of the geode node `id`.
fn geode_drawables(scene: &Scene, id: NodeId) -> &[Drawable] {
    match scene.node(id).expect("node resolves").kind() {
        NodeKind::Geode(g) => &g.drawables,
        other => panic!("expected a geode, got {other:?}"),
    }
}

/// A minimal zone carrying one `VimProp`, with the script text supplied
/// inline (the way the loader would after `resolve_vim_sources`).
fn prop_zone(vim_text: &str) -> ZoneSource {
    let mut src = parse_zone_str(
        r#"ZoneSource(
            name: "PropTest",
            seed: 42,
            size_m: (40.0, 40.0),
            ambient_biome: Grass,
            features: [
                VimProp(src: "props/test.vim", pos: (20.0, 0.0, 20.0),
                        yaw_deg: 30.0, scale: 1.5, thermal: MetalRoof,
                        name: "TestProp"),
            ],
        )"#,
    )
    .expect("prop zone parses");
    src.vim_sources
        .insert("props/test.vim".to_owned(), vim_text.to_owned());
    src
}

const CUT_CUBE: &str = "let b = box(2, 2, 2)\nlet d = cylinder(r = 0.5, h = 3)\nmodel b - d";

#[test]
fn every_shipped_prop_script_compiles() {
    let dir = assets_dir().join("props");
    let mut checked = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("assets/props exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "vim"))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path).expect("prop script reads");
        let compiled = da_csg::compile_vim(&text)
            .unwrap_or_else(|e| panic!("{} failed to compile: {e}", path.display()));
        let (verts, idx) = compiled.solid.to_mesh_yup();
        assert!(
            !verts.is_empty() && idx.len() >= 3,
            "{}: empty mesh",
            path.display()
        );
        checked += 1;
    }
    assert!(checked >= 2, "expected at least 2 shipped props, got {checked}");
}

#[test]
fn shipped_props_stand_on_the_ground_with_sane_polycounts() {
    for name in ["fluted_silo.vim", "water_tower.vim"] {
        let path = assets_dir().join("props").join(name);
        let text = std::fs::read_to_string(&path).expect("prop script reads");
        let compiled = da_csg::compile_vim(&text).expect("compiles");
        let (verts, idx) = compiled.solid.to_mesh_yup();
        let min_y = verts.iter().map(|v| v.y).fold(f32::INFINITY, f32::min);
        let max_y = verts.iter().map(|v| v.y).fold(f32::NEG_INFINITY, f32::max);
        assert!(
            min_y > -0.01,
            "{name}: base below ground, min_y = {min_y}"
        );
        assert!(max_y > 5.0, "{name}: too short, max_y = {max_y}");
        assert!(
            idx.len() / 3 < 30_000,
            "{name}: {} triangles is unreasonable",
            idx.len() / 3
        );
    }
}

#[test]
fn vim_prop_expands_to_one_mesh_part_with_thermal_state() {
    let zone = expand_zone(&prop_zone(CUT_CUBE)).expect("expands");
    let root = zone
        .scene
        .find_by_name("TestProp")
        .expect("prop root carries the authored name");
    let node = zone.scene.node(root).expect("node resolves");
    assert_eq!(node.children().len(), 1, "one part per prop");
    let geode = node.children()[0];
    let state = zone.scene.effective_state(geode);
    let thermal = state.thermal.expect("prop carries a thermal attach");
    assert!(
        thermal.sky_exposure > 0.7,
        "MetalRoof preset is sky-exposed, got {}",
        thermal.sky_exposure
    );
    // The single drawable is a triangle mesh with real triangles.
    let drawables = geode_drawables(&zone.scene, geode);
    assert_eq!(drawables.len(), 1);
    let Shape::Mesh { vertices, indices } = &drawables[0].shape else {
        panic!("prop drawable must be a mesh, got {:?}", drawables[0].shape);
    };
    assert!(!vertices.is_empty());
    assert!(indices.len() >= 3 && indices.len() % 3 == 0);
}

#[test]
fn vim_prop_expansion_is_byte_identical() {
    let src = prop_zone(CUT_CUBE);
    let a = expand_zone(&src).expect("first expansion");
    let b = expand_zone(&src).expect("second expansion");
    assert_eq!(
        a.scene.to_ron().expect("ron a"),
        b.scene.to_ron().expect("ron b"),
        "same source must expand byte-identically"
    );
}

#[test]
fn broken_vim_source_errors_with_the_path() {
    let err = expand_zone(&prop_zone("model wibble(3)")).expect_err("must fail");
    let ParamError::VimCompile { ref path, ref message } = err else {
        panic!("expected VimCompile, got {err:?}");
    };
    assert_eq!(path, "props/test.vim");
    assert!(message.contains("wibble"), "DSL error passes through: {message}");
    let shown = err.to_string();
    assert!(
        shown.contains("props/test.vim") && shown.contains("wibble"),
        "display mentions path and cause: {shown}"
    );
}

#[test]
fn unresolved_vim_source_is_a_clear_error() {
    let mut src = prop_zone(CUT_CUBE);
    src.vim_sources.clear();
    let err = expand_zone(&src).expect_err("must fail");
    assert!(matches!(err, ParamError::VimMissing { .. }));
    assert!(err.to_string().contains("props/test.vim"));
}

#[test]
fn resolve_vim_sources_inlines_from_the_assets_dir() {
    let mut src = parse_zone_str(
        r#"ZoneSource(
            name: "ResolveTest",
            seed: 7,
            size_m: (60.0, 60.0),
            ambient_biome: Gravel,
            features: [
                VimProp(src: "props/water_tower.vim", pos: (30.0, 0.0, 30.0)),
            ],
        )"#,
    )
    .expect("parses");
    assert!(src.vim_sources.is_empty());
    resolve_vim_sources(&mut src, assets_dir()).expect("resolves");
    assert!(src.vim_sources.contains_key("props/water_tower.vim"));
    let zone = expand_zone(&src).expect("expands");
    // Defaulted name: the subgraph root is called "VimProp".
    assert!(zone.scene.find_by_name("VimProp").is_some());
}

#[test]
fn grain_coop_carries_both_vim_props_via_the_loader() {
    let path = assets_dir().join("zones/grain_coop.zone.ron");
    let src = load_zone_file(path).expect("grain co-op loads");
    assert_eq!(src.vim_sources.len(), 2, "loader inlined both scripts");
    let zone = expand_zone(&src).expect("expands");
    for name in ["FlutedSilo", "WaterTower"] {
        let root = zone
            .scene
            .find_by_name(name)
            .unwrap_or_else(|| panic!("{name} missing from grain co-op"));
        let geode = zone.scene.node(root).expect("node").children()[0];
        let drawables = geode_drawables(&zone.scene, geode);
        assert!(matches!(drawables[0].shape, Shape::Mesh { .. }));
    }
}
