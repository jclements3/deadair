//! Integration tests for the da-graph scene graph.

use da_graph::prelude::*;
use glam::{Mat4, Quat, Vec3, Vec4};

fn sphere(radius: f32) -> Drawable {
    Drawable::new(Shape::Sphere { radius })
}

/// root -> xform(t) -> geode with one sphere drawable of radius r.
fn scene_with_sphere_at(t: Vec3, r: f32) -> (Scene, NodeId, NodeId) {
    let mut s = Scene::new();
    let x = s.add_transform_at(s.root(), t).unwrap();
    let g = s.add_geode(x).unwrap();
    s.add_drawable(g, sphere(r)).unwrap();
    (s, x, g)
}

// ---------------------------------------------------------------------
// Traversal order and visitor dispatch
// ---------------------------------------------------------------------

#[derive(Default)]
struct Recorder {
    events: Vec<String>,
    prune: Option<String>,
}

impl Visitor for Recorder {
    fn enter(&mut self, _s: &Scene, n: &Node) -> bool {
        let name = n.name().unwrap_or("?").to_owned();
        self.events.push(format!("+{name}"));
        self.prune.as_deref() != Some(&name)
    }
    fn leave(&mut self, _s: &Scene, n: &Node) {
        self.events.push(format!("-{}", n.name().unwrap_or("?")));
    }
}

fn build_named_tree() -> Scene {
    // root
    // ├── a (group)
    // │   ├── a1 (geode)
    // │   └── a2 (transform)
    // └── b (group)
    //     └── b1 (geode)
    let mut s = Scene::new();
    let a = s.add_group(s.root()).unwrap();
    s.set_name(a, Some("a".into())).unwrap();
    let a1 = s.add_geode(a).unwrap();
    s.set_name(a1, Some("a1".into())).unwrap();
    let a2 = s.add_transform(a).unwrap();
    s.set_name(a2, Some("a2".into())).unwrap();
    let b = s.add_group(s.root()).unwrap();
    s.set_name(b, Some("b".into())).unwrap();
    let b1 = s.add_geode(b).unwrap();
    s.set_name(b1, Some("b1".into())).unwrap();
    s
}

#[test]
fn traversal_is_preorder_enter_postorder_leave() {
    let s = build_named_tree();
    let mut v = Recorder::default();
    visit_depth_first(&s, s.root(), &mut v);
    assert_eq!(
        v.events,
        vec!["+root", "+a", "+a1", "-a1", "+a2", "-a2", "-a", "+b", "+b1", "-b1", "-b", "-root"]
    );
}

#[test]
fn enter_false_prunes_subtree_but_still_leaves() {
    let s = build_named_tree();
    let mut v = Recorder {
        prune: Some("a".into()),
        ..Default::default()
    };
    visit_depth_first(&s, s.root(), &mut v);
    assert_eq!(
        v.events,
        vec!["+root", "+a", "-a", "+b", "+b1", "-b1", "-b", "-root"]
    );
}

#[test]
fn per_kind_hooks_dispatch_by_node_kind() {
    #[derive(Default)]
    struct Counter {
        groups: usize,
        transforms: usize,
        geodes: usize,
        switches: usize,
        lods: usize,
    }
    impl Visitor for Counter {
        fn enter_group(&mut self, _: &Scene, _: &Node) -> bool {
            self.groups += 1;
            true
        }
        fn enter_transform(&mut self, _: &Scene, _: &Node) -> bool {
            self.transforms += 1;
            true
        }
        fn enter_geode(&mut self, _: &Scene, _: &Node) -> bool {
            self.geodes += 1;
            true
        }
        fn enter_switch(&mut self, _: &Scene, _: &Node) -> bool {
            self.switches += 1;
            true
        }
        fn enter_lod(&mut self, _: &Scene, _: &Node) -> bool {
            self.lods += 1;
            true
        }
    }

    let mut s = build_named_tree(); // root + 2 groups + 1 transform + 2 geodes
    let sw = s.add_switch(s.root()).unwrap();
    s.add_lod(sw).unwrap();

    let mut c = Counter::default();
    visit_depth_first(&s, s.root(), &mut c);
    assert_eq!(c.groups, 3); // root, a, b
    assert_eq!(c.transforms, 1);
    assert_eq!(c.geodes, 2);
    assert_eq!(c.switches, 1);
    assert_eq!(c.lods, 1);
}

// ---------------------------------------------------------------------
// State inheritance
// ---------------------------------------------------------------------

#[test]
fn effective_state_is_override_merge_of_ancestors() {
    let mut s = Scene::new();
    let mid = s.add_group(s.root()).unwrap();
    let leaf = s.add_geode(mid).unwrap();

    s.set_state(
        s.root(),
        StateSet::new()
            .with_base_color(Vec4::new(1.0, 0.0, 0.0, 1.0))
            .with_metallic(0.9)
            .with_glass(false),
    )
    .unwrap();
    s.set_state(
        mid,
        StateSet::new()
            .with_base_color(Vec4::new(0.0, 1.0, 0.0, 1.0))
            .with_roughness(0.4),
    )
    .unwrap();

    let eff = s.effective_state(leaf);
    // Child override wins.
    assert_eq!(eff.base_color, Some(Vec4::new(0.0, 1.0, 0.0, 1.0)));
    // Unset on child: inherited from root.
    assert_eq!(eff.metallic, Some(0.9));
    assert_eq!(eff.glass, Some(false));
    // Introduced mid-path.
    assert_eq!(eff.roughness, Some(0.4));
    // Never set anywhere.
    assert_eq!(eff.emissive, None);
    assert_eq!(eff.thermal, None);
}

#[test]
fn thermal_attach_inherits_and_overrides() {
    let mut s = Scene::new();
    let barn = s.add_group(s.root()).unwrap();
    let roof = s.add_geode(barn).unwrap();
    let wall = s.add_geode(barn).unwrap();

    let barn_thermal = ThermalAttach {
        base_temp: TempF(55.0),
        thermal_mass: 100.0,
        sky_exposure: 0.2,
    };
    let roof_thermal = ThermalAttach {
        base_temp: TempF(48.0),
        thermal_mass: 5.0,
        sky_exposure: 0.95, // metal roof: high sky exposure
    };
    s.set_state(barn, StateSet::new().with_thermal(barn_thermal))
        .unwrap();
    s.set_state(roof, StateSet::new().with_thermal(roof_thermal))
        .unwrap();

    assert_eq!(s.effective_state(wall).thermal, Some(barn_thermal));
    assert_eq!(s.effective_state(roof).thermal, Some(roof_thermal));
}

#[test]
fn cull_leaf_carries_incrementally_merged_state() {
    let mut s = Scene::new();
    s.set_state(s.root(), StateSet::new().with_metallic(1.0))
        .unwrap();
    let x = s.add_transform_at(s.root(), Vec3::X).unwrap();
    let g = s.add_geode(x).unwrap();
    s.set_state(g, StateSet::new().with_glass(true)).unwrap();
    s.add_drawable(g, sphere(1.0)).unwrap();

    let leaves = CullVisitor::new(Vec3::ZERO).cull(&s);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].state.metallic, Some(1.0));
    assert_eq!(leaves[0].state.glass, Some(true));
    assert_eq!(leaves[0].state, s.effective_state(g));
}

// ---------------------------------------------------------------------
// Switch and Lod
// ---------------------------------------------------------------------

#[test]
fn switch_masks_children_in_cull() {
    let mut s = Scene::new();
    let sw = s.add_switch(s.root()).unwrap();
    let g_on = s.add_geode(sw).unwrap();
    s.add_drawable(g_on, sphere(1.0)).unwrap();
    let g_off = s.add_geode(sw).unwrap();
    s.add_drawable(g_off, sphere(1.0)).unwrap();

    // Default: all on.
    assert_eq!(s.switch_mask(sw), Some(&[true, true][..]));
    let cv = CullVisitor::new(Vec3::ZERO);
    assert_eq!(cv.cull(&s).len(), 2);

    // Mask child 1 off.
    s.set_switch(sw, 1, false).unwrap();
    let leaves = cv.cull(&s);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].node, g_on);

    // All off.
    s.set_switch_all(sw, false).unwrap();
    assert!(cv.cull(&s).is_empty());

    // Structural traversal still sees masked children.
    let mut r = Recorder::default();
    visit_depth_first(&s, sw, &mut r);
    assert_eq!(r.events.iter().filter(|e| e.starts_with('+')).count(), 3);
}

#[test]
fn lod_selects_child_by_eye_distance() {
    let mut s = Scene::new();
    let lod = s.add_lod(s.root()).unwrap();
    let near = s.add_geode(lod).unwrap();
    s.add_drawable(near, sphere(1.0)).unwrap();
    let far = s.add_geode(lod).unwrap();
    s.add_drawable(far, sphere(1.0)).unwrap();
    s.set_lod_range(lod, 0, 0.0, 50.0).unwrap();
    s.set_lod_range(lod, 1, 50.0, f32::INFINITY).unwrap();

    // Geometry sits at the origin; eye at 10 m picks the near child.
    let leaves = CullVisitor::new(Vec3::new(10.0, 0.0, 0.0)).cull(&s);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].node, near);

    // Eye at 200 m picks the far child.
    let leaves = CullVisitor::new(Vec3::new(200.0, 0.0, 0.0)).cull(&s);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].node, far);

    // Boundary is [min, max): exactly 50 m selects the far child.
    let leaves = CullVisitor::new(Vec3::new(50.0, 0.0, 0.0)).cull(&s);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].node, far);
}

// ---------------------------------------------------------------------
// World matrices and dirty propagation
// ---------------------------------------------------------------------

#[test]
fn moving_a_parent_moves_child_world_position() {
    let mut s = Scene::new();
    let parent = s.add_transform_at(s.root(), Vec3::new(1.0, 0.0, 0.0)).unwrap();
    let child = s.add_transform_at(parent, Vec3::new(0.0, 2.0, 0.0)).unwrap();
    let g = s.add_geode(child).unwrap();
    s.add_drawable(g, sphere(0.5)).unwrap();

    UpdateVisitor::run(&s);
    let p0 = s.world_matrix(g).transform_point3(Vec3::ZERO);
    assert!((p0 - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-5);

    // Move the parent: the child's cached world matrix must be dirtied.
    s.set_translation(parent, Vec3::new(10.0, 0.0, 0.0)).unwrap();
    let p1 = s.world_matrix(g).transform_point3(Vec3::ZERO);
    assert!((p1 - Vec3::new(10.0, 2.0, 0.0)).length() < 1e-5);

    // Rotation composes too: 90 deg about Z at the parent sends the child's
    // local +Y offset to world -X.
    s.set_transform(
        parent,
        Vec3::ZERO,
        Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        Vec3::ONE,
    )
    .unwrap();
    let p2 = s.world_matrix(g).transform_point3(Vec3::ZERO);
    assert!((p2 - Vec3::new(-2.0, 0.0, 0.0)).length() < 1e-5);
}

#[test]
fn sibling_world_matrix_survives_unrelated_edit() {
    let mut s = Scene::new();
    let a = s.add_transform_at(s.root(), Vec3::X).unwrap();
    let b = s.add_transform_at(s.root(), Vec3::Y).unwrap();
    UpdateVisitor::run(&s);

    s.set_translation(a, Vec3::new(5.0, 0.0, 0.0)).unwrap();
    // Sibling untouched.
    assert_eq!(s.world_matrix(b).transform_point3(Vec3::ZERO), Vec3::Y);
    assert_eq!(
        s.world_matrix(a).transform_point3(Vec3::ZERO),
        Vec3::new(5.0, 0.0, 0.0)
    );
}

#[test]
fn update_visitor_refreshes_whole_scene() {
    let mut s = Scene::new();
    let x = s.add_transform_at(s.root(), Vec3::X).unwrap();
    let g = s.add_geode(x).unwrap();
    s.add_drawable(g, sphere(1.0)).unwrap();

    let v = UpdateVisitor::run(&s);
    assert_eq!(v.visited, 3); // root, x, g
    assert_eq!(s.world_matrix(g), Mat4::from_translation(Vec3::X));
}

// ---------------------------------------------------------------------
// Bounds
// ---------------------------------------------------------------------

#[test]
fn parent_bounds_enclose_children() {
    let mut s = Scene::new();
    let mut geodes = Vec::new();
    for t in [
        Vec3::new(10.0, 0.0, 0.0),
        Vec3::new(-5.0, 3.0, 2.0),
        Vec3::new(0.0, 0.0, -20.0),
    ] {
        let x = s.add_transform_at(s.root(), t).unwrap();
        let g = s.add_geode(x).unwrap();
        s.add_drawable(g, sphere(1.5)).unwrap();
        geodes.push(g);
    }

    let root_bound = s.world_bound(s.root());
    for &g in &geodes {
        let b = s.world_bound(g);
        assert!(!b.is_empty());
        assert!(
            root_bound.contains_sphere(&b),
            "root bound {root_bound:?} must contain {b:?}"
        );
    }
}

#[test]
fn bounds_follow_transform_edits() {
    let (mut s, x, g) = scene_with_sphere_at(Vec3::ZERO, 1.0);
    let b0 = s.world_bound(s.root());
    assert!((b0.center - Vec3::ZERO).length() < 1e-5);
    assert!((b0.radius - 1.0).abs() < 1e-5);

    s.set_translation(x, Vec3::new(100.0, 0.0, 0.0)).unwrap();
    let b1 = s.world_bound(s.root());
    assert!((b1.center - Vec3::new(100.0, 0.0, 0.0)).length() < 1e-4);
    assert!((b1.radius - 1.0).abs() < 1e-4);
    assert!(b1.contains_sphere(&s.world_bound(g)));
}

#[test]
fn bounds_scale_with_nonuniform_scale_conservatively() {
    let mut s = Scene::new();
    let x = s.add_transform(s.root()).unwrap();
    s.set_transform(x, Vec3::ZERO, Quat::IDENTITY, Vec3::new(3.0, 1.0, 1.0))
        .unwrap();
    let g = s.add_geode(x).unwrap();
    s.add_drawable(g, sphere(1.0)).unwrap();
    // Conservative: radius scaled by the max axis scale.
    assert!((s.world_bound(g).radius - 3.0).abs() < 1e-5);
}

#[test]
fn shape_local_bounds() {
    let b = Shape::Box {
        half_extents: Vec3::new(1.0, 2.0, 2.0),
    }
    .local_bound();
    assert!((b.radius - 3.0).abs() < 1e-5);

    let c = Shape::Cylinder {
        radius: 3.0,
        height: 8.0,
    }
    .local_bound();
    assert!((c.radius - 5.0).abs() < 1e-5);

    let cap = Shape::Capsule {
        radius: 1.0,
        height: 4.0,
    }
    .local_bound();
    assert!((cap.radius - 3.0).abs() < 1e-5);

    let m = Shape::Mesh {
        vertices: vec![Vec3::new(-1.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0)],
        indices: vec![],
    }
    .local_bound();
    assert!((m.center - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-5);
    assert!((m.radius - 2.0).abs() < 1e-5);

    assert!(Shape::Mesh {
        vertices: vec![],
        indices: vec![]
    }
    .local_bound()
    .is_empty());
}

// ---------------------------------------------------------------------
// Frustum culling
// ---------------------------------------------------------------------

#[test]
fn frustum_culls_subtrees_behind_a_plane() {
    let mut s = Scene::new();
    let front = s.add_transform_at(s.root(), Vec3::new(0.0, 0.0, -10.0)).unwrap();
    let fg = s.add_geode(front).unwrap();
    s.add_drawable(fg, sphere(1.0)).unwrap();
    let behind = s.add_transform_at(s.root(), Vec3::new(0.0, 0.0, 10.0)).unwrap();
    let bg = s.add_geode(behind).unwrap();
    s.add_drawable(bg, sphere(1.0)).unwrap();

    // Eye at origin looking down -Z; single near-ish plane keeping z < 0.
    let planes = vec![plane(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0))];
    let leaves = CullVisitor::new(Vec3::ZERO)
        .with_frustum(planes)
        .cull(&s);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves[0].node, fg);

    // A sphere straddling the plane is kept.
    let straddle = s.add_transform_at(s.root(), Vec3::new(0.0, 0.0, 0.5)).unwrap();
    let sg = s.add_geode(straddle).unwrap();
    s.add_drawable(sg, sphere(2.0)).unwrap();
    let planes = vec![plane(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0))];
    let leaves = CullVisitor::new(Vec3::ZERO)
        .with_frustum(planes)
        .cull(&s);
    assert_eq!(leaves.len(), 2);
}

#[test]
fn cull_emits_one_leaf_per_drawable_with_world_matrix() {
    let mut s = Scene::new();
    let x = s.add_transform_at(s.root(), Vec3::new(2.0, 0.0, 0.0)).unwrap();
    let g = s.add_geode(x).unwrap();
    s.add_drawable(g, Drawable::new(Shape::Sphere { radius: 1.0 })).unwrap();
    s.add_drawable(
        g,
        Drawable::new(Shape::Box {
            half_extents: Vec3::ONE,
        })
        .with_material(2),
    )
    .unwrap();

    let leaves = CullVisitor::new(Vec3::ZERO).cull(&s);
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].drawable, 0);
    assert_eq!(leaves[1].drawable, 1);
    for leaf in &leaves {
        assert_eq!(leaf.node, g);
        assert_eq!(leaf.world, Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0)));
    }
    // The material slot rides along on the drawable itself.
    match s.node(g).unwrap().kind() {
        NodeKind::Geode(geode) => assert_eq!(geode.drawables[1].material, 2),
        _ => panic!("expected geode"),
    }
}

// ---------------------------------------------------------------------
// Names and paths
// ---------------------------------------------------------------------

#[test]
fn find_by_name_and_path() {
    let mut s = Scene::new();
    let barn = s.add_group(s.root()).unwrap();
    s.set_name(barn, Some("barn".into())).unwrap();
    let roof = s.add_transform(barn).unwrap();
    s.set_name(roof, Some("roof".into())).unwrap();
    let panel = s.add_geode(roof).unwrap();
    s.set_name(panel, Some("panel".into())).unwrap();

    assert_eq!(s.find_by_name("root"), Some(s.root()));
    assert_eq!(s.find_by_name("roof"), Some(roof));
    assert_eq!(s.find_by_name("nope"), None);

    assert_eq!(s.path(panel), vec![s.root(), barn, roof, panel]);
    assert_eq!(s.path(s.root()), vec![s.root()]);
    // Unknown ids yield an empty path.
    assert!(s.path(NodeId(9999)).is_empty());

    // find_by_name returns the first match in depth-first preorder.
    let dup_late = s.add_group(barn).unwrap();
    s.set_name(dup_late, Some("roof".into())).unwrap();
    assert_eq!(s.find_by_name("roof"), Some(roof));
}

// ---------------------------------------------------------------------
// RON round trip
// ---------------------------------------------------------------------

fn build_kitchen_sink() -> Scene {
    let mut s = Scene::new();
    s.set_state(
        s.root(),
        StateSet::new().with_base_color(Vec4::new(0.5, 0.5, 0.5, 1.0)),
    )
    .unwrap();

    let barn = s.add_transform_at(s.root(), Vec3::new(4.0, 0.0, -12.0)).unwrap();
    s.set_name(barn, Some("barn".into())).unwrap();
    s.set_transform(
        barn,
        Vec3::new(4.0, 0.0, -12.0),
        Quat::from_rotation_y(0.3),
        Vec3::ONE,
    )
    .unwrap();

    let roof = s.add_geode(barn).unwrap();
    s.set_name(roof, Some("roof".into())).unwrap();
    s.set_state(
        roof,
        StateSet::new()
            .with_metallic(1.0)
            .with_roughness(0.35)
            .with_thermal(ThermalAttach {
                base_temp: TempF(41.0),
                thermal_mass: 4.0,
                sky_exposure: 0.95,
            }),
    )
    .unwrap();
    s.add_drawable(
        roof,
        Drawable::new(Shape::Box {
            half_extents: Vec3::new(6.0, 0.2, 9.0),
        }),
    )
    .unwrap();

    let silo = s.add_geode(barn).unwrap();
    s.add_drawable(
        silo,
        Drawable::new(Shape::Cylinder {
            radius: 2.0,
            height: 10.0,
        })
        .with_material(1),
    )
    .unwrap();
    s.add_drawable(silo, Drawable::new(Shape::Capsule { radius: 0.4, height: 1.2 }))
        .unwrap();

    let sw = s.add_switch(barn).unwrap();
    let door_open = s.add_geode(sw).unwrap();
    s.add_drawable(door_open, sphere(0.5)).unwrap();
    let door_closed = s.add_geode(sw).unwrap();
    s.add_drawable(door_closed, sphere(0.5)).unwrap();
    s.set_switch(sw, 0, false).unwrap();

    let lod = s.add_lod(s.root()).unwrap();
    let hi = s.add_geode(lod).unwrap();
    s.add_drawable(
        hi,
        Drawable::new(Shape::Mesh {
            vertices: vec![
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2],
        }),
    )
    .unwrap();
    let lo = s.add_geode(lod).unwrap();
    s.add_drawable(lo, sphere(0.8)).unwrap();
    s.set_lod_range(lod, 0, 0.0, 75.0).unwrap();
    s.set_lod_range(lod, 1, 75.0, f32::INFINITY).unwrap();

    s
}

#[test]
fn ron_round_trip_is_stable_and_equivalent() {
    let s = build_kitchen_sink();
    let text = s.to_ron().unwrap();
    let s2 = Scene::from_ron(&text).unwrap();
    let text2 = s2.to_ron().unwrap();

    // Byte-stable: serializing the reloaded scene reproduces the file.
    assert_eq!(text, text2);

    // Structural equivalence.
    assert_eq!(s.len(), s2.len());
    assert_eq!(s2.root(), s.root());
    let barn = s2.find_by_name("barn").expect("barn survives");
    assert_eq!(s2.find_by_name("barn"), s.find_by_name("barn"));
    assert_eq!(s2.path(barn), s.path(barn));

    // Caches rebuild after load: world matrices and bounds match.
    let roof = s2.find_by_name("roof").expect("roof survives");
    assert_eq!(s.world_matrix(roof), s2.world_matrix(roof));
    assert_eq!(s.world_bound(s.root()), s2.world_bound(s2.root()));

    // Inherited state survives.
    assert_eq!(s.effective_state(roof), s2.effective_state(roof));
    assert_eq!(s2.effective_state(roof).base_color, Some(Vec4::new(0.5, 0.5, 0.5, 1.0)));

    // Switch mask and LOD ranges survive.
    let mut masks = Vec::new();
    let mut ranges = Vec::new();
    for n in s2.nodes() {
        match n.kind() {
            NodeKind::Switch { mask } => masks.push(mask.clone()),
            NodeKind::Lod { ranges: r } => ranges.push(r.clone()),
            _ => {}
        }
    }
    assert_eq!(masks, vec![vec![false, true]]);
    assert_eq!(
        ranges,
        vec![vec![LodRange::new(0.0, 75.0), LodRange::new(75.0, f32::INFINITY)]]
    );

    // Cull output is identical (same draw list after reload).
    let cv = CullVisitor::new(Vec3::new(0.0, 1.7, 0.0));
    assert_eq!(cv.cull(&s), cv.cull(&s2));

    // Id generation continues without collisions after load.
    let mut s2 = s2;
    let fresh = s2.add_group(s2.root()).unwrap();
    assert!(s.nodes().all(|n| n.id() != fresh));
}

// ---------------------------------------------------------------------
// Errors and misc invariants
// ---------------------------------------------------------------------

#[test]
fn geodes_are_leaves_and_kind_mismatches_error() {
    let mut s = Scene::new();
    let g = s.add_geode(s.root()).unwrap();
    assert_eq!(s.add_group(g), Err(GraphError::GeodeIsLeaf(g)));

    assert_eq!(
        s.add_drawable(s.root(), sphere(1.0)),
        Err(GraphError::NotAGeode(s.root()))
    );
    assert_eq!(s.set_switch(g, 0, true), Err(GraphError::NotASwitch(g)));
    assert_eq!(
        s.set_lod_range(g, 0, 0.0, 1.0),
        Err(GraphError::NotALod(g))
    );
    assert_eq!(
        s.set_translation(g, Vec3::ONE),
        Err(GraphError::NotATransform(g))
    );

    let sw = s.add_switch(s.root()).unwrap();
    assert_eq!(
        s.set_switch(sw, 3, true),
        Err(GraphError::ChildIndexOutOfRange { node: sw, index: 3 })
    );

    let missing = NodeId(123_456);
    assert_eq!(s.add_group(missing), Err(GraphError::NoSuchNode(missing)));
    assert!(s.node(missing).is_none());
    assert_eq!(s.world_matrix(missing), Mat4::IDENTITY);
    assert!(s.world_bound(missing).is_empty());
    assert_eq!(s.effective_state(missing), StateSet::default());
}

#[test]
fn empty_scene_has_root_group_only() {
    let s = Scene::new();
    assert_eq!(s.len(), 1);
    assert!(!s.is_empty());
    let root = s.node(s.root()).unwrap();
    assert!(matches!(root.kind(), NodeKind::Group));
    assert_eq!(root.name(), Some("root"));
    assert_eq!(root.parent(), None);
    assert!(s.world_bound(s.root()).is_empty());
    assert!(CullVisitor::new(Vec3::ZERO).cull(&s).is_empty());
}
