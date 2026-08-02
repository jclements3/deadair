//! Zone expansion: compile a declarative [`ZoneSource`] into a scene graph
//! plus gameplay data (spawn points, patrol routes, friendly setups, hazard
//! volumes).
//!
//! Determinism (SDD §10): expansion is a pure function of the source. The
//! same source expands to a byte-identical `Scene::to_ron` every time, and
//! changing only the `seed` moves placement jitter without changing node
//! counts or names.

use da_core::Rng;
use da_graph::Scene;
use glam::Vec3;

use crate::error::ParamError;
use crate::generate::{add_ground, build_pen, expand_feature, v3, FeatureInstance};
use crate::source::{
    Biome, Connection, HazardKind, HazardRecord, SpawnRef, ZoneSource,
};

/// One resolved animal spawn position.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnPoint {
    /// Species name (e.g. `"Rat"`).
    pub species: String,
    /// World-space position; y is canopy height for elevated spawns.
    pub pos: Vec3,
    /// True if the spawn sits at canopy height (orchard possums).
    pub elevated: bool,
}

/// A resolved friendly-animal setup.
#[derive(Debug, Clone, PartialEq)]
pub struct FriendlySetup {
    /// Species name (e.g. `"Dog"`).
    pub species: String,
    /// Number of individuals.
    pub count: u32,
    /// How the animal occupies the zone.
    pub behavior: FriendlyBehavior,
}

/// How a friendly animal occupies the zone.
#[derive(Debug, Clone, PartialEq)]
pub enum FriendlyBehavior {
    /// Walks a fixed polyline (dogs).
    Patrol(Vec<Vec3>),
    /// Held inside a fenced rectangle expanded into the scene.
    Penned {
        /// Pen corner (matches the source record's `pos`).
        corner: Vec3,
        /// Pen extent in meters, `(x, z)`.
        size: (f32, f32),
        /// Static per-animal positions inside the pen.
        positions: Vec<Vec3>,
    },
    /// Loiters near resolved feature anchor positions (cats at feed sheds).
    WanderNear(Vec<Vec3>),
}

/// A resolved hazard volume.
#[derive(Debug, Clone, PartialEq)]
pub struct HazardVolume {
    /// What kind of hazard this is.
    pub kind: HazardKind,
    /// Where it applies.
    pub volume: Volume,
}

/// Geometric extent of a hazard.
#[derive(Debug, Clone, PartialEq)]
pub enum Volume {
    /// A capsule-like segment (fence wire).
    Segment {
        /// Segment start.
        from: Vec3,
        /// Segment end.
        to: Vec3,
        /// Effect radius around the segment, meters.
        radius: f32,
    },
    /// A sphere (holes, limb piles).
    Sphere {
        /// Sphere center.
        center: Vec3,
        /// Sphere radius, meters.
        radius: f32,
    },
    /// A widened polyline (creek water, creek banks).
    Polyline {
        /// Centerline vertices.
        points: Vec<Vec3>,
        /// Total strip width, meters.
        width: f32,
    },
}

/// Everything [`expand_zone`] produces from one [`ZoneSource`].
#[derive(Debug)]
pub struct ZoneExpansion {
    /// The compiled scene graph (a build artifact — never hand-edited).
    pub scene: Scene,
    /// Resolved pest spawn positions.
    pub spawn_points: Vec<SpawnPoint>,
    /// Patrol routes from spawn tables: `(species, polyline)`.
    pub patrol_routes: Vec<(String, Vec<Vec3>)>,
    /// Resolved friendly-animal setups.
    pub friendly_setups: Vec<FriendlySetup>,
    /// Resolved hazard volumes.
    pub hazard_volumes: Vec<HazardVolume>,
    /// Zombie spawn weighting, `0..=1`, copied from the source.
    pub zombie_weight: f32,
    /// Zone connections, copied from the source.
    pub connections: Vec<Connection>,
    /// Ground biome, copied from the source.
    pub ground_biome: Biome,
}

/// Compile a zone source into a scene graph and gameplay data.
///
/// Deterministic: calling this twice on the same source yields a scene
/// whose [`Scene::to_ron`] output is byte-identical, and identical spawn
/// data. All jitter derives from `source.seed` via forked [`Rng`] streams.
///
/// Errors if a `Feature("...")` spawn reference or an `along: "..."`
/// hazard reference does not resolve, or a hazard defines no volume.
pub fn expand_zone(source: &ZoneSource) -> Result<ZoneExpansion, ParamError> {
    let mut scene = Scene::new();
    let root = scene.root();

    // Independent jitter streams per subsystem, all derived from the seed.
    let mut master = Rng::new(source.seed);
    let mut feature_stream = master.fork(0xFEA7);
    let mut spawn_stream = master.fork(0x5AA5);
    let mut friendly_stream = master.fork(0xF00D);

    // Ground plane, then features in listed order.
    add_ground(&mut scene, root, source.size_m, source.ambient_biome)?;
    let mut instances: Vec<FeatureInstance> = Vec::with_capacity(source.features.len());
    for (i, feature) in source.features.iter().enumerate() {
        let mut rng = feature_stream.fork(i as u64);
        instances.push(expand_feature(&mut scene, root, feature, &mut rng)?);
    }

    // --- spawn tables -------------------------------------------------
    let mut spawn_points = Vec::new();
    let mut patrol_routes = Vec::new();
    for (ti, table) in source.spawn_tables.iter().enumerate() {
        let mut rng = spawn_stream.fork(ti as u64);
        if !table.nodes.is_empty() {
            let anchors =
                resolve_anchors(source, &instances, &table.nodes, table.elevated)?;
            for k in 0..table.base_count {
                let base = anchors[k as usize % anchors.len()];
                let j = Vec3::new(rng.range(-0.6, 0.6), 0.0, rng.range(-0.6, 0.6));
                spawn_points.push(SpawnPoint {
                    species: table.species.to_string(),
                    pos: base + j,
                    elevated: table.elevated,
                });
            }
        }
        if !table.patrol.is_empty() {
            let pts: Vec<Vec3> = table.patrol.iter().map(|p| v3(*p)).collect();
            for k in 0..table.base_count {
                let base = pts[k as usize % pts.len()];
                let j = Vec3::new(rng.range(-1.0, 1.0), 0.0, rng.range(-1.0, 1.0));
                spawn_points.push(SpawnPoint {
                    species: table.species.to_string(),
                    pos: base + j,
                    elevated: false,
                });
            }
            patrol_routes.push((table.species.to_string(), pts));
        }
    }

    // --- friendlies ----------------------------------------------------
    let mut friendly_setups = Vec::new();
    for (fi, fr) in source.friendlies.iter().enumerate() {
        let mut rng = friendly_stream.fork(fi as u64);
        let behavior = if let Some(pen) = &fr.pen {
            let positions = build_pen(&mut scene, root, pen, fr.count, &mut rng)?;
            FriendlyBehavior::Penned {
                corner: v3(pen.pos),
                size: pen.size,
                positions,
            }
        } else if !fr.patrol.is_empty() {
            FriendlyBehavior::Patrol(fr.patrol.iter().map(|p| v3(*p)).collect())
        } else {
            let anchors = resolve_anchors(source, &instances, &fr.wander_near, false)?;
            FriendlyBehavior::WanderNear(anchors)
        };
        friendly_setups.push(FriendlySetup {
            species: fr.species.to_string(),
            count: fr.count,
            behavior,
        });
    }

    // --- hazards ---------------------------------------------------------
    let mut hazard_volumes = Vec::new();
    for (hi, h) in source.hazards.iter().enumerate() {
        hazard_volumes.push(HazardVolume {
            kind: h.kind,
            volume: resolve_hazard(source, &instances, h, hi)?,
        });
    }

    Ok(ZoneExpansion {
        scene,
        spawn_points,
        patrol_routes,
        friendly_setups,
        hazard_volumes,
        zombie_weight: source.zombie_weight,
        connections: source.connections.clone(),
        ground_biome: source.ambient_biome,
    })
}

/// Gather anchor positions for a list of `Feature("...")` references,
/// erroring on any name no expanded feature carries. With `elevated`, tree
/// features contribute canopy-height anchors; features with no canopy fall
/// back to ground anchors raised 3 m.
fn resolve_anchors(
    source: &ZoneSource,
    instances: &[FeatureInstance],
    refs: &[SpawnRef],
    elevated: bool,
) -> Result<Vec<Vec3>, ParamError> {
    let mut anchors = Vec::new();
    for r in refs {
        let SpawnRef::Feature(name) = r;
        let mut found = false;
        for inst in instances.iter().filter(|i| i.name == name.as_str()) {
            found = true;
            if elevated {
                if inst.elevated.is_empty() {
                    anchors.extend(inst.ground.iter().map(|g| *g + Vec3::new(0.0, 3.0, 0.0)));
                } else {
                    anchors.extend(inst.elevated.iter().copied());
                }
            } else {
                anchors.extend(inst.ground.iter().copied());
            }
        }
        if !found {
            return Err(ParamError::UnresolvedFeature {
                zone: source.name.clone(),
                reference: name.clone(),
            });
        }
    }
    Ok(anchors)
}

/// Resolve one hazard record to a concrete volume.
fn resolve_hazard(
    source: &ZoneSource,
    instances: &[FeatureInstance],
    h: &HazardRecord,
    index: usize,
) -> Result<Volume, ParamError> {
    if let Some(along) = &h.along {
        let path = instances
            .iter()
            .filter(|i| i.name == along.as_str())
            .find_map(|i| i.path.as_ref());
        let Some((points, width)) = path else {
            return Err(ParamError::UnresolvedAlong {
                zone: source.name.clone(),
                reference: along.clone(),
            });
        };
        let width = match h.kind {
            // Banks extend past the water on both sides.
            HazardKind::CreekBank => *width + 2.0,
            _ => *width,
        };
        return Ok(Volume::Polyline {
            points: points.clone(),
            width,
        });
    }
    if let (Some(from), Some(to)) = (h.from, h.to) {
        return Ok(Volume::Segment {
            from: v3(from),
            to: v3(to),
            radius: h.radius_m.unwrap_or(0.5),
        });
    }
    if let Some(pos) = h.pos {
        return Ok(Volume::Sphere {
            center: v3(pos),
            radius: h.radius_m.unwrap_or(1.0),
        });
    }
    Err(ParamError::MalformedHazard {
        zone: source.name.clone(),
        index,
    })
}
