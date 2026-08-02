//! Feature generators: each [`Feature`] variant expands into a named
//! subgraph of Box/Cylinder/Sphere primitives with materials and thermal
//! attaches applied. Silhouette-correct, not artistic — these exist so the
//! thermal/NV optics and the AI have something physically plausible to read.
//!
//! Determinism contract: for a fixed source, expansion is byte-identical;
//! the `Rng` is consumed only for *placement jitter*, never for structure,
//! so changing the seed moves things around without changing node counts
//! or names.

use da_core::Rng;
use da_graph::{Drawable, NodeId, Scene, Shape, StateSet};
use glam::{Quat, Vec3};

use crate::error::ParamError;
use crate::material as mat;
use crate::source::{Biome, Feature, PenSpec, RoofKind, TreeKind, P3};

/// Result of expanding one feature: the subgraph root plus the anchor
/// positions spawn tables resolve against (world space).
pub(crate) struct FeatureInstance {
    /// Subgraph root name — exactly the feature variant name.
    pub name: &'static str,
    /// Root node of the subgraph.
    #[allow(dead_code)]
    pub root: NodeId,
    /// Ground-level positions animals can spawn at, near this feature.
    pub ground: Vec<Vec3>,
    /// Canopy-height positions for elevated spawns (trees only).
    pub elevated: Vec<Vec3>,
    /// For path features (Creek): the world-space polyline and width, used
    /// to resolve `along:` hazards.
    pub path: Option<(Vec<Vec3>, f32)>,
}

/// Convert a source-model point tuple to a `Vec3`.
pub(crate) fn v3(p: P3) -> Vec3 {
    Vec3::new(p.0, p.1, p.2)
}

/// Rotation about +Y that carries local +X onto `dir` (projected to XZ).
fn yaw_to(dir: Vec3) -> Quat {
    Quat::from_rotation_y(-dir.z.atan2(dir.x))
}

/// Add a named `Transform` with a single-drawable `Geode` child carrying
/// `state`. Returns the geode id. This is the unit every generator is
/// assembled from; shapes are centered on the transform's origin.
fn part(
    scene: &mut Scene,
    parent: NodeId,
    name: &str,
    at: Vec3,
    rot: Quat,
    shape: Shape,
    state: StateSet,
) -> Result<NodeId, ParamError> {
    let t = scene.add_transform(parent)?;
    scene.set_transform(t, at, rot, Vec3::ONE)?;
    scene.set_name(t, Some(name.to_owned()))?;
    let g = scene.add_geode(t)?;
    scene.add_drawable(g, Drawable::new(shape))?;
    scene.set_state(g, Some(state))?;
    Ok(g)
}

fn boxed(hx: f32, hy: f32, hz: f32) -> Shape {
    Shape::Box {
        half_extents: Vec3::new(hx, hy, hz),
    }
}

/// XZ-plane jitter of at most `r` meters in each axis.
fn jitter(rng: &mut Rng, r: f32) -> Vec3 {
    Vec3::new(rng.range(-r, r), 0.0, rng.range(-r, r))
}

/// Add the zone's ground plane under `parent`.
pub(crate) fn add_ground(
    scene: &mut Scene,
    parent: NodeId,
    size_m: (f32, f32),
    biome: Biome,
) -> Result<(), ParamError> {
    part(
        scene,
        parent,
        "Ground",
        Vec3::new(size_m.0 * 0.5, -0.05, size_m.1 * 0.5),
        Quat::IDENTITY,
        boxed(size_m.0 * 0.5, 0.05, size_m.1 * 0.5),
        mat::ground(biome),
    )?;
    Ok(())
}

// ----------------------------------------------------------------------
// Trees (shared by TreeRow / TreeGrid / Park)
// ----------------------------------------------------------------------

/// (trunk height, trunk radius, canopy radius, canopy center height).
fn tree_dims(kind: TreeKind) -> (f32, f32, f32, f32) {
    match kind {
        TreeKind::Oak => (3.0, 0.35, 3.2, 4.8),
        TreeKind::Pine => (4.5, 0.28, 1.9, 6.2),
        TreeKind::Sycamore => (3.6, 0.42, 3.1, 5.4),
        TreeKind::Apple => (1.8, 0.24, 2.0, 3.0),
        TreeKind::Maple => (2.8, 0.3, 2.7, 4.4),
    }
}

/// Add one tree (trunk + canopy) at `local` under `parent`. Returns the
/// canopy center height above ground.
fn add_tree(
    scene: &mut Scene,
    parent: NodeId,
    kind: TreeKind,
    local: Vec3,
) -> Result<f32, ParamError> {
    let (th, tr, cr, cy) = tree_dims(kind);
    let root = scene.add_transform(parent)?;
    scene.set_transform(root, local, Quat::IDENTITY, Vec3::ONE)?;
    scene.set_name(root, Some("Tree".to_owned()))?;
    part(
        scene,
        root,
        "TreeTrunk",
        Vec3::new(0.0, th * 0.5, 0.0),
        Quat::IDENTITY,
        Shape::Cylinder {
            radius: tr,
            height: th,
        },
        mat::tree_trunk(),
    )?;
    part(
        scene,
        root,
        "TreeCanopy",
        Vec3::new(0.0, cy, 0.0),
        Quat::IDENTITY,
        Shape::Sphere { radius: cr },
        mat::tree_canopy(kind),
    )?;
    Ok(cy)
}

// ----------------------------------------------------------------------
// Feature dispatch
// ----------------------------------------------------------------------

/// Expand one feature into a named subgraph under `parent` and report its
/// spawn anchors. `rng` is this feature's private jitter stream.
pub(crate) fn expand_feature(
    scene: &mut Scene,
    parent: NodeId,
    feature: &Feature,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let name = feature.root_name();
    match feature {
        Feature::Barn {
            pos,
            width_m,
            bays,
            roof,
        } => barn(scene, parent, name, v3(*pos), *width_m, *bays, *roof),
        Feature::FeedShed { pos } => feed_shed(scene, parent, name, v3(*pos)),
        Feature::House { pos, floors } => house(scene, parent, name, v3(*pos), *floors),
        Feature::Shed { pos } => shed(scene, parent, name, v3(*pos)),
        Feature::Silo {
            pos,
            radius_m,
            height_m,
        } => silo(scene, parent, name, v3(*pos), *radius_m, *height_m),
        Feature::LoadingDock { pos, len_m } => loading_dock(scene, parent, name, v3(*pos), *len_m),
        Feature::FenceLine {
            from,
            to,
            post_gap_m,
        } => fence_line(scene, parent, name, v3(*from), v3(*to), *post_gap_m, rng),
        Feature::TreeRow {
            from,
            to,
            count,
            kind,
        } => tree_row(scene, parent, name, v3(*from), v3(*to), *count, *kind, rng),
        Feature::TreeGrid {
            pos,
            rows,
            cols,
            gap_m,
            kind,
        } => tree_grid(scene, parent, name, v3(*pos), *rows, *cols, *gap_m, *kind, rng),
        Feature::CropRows {
            pos,
            rows,
            len_m,
            gap_m,
        } => crop_rows(scene, parent, name, v3(*pos), *rows, *len_m, *gap_m),
        Feature::Creek { path, width_m } => creek(scene, parent, name, path, *width_m),
        Feature::BeaverDam { pos } => beaver_dam(scene, parent, name, v3(*pos), rng),
        Feature::Deadfall { pos, radius_m } => {
            deadfall(scene, parent, name, v3(*pos), *radius_m, rng)
        }
        Feature::BurrowField {
            pos,
            radius_m,
            count,
        } => burrow_field(scene, parent, name, v3(*pos), *radius_m, *count, rng),
        Feature::DumpsterRow { pos, count } => {
            dumpster_row(scene, parent, name, v3(*pos), *count, rng)
        }
        Feature::Storefront { pos, glass } => storefront(scene, parent, name, v3(*pos), *glass),
        Feature::TownHall { pos } => town_hall(scene, parent, name, v3(*pos)),
        Feature::Cemetery { pos, size } => cemetery(scene, parent, name, v3(*pos), *size, rng),
        Feature::AlleyRow { pos, len_m } => alley_row(scene, parent, name, v3(*pos), *len_m, rng),
        Feature::Park { pos, size } => park(scene, parent, name, v3(*pos), *size, rng),
        Feature::StreetlightRow { from, to, gap_m } => {
            streetlight_row(scene, parent, name, v3(*from), v3(*to), *gap_m, rng)
        }
        Feature::RadioMast { pos, height_m } => {
            radio_mast(scene, parent, name, v3(*pos), *height_m)
        }
    }
}

/// Create the named feature-root transform at `at`.
fn feature_root(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
) -> Result<NodeId, ParamError> {
    let root = scene.add_transform(parent)?;
    scene.set_transform(root, at, Quat::IDENTITY, Vec3::ONE)?;
    scene.set_name(root, Some(name.to_owned()))?;
    Ok(root)
}

fn instance(
    name: &'static str,
    root: NodeId,
    ground: Vec<Vec3>,
    elevated: Vec<Vec3>,
) -> FeatureInstance {
    FeatureInstance {
        name,
        root,
        ground,
        elevated,
        path: None,
    }
}

// ----------------------------------------------------------------------
// Buildings
// ----------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn barn(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    width_m: f32,
    bays: u32,
    roof: RoofKind,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let len = bays as f32 * 4.0; // 4 m per bay, ridge along local x
    let hw = width_m * 0.5;
    let wall_h = 4.0;
    // Two long side walls (along x, at ±z).
    for side in [-1.0f32, 1.0] {
        part(
            scene,
            root,
            "BarnWall",
            Vec3::new(0.0, wall_h * 0.5, side * hw),
            Quat::IDENTITY,
            boxed(len * 0.5, wall_h * 0.5, 0.15),
            mat::building_wall(0.5, 0.12, 0.1),
        )?;
    }
    // Two gable end walls (across z, at ±x).
    for side in [-1.0f32, 1.0] {
        part(
            scene,
            root,
            "BarnWall",
            Vec3::new(side * len * 0.5, wall_h * 0.5, 0.0),
            Quat::IDENTITY,
            boxed(0.15, wall_h * 0.5, hw),
            mat::building_wall(0.5, 0.12, 0.1),
        )?;
    }
    // Gabled roof: two panels pitched about the x axis (ridge along x).
    let pitch = 25.0f32.to_radians();
    let panel_half_w = hw / pitch.cos();
    let ridge_rise = hw * pitch.tan();
    let roof_state = match roof {
        RoofKind::Metal => mat::metal_roof(),
        RoofKind::Shingle => mat::shingle_roof(),
    };
    for side in [-1.0f32, 1.0] {
        part(
            scene,
            root,
            "BarnRoofPanel",
            Vec3::new(0.0, wall_h + ridge_rise * 0.5, side * hw * 0.5),
            Quat::from_rotation_x(side * pitch),
            boxed(len * 0.5 + 0.3, 0.08, panel_half_w * 0.5 + 0.2),
            roof_state.clone(),
        )?;
    }
    // One structural post per bay along the centerline; their positions
    // double as rat spawn anchors (feed spills at the posts).
    let mut ground = Vec::new();
    for b in 0..bays {
        let x = -len * 0.5 + (b as f32 + 0.5) * 4.0;
        part(
            scene,
            root,
            "BarnPost",
            Vec3::new(x, wall_h * 0.5, 0.0),
            Quat::IDENTITY,
            Shape::Cylinder {
                radius: 0.12,
                height: wall_h,
            },
            mat::wood(),
        )?;
        ground.push(at + Vec3::new(x, 0.0, 0.0));
    }
    Ok(instance(name, root, ground, Vec::new()))
}

fn feed_shed(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "FeedShedWalls",
        Vec3::new(0.0, 1.2, 0.0),
        Quat::IDENTITY,
        boxed(2.0, 1.2, 1.5),
        mat::building_wall(0.55, 0.45, 0.35),
    )?;
    part(
        scene,
        root,
        "FeedShedRoof",
        Vec3::new(0.0, 2.55, 0.0),
        Quat::from_rotation_z(6.0f32.to_radians()),
        boxed(2.3, 0.06, 1.8),
        mat::metal_roof(),
    )?;
    part(
        scene,
        root,
        "FeedTrough",
        Vec3::new(0.0, 0.3, 2.0),
        Quat::IDENTITY,
        boxed(1.6, 0.3, 0.35),
        mat::wood(),
    )?;
    let ground = vec![
        at + Vec3::new(0.0, 0.0, 2.0),
        at + Vec3::new(1.8, 0.0, -1.0),
        at + Vec3::new(-1.8, 0.0, 1.0),
    ];
    Ok(instance(name, root, ground, Vec::new()))
}

fn house(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    floors: u32,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let h = floors.max(1) as f32 * 2.8;
    let (hx, hz) = (4.5, 3.5);
    part(
        scene,
        root,
        "HouseWalls",
        Vec3::new(0.0, h * 0.5, 0.0),
        Quat::IDENTITY,
        boxed(hx, h * 0.5, hz),
        mat::building_wall(0.75, 0.72, 0.65),
    )?;
    // Gabled shingle roof, ridge along x.
    let pitch = 30.0f32.to_radians();
    let rise = hz * pitch.tan();
    for side in [-1.0f32, 1.0] {
        part(
            scene,
            root,
            "HouseRoofPanel",
            Vec3::new(0.0, h + rise * 0.5, side * hz * 0.5),
            Quat::from_rotation_x(side * pitch),
            boxed(hx + 0.3, 0.07, hz / pitch.cos() * 0.5 + 0.2),
            mat::shingle_roof(),
        )?;
    }
    // Front door plus one glass window pair per floor on the front face.
    part(
        scene,
        root,
        "HouseDoor",
        Vec3::new(0.0, 1.05, hz + 0.02),
        Quat::IDENTITY,
        boxed(0.5, 1.05, 0.04),
        mat::wood(),
    )?;
    for fl in 0..floors.max(1) {
        let wy = fl as f32 * 2.8 + 1.6;
        for side in [-1.0f32, 1.0] {
            part(
                scene,
                root,
                "HouseWindow",
                Vec3::new(side * 2.4, wy, hz + 0.02),
                Quat::IDENTITY,
                boxed(0.55, 0.65, 0.03),
                mat::glass_pane(),
            )?;
        }
    }
    part(
        scene,
        root,
        "HouseChimney",
        Vec3::new(hx - 0.8, h + rise + 0.6, 0.0),
        Quat::IDENTITY,
        boxed(0.35, 1.0, 0.35),
        mat::concrete(),
    )?;
    let ground = vec![at + Vec3::new(0.0, 0.0, hz + 1.5)];
    Ok(instance(name, root, ground, Vec::new()))
}

fn shed(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "ShedWalls",
        Vec3::new(0.0, 1.1, 0.0),
        Quat::IDENTITY,
        boxed(1.5, 1.1, 1.25),
        mat::building_wall(0.42, 0.4, 0.36),
    )?;
    part(
        scene,
        root,
        "ShedRoof",
        Vec3::new(0.0, 2.35, 0.0),
        Quat::from_rotation_z(8.0f32.to_radians()),
        boxed(1.8, 0.05, 1.55),
        mat::metal_roof(),
    )?;
    let ground = vec![at + Vec3::new(0.0, 0.0, 1.9), at + Vec3::new(-1.9, 0.0, 0.0)];
    Ok(instance(name, root, ground, Vec::new()))
}

fn silo(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    radius_m: f32,
    height_m: f32,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "SiloBarrel",
        Vec3::new(0.0, height_m * 0.5, 0.0),
        Quat::IDENTITY,
        Shape::Cylinder {
            radius: radius_m,
            height: height_m,
        },
        mat::metal_surface(),
    )?;
    part(
        scene,
        root,
        "SiloDome",
        Vec3::new(0.0, height_m, 0.0),
        Quat::IDENTITY,
        Shape::Sphere { radius: radius_m },
        mat::metal_roof(),
    )?;
    // Unloading chute at the base — the feed spill line rats work.
    part(
        scene,
        root,
        "SiloChute",
        Vec3::new(radius_m + 0.6, 0.5, 0.0),
        Quat::IDENTITY,
        boxed(0.7, 0.5, 0.4),
        mat::metal_surface(),
    )?;
    let ground = vec![
        at + Vec3::new(radius_m + 1.2, 0.0, 0.0),
        at + Vec3::new(-radius_m - 0.8, 0.0, 0.6),
        at + Vec3::new(0.0, 0.0, radius_m + 1.0),
    ];
    Ok(instance(name, root, ground, Vec::new()))
}

fn loading_dock(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    len_m: f32,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "DockPlatform",
        Vec3::new(0.0, 1.1, 0.0),
        Quat::IDENTITY,
        boxed(len_m * 0.5, 0.25, 2.0),
        mat::concrete(),
    )?;
    let n_posts = (len_m / 5.0).floor() as u32 + 1;
    let mut ground = Vec::new();
    for i in 0..n_posts {
        let x = -len_m * 0.5 + i as f32 * 5.0;
        part(
            scene,
            root,
            "DockPost",
            Vec3::new(x, 0.45, 0.0),
            Quat::IDENTITY,
            Shape::Cylinder {
                radius: 0.15,
                height: 0.9,
            },
            mat::wood(),
        )?;
        // Spill line: rats work the dark space under the dock lip.
        ground.push(at + Vec3::new(x, 0.0, 2.4));
    }
    part(
        scene,
        root,
        "DockRamp",
        Vec3::new(len_m * 0.5 + 1.4, 0.55, 0.0),
        Quat::from_rotation_z(-20.0f32.to_radians()),
        boxed(1.6, 0.1, 1.5),
        mat::concrete(),
    )?;
    Ok(instance(name, root, ground, Vec::new()))
}

fn storefront(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    glass: bool,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let (hx, h, hz) = (4.0, 4.0, 3.0);
    part(
        scene,
        root,
        "StorefrontWalls",
        Vec3::new(0.0, h * 0.5, 0.0),
        Quat::IDENTITY,
        boxed(hx, h * 0.5, hz),
        mat::building_wall(0.6, 0.5, 0.42),
    )?;
    // Flat tar roof with a parapet lip.
    part(
        scene,
        root,
        "StorefrontRoof",
        Vec3::new(0.0, h + 0.05, 0.0),
        Quat::IDENTITY,
        boxed(hx, 0.05, hz),
        mat::shingle_roof(),
    )?;
    part(
        scene,
        root,
        "StorefrontParapet",
        Vec3::new(0.0, h + 0.35, hz - 0.1),
        Quat::IDENTITY,
        boxed(hx, 0.25, 0.1),
        mat::building_wall(0.6, 0.5, 0.42),
    )?;
    if glass {
        // Full display pane on the street face — LWIR-opaque (SDD §7).
        part(
            scene,
            root,
            "StorefrontGlass",
            Vec3::new(0.0, 1.6, hz + 0.03),
            Quat::IDENTITY,
            boxed(hx - 0.5, 1.4, 0.03),
            mat::glass_pane(),
        )?;
    } else {
        part(
            scene,
            root,
            "StorefrontBoard",
            Vec3::new(0.0, 1.6, hz + 0.03),
            Quat::IDENTITY,
            boxed(hx - 0.5, 1.4, 0.04),
            mat::wood(),
        )?;
    }
    let ground = vec![
        at + Vec3::new(0.0, 0.0, hz + 1.2),
        at + Vec3::new(-hx - 0.8, 0.0, -hz),
    ];
    Ok(instance(name, root, ground, Vec::new()))
}

fn town_hall(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let (hx, h, hz) = (7.0, 6.0, 5.0);
    part(
        scene,
        root,
        "TownHallWalls",
        Vec3::new(0.0, h * 0.5, 0.0),
        Quat::IDENTITY,
        boxed(hx, h * 0.5, hz),
        mat::building_wall(0.72, 0.68, 0.6),
    )?;
    part(
        scene,
        root,
        "TownHallRoof",
        Vec3::new(0.0, h + 0.1, 0.0),
        Quat::IDENTITY,
        boxed(hx + 0.4, 0.1, hz + 0.4),
        mat::shingle_roof(),
    )?;
    part(
        scene,
        root,
        "TownHallSteps",
        Vec3::new(0.0, 0.3, hz + 1.0),
        Quat::IDENTITY,
        boxed(3.0, 0.3, 1.0),
        mat::concrete(),
    )?;
    for i in 0..4u32 {
        let x = -3.0 + i as f32 * 2.0;
        part(
            scene,
            root,
            "TownHallColumn",
            Vec3::new(x, 2.5, hz + 0.4),
            Quat::IDENTITY,
            Shape::Cylinder {
                radius: 0.25,
                height: 5.0,
            },
            mat::concrete(),
        )?;
    }
    part(
        scene,
        root,
        "TownHallCupola",
        Vec3::new(0.0, h + 1.4, 0.0),
        Quat::IDENTITY,
        Shape::Sphere { radius: 1.1 },
        mat::metal_roof(),
    )?;
    let ground = vec![at + Vec3::new(0.0, 0.0, hz + 2.5)];
    Ok(instance(name, root, ground, Vec::new()))
}

// ----------------------------------------------------------------------
// Linear features
// ----------------------------------------------------------------------

fn fence_line(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    from: Vec3,
    to: Vec3,
    post_gap_m: f32,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, from)?;
    let delta = to - from;
    let len = delta.length();
    let dir = if len > 0.0 { delta / len } else { Vec3::X };
    let rot = yaw_to(dir);
    // Posts every `post_gap_m`, including the origin post.
    let n_posts = (len / post_gap_m).floor() as u32 + 1;
    for i in 0..n_posts {
        let along = dir * (i as f32 * post_gap_m);
        let j = jitter(rng, 0.12);
        part(
            scene,
            root,
            "FencePost",
            along + j + Vec3::new(0.0, 0.7, 0.0),
            Quat::IDENTITY,
            Shape::Cylinder {
                radius: 0.07,
                height: 1.4,
            },
            mat::wood(),
        )?;
    }
    // Two rails spanning the whole run.
    for h in [0.55f32, 1.1] {
        part(
            scene,
            root,
            "FenceRail",
            dir * (len * 0.5) + Vec3::new(0.0, h, 0.0),
            rot,
            boxed(len * 0.5, 0.04, 0.03),
            mat::wood(),
        )?;
    }
    Ok(instance(name, root, vec![from, to], Vec::new()))
}

#[allow(clippy::too_many_arguments)]
fn tree_row(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    from: Vec3,
    to: Vec3,
    count: u32,
    kind: TreeKind,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, from)?;
    let delta = to - from;
    let n = count.max(1);
    let mut ground = Vec::new();
    let mut elevated = Vec::new();
    for i in 0..n {
        let t = (i as f32 + 0.5) / n as f32;
        let local = delta * t + jitter(rng, 1.2);
        let canopy_y = add_tree(scene, root, kind, local)?;
        ground.push(from + local);
        elevated.push(from + local + Vec3::new(0.0, canopy_y, 0.0));
    }
    Ok(instance(name, root, ground, elevated))
}

#[allow(clippy::too_many_arguments)]
fn tree_grid(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    rows: u32,
    cols: u32,
    gap_m: f32,
    kind: TreeKind,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let mut ground = Vec::new();
    let mut elevated = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            let local = Vec3::new(c as f32 * gap_m, 0.0, r as f32 * gap_m) + jitter(rng, 0.9);
            let canopy_y = add_tree(scene, root, kind, local)?;
            ground.push(at + local);
            elevated.push(at + local + Vec3::new(0.0, canopy_y, 0.0));
        }
    }
    Ok(instance(name, root, ground, elevated))
}

fn crop_rows(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    rows: u32,
    len_m: f32,
    gap_m: f32,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    for r in 0..rows {
        part(
            scene,
            root,
            "CropRow",
            Vec3::new(len_m * 0.5, 0.35, r as f32 * gap_m),
            Quat::IDENTITY,
            boxed(len_m * 0.5, 0.35, 0.3),
            mat::vegetation(),
        )?;
    }
    let ground = vec![
        at + Vec3::new(len_m * 0.5, 0.0, rows.saturating_sub(1) as f32 * gap_m * 0.5),
    ];
    Ok(instance(name, root, ground, Vec::new()))
}

fn creek(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    path: &[P3],
    width_m: f32,
) -> Result<FeatureInstance, ParamError> {
    let verts: Vec<Vec3> = path.iter().map(|p| v3(*p)).collect();
    let origin = verts.first().copied().unwrap_or(Vec3::ZERO);
    let root = feature_root(scene, parent, name, origin)?;
    let hw = width_m * 0.5;
    for seg in verts.windows(2) {
        let (a, b) = (seg[0] - origin, seg[1] - origin);
        let delta = b - a;
        let len = delta.length();
        if len <= f32::EPSILON {
            continue;
        }
        let dir = delta / len;
        let rot = yaw_to(dir);
        let mid = a + delta * 0.5;
        // Water surface, slightly below grade.
        part(
            scene,
            root,
            "CreekWater",
            mid + Vec3::new(0.0, -0.25, 0.0),
            rot,
            boxed(len * 0.5 + hw * 0.4, 0.05, hw),
            mat::water(),
        )?;
        // A bank berm on each side.
        let perp = rot * Vec3::Z;
        for side in [-1.0f32, 1.0] {
            part(
                scene,
                root,
                "CreekBank",
                mid + perp * (side * (hw + 0.5)),
                rot,
                boxed(len * 0.5 + hw * 0.4, 0.3, 0.5),
                mat::bank_earth(),
            )?;
        }
    }
    let ground = verts.clone();
    Ok(FeatureInstance {
        name,
        root,
        ground,
        elevated: Vec::new(),
        path: Some((verts, width_m)),
    })
}

fn streetlight_row(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    from: Vec3,
    to: Vec3,
    gap_m: f32,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, from)?;
    let delta = to - from;
    let len = delta.length();
    let dir = if len > 0.0 { delta / len } else { Vec3::X };
    let n = (len / gap_m).floor() as u32 + 1;
    let mut ground = Vec::new();
    for i in 0..n {
        let along = dir * (i as f32 * gap_m) + jitter(rng, 0.05);
        part(
            scene,
            root,
            "StreetlightPole",
            along + Vec3::new(0.0, 2.25, 0.0),
            Quat::IDENTITY,
            Shape::Cylinder {
                radius: 0.1,
                height: 4.5,
            },
            mat::metal_surface(),
        )?;
        part(
            scene,
            root,
            "StreetlightHead",
            along + Vec3::new(0.0, 4.6, 0.0),
            Quat::IDENTITY,
            Shape::Sphere { radius: 0.3 },
            mat::lamp_head(),
        )?;
        ground.push(from + along);
    }
    Ok(instance(name, root, ground, Vec::new()))
}

fn alley_row(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    len_m: f32,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "AlleyPavement",
        Vec3::new(len_m * 0.5, 0.02, 0.0),
        Quat::IDENTITY,
        boxed(len_m * 0.5, 0.02, 2.0),
        mat::ground(Biome::Asphalt),
    )?;
    // Rear wall segments with rat-run gaps between them.
    let n_walls = (len_m / 30.0).floor() as u32 + 1;
    let mut ground = Vec::new();
    for i in 0..n_walls {
        let x = (i as f32 + 0.5) * (len_m / n_walls as f32);
        part(
            scene,
            root,
            "AlleyWall",
            Vec3::new(x, 1.5, -2.2),
            Quat::IDENTITY,
            boxed(len_m / n_walls as f32 * 0.4, 1.5, 0.15),
            mat::building_wall(0.5, 0.45, 0.4),
        )?;
        // Clutter pile near each wall segment.
        let j = jitter(rng, 0.8);
        part(
            scene,
            root,
            "AlleyClutter",
            Vec3::new(x, 0.35, -1.4) + j,
            Quat::from_rotation_y(rng.range(0.0, std::f32::consts::TAU)),
            boxed(0.5, 0.35, 0.4),
            mat::wood(),
        )?;
        ground.push(at + Vec3::new(x, 0.0, -1.0));
    }
    Ok(instance(name, root, ground, Vec::new()))
}

// ----------------------------------------------------------------------
// Habitat clutter
// ----------------------------------------------------------------------

fn beaver_dam(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "DamMound",
        Vec3::new(0.0, 0.4, 0.0),
        Quat::IDENTITY,
        Shape::Sphere { radius: 1.6 },
        mat::bank_earth(),
    )?;
    for i in 0..5u32 {
        let yaw = i as f32 * 0.7 + rng.range(-0.25, 0.25);
        let off = jitter(rng, 1.0);
        part(
            scene,
            root,
            "DamLog",
            off + Vec3::new(0.0, 0.35 + i as f32 * 0.12, 0.0),
            Quat::from_rotation_y(yaw) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Shape::Cylinder {
                radius: 0.14,
                height: 3.2,
            },
            mat::wood(),
        )?;
    }
    let ground = vec![at + Vec3::new(2.2, 0.0, 0.0), at + Vec3::new(-2.2, 0.0, 0.8)];
    Ok(instance(name, root, ground, Vec::new()))
}

fn deadfall(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    radius_m: f32,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let mut ground = Vec::new();
    for _ in 0..6u32 {
        let off = jitter(rng, radius_m * 0.7);
        let yaw = rng.range(0.0, std::f32::consts::TAU);
        part(
            scene,
            root,
            "DeadfallLog",
            off + Vec3::new(0.0, 0.3, 0.0),
            Quat::from_rotation_y(yaw) * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Shape::Cylinder {
                radius: 0.25,
                height: 4.0,
            },
            mat::wood(),
        )?;
        ground.push(at + off);
    }
    Ok(instance(name, root, ground, Vec::new()))
}

fn burrow_field(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    radius_m: f32,
    count: u32,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let mut ground = Vec::new();
    for i in 0..count {
        // Deterministic ring placement + jitter: structure never depends
        // on the seed, only the exact mound positions do.
        let theta = i as f32 / count.max(1) as f32 * std::f32::consts::TAU;
        let r = radius_m * 0.6;
        let off = Vec3::new(theta.cos() * r, 0.0, theta.sin() * r) + jitter(rng, radius_m * 0.25);
        part(
            scene,
            root,
            "BurrowMound",
            off + Vec3::new(0.0, 0.1, 0.0),
            Quat::IDENTITY,
            Shape::Sphere { radius: 0.55 },
            mat::dirt(),
        )?;
        ground.push(at + off);
    }
    Ok(instance(name, root, ground, Vec::new()))
}

fn dumpster_row(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    count: u32,
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    let mut ground = Vec::new();
    for i in 0..count {
        let off = Vec3::new(i as f32 * 2.6, 0.0, 0.0) + jitter(rng, 0.15);
        part(
            scene,
            root,
            "Dumpster",
            off + Vec3::new(0.0, 0.7, 0.0),
            Quat::from_rotation_y(rng.range(-0.08, 0.08)),
            boxed(1.0, 0.7, 0.65),
            mat::metal_surface(),
        )?;
        // Lid propped open.
        part(
            scene,
            root,
            "DumpsterLid",
            off + Vec3::new(0.0, 1.5, -0.5),
            Quat::from_rotation_x(-0.6),
            boxed(1.0, 0.03, 0.65),
            mat::metal_surface(),
        )?;
        ground.push(at + off + Vec3::new(0.0, 0.0, 1.1));
    }
    Ok(instance(name, root, ground, Vec::new()))
}

fn cemetery(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    size: (f32, f32),
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "CemeteryLawn",
        Vec3::new(size.0 * 0.5, 0.03, size.1 * 0.5),
        Quat::IDENTITY,
        boxed(size.0 * 0.5, 0.03, size.1 * 0.5),
        mat::vegetation(),
    )?;
    let nx = (size.0 / 4.0).floor().max(1.0) as u32;
    let nz = (size.1 / 4.0).floor().max(1.0) as u32;
    for r in 0..nz {
        for c in 0..nx {
            let off = Vec3::new(2.0 + c as f32 * 4.0, 0.0, 2.0 + r as f32 * 4.0)
                + jitter(rng, 0.35);
            part(
                scene,
                root,
                "Headstone",
                off + Vec3::new(0.0, 0.45, 0.0),
                Quat::from_rotation_y(rng.range(-0.1, 0.1)),
                boxed(0.35, 0.45, 0.08),
                mat::concrete(),
            )?;
        }
    }
    let ground = vec![at + Vec3::new(size.0 * 0.5, 0.0, size.1 * 0.5)];
    Ok(instance(name, root, ground, Vec::new()))
}

fn park(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    size: (f32, f32),
    rng: &mut Rng,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "ParkLawn",
        Vec3::new(size.0 * 0.5, 0.03, size.1 * 0.5),
        Quat::IDENTITY,
        boxed(size.0 * 0.5, 0.03, size.1 * 0.5),
        mat::vegetation(),
    )?;
    // Tree count is a pure function of area — seed changes never change it.
    let n = ((size.0 * size.1 / 300.0) as u32).max(3);
    let mut ground = Vec::new();
    let mut elevated = Vec::new();
    for _ in 0..n {
        let local = Vec3::new(
            rng.range(3.0, (size.0 - 3.0).max(3.1)),
            0.0,
            rng.range(3.0, (size.1 - 3.0).max(3.1)),
        );
        let canopy_y = add_tree(scene, root, TreeKind::Maple, local)?;
        ground.push(at + local);
        elevated.push(at + local + Vec3::new(0.0, canopy_y, 0.0));
    }
    // Rooting spots at the lawn center for the hogs.
    ground.push(at + Vec3::new(size.0 * 0.5, 0.0, size.1 * 0.5));
    Ok(instance(name, root, ground, elevated))
}

fn radio_mast(
    scene: &mut Scene,
    parent: NodeId,
    name: &'static str,
    at: Vec3,
    height_m: f32,
) -> Result<FeatureInstance, ParamError> {
    let root = feature_root(scene, parent, name, at)?;
    part(
        scene,
        root,
        "MastPole",
        Vec3::new(0.0, height_m * 0.5, 0.0),
        Quat::IDENTITY,
        Shape::Cylinder {
            radius: 0.3,
            height: height_m,
        },
        mat::metal_surface(),
    )?;
    for (i, frac) in [0.55f32, 0.8].iter().enumerate() {
        let rot = if i == 0 {
            Quat::IDENTITY
        } else {
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
        };
        part(
            scene,
            root,
            "MastCrossarm",
            Vec3::new(0.0, height_m * frac, 0.0),
            rot,
            boxed(1.6, 0.08, 0.08),
            mat::metal_surface(),
        )?;
    }
    part(
        scene,
        root,
        "MastBeacon",
        Vec3::new(0.0, height_m + 0.4, 0.0),
        Quat::IDENTITY,
        Shape::Sphere { radius: 0.25 },
        mat::beacon(),
    )?;
    let ground = vec![at + Vec3::new(1.5, 0.0, 0.0)];
    Ok(instance(name, root, ground, Vec::new()))
}

// ----------------------------------------------------------------------
// Pens (from friendly records)
// ----------------------------------------------------------------------

/// Expand a livestock pen: a fenced rectangle named `"Pen"` in the scene,
/// plus `count` static animal positions inside it (world space).
pub(crate) fn build_pen(
    scene: &mut Scene,
    parent: NodeId,
    pen: &PenSpec,
    count: u32,
    rng: &mut Rng,
) -> Result<Vec<Vec3>, ParamError> {
    let at = v3(pen.pos);
    let (w, d) = pen.size;
    let root = feature_root(scene, parent, "Pen", at)?;
    // Corner-to-corner sides.
    let corners = [
        Vec3::ZERO,
        Vec3::new(w, 0.0, 0.0),
        Vec3::new(w, 0.0, d),
        Vec3::new(0.0, 0.0, d),
    ];
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        let delta = b - a;
        let len = delta.length();
        let dir = delta / len;
        let rot = yaw_to(dir);
        let n_posts = (len / 2.5).floor() as u32 + 1;
        for p in 0..n_posts {
            part(
                scene,
                root,
                "PenPost",
                a + dir * (p as f32 * 2.5) + Vec3::new(0.0, 0.6, 0.0),
                Quat::IDENTITY,
                Shape::Cylinder {
                    radius: 0.06,
                    height: 1.2,
                },
                mat::wood(),
            )?;
        }
        part(
            scene,
            root,
            "PenRail",
            a + dir * (len * 0.5) + Vec3::new(0.0, 0.9, 0.0),
            rot,
            boxed(len * 0.5, 0.04, 0.03),
            mat::wood(),
        )?;
    }
    // Static livestock positions, jittered inside the fenced rectangle.
    let mut positions = Vec::new();
    for i in 0..count {
        let gx = (i as f32 + 0.5) / count.max(1) as f32;
        let base = Vec3::new(w * gx, 0.0, d * 0.5);
        let p = base + jitter(rng, (d * 0.3).min(w * 0.2));
        positions.push(at + Vec3::new(p.x.clamp(1.0, w - 1.0), 0.0, p.z.clamp(1.0, d - 1.0)));
    }
    Ok(positions)
}
