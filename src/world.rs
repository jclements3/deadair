//! Runtime world state derived from a [`Scene`].
//!
//! A `World` is the live, mutable representation used by the hunt simulation:
//! entities can be killed, positions can change, but the underlying scene
//! document remains immutable.

use serde::{Deserialize, Serialize};
use crate::{
    entity::{Entity, EntityKind},
    scene::{NodeKind, Scene},
    vec::Vec3,
};

/// An axis-aligned box obstacle (building, wall, crate …).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Obstacle {
    pub position: Vec3,
    /// [width, depth, height] in metres.
    pub size: [f32; 3],
}

/// Live world state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct World {
    pub ambient_temp_c: f32,
    pub entities: Vec<Entity>,
    pub obstacles: Vec<Obstacle>,
    pub width_m: f32,
    pub depth_m: f32,
}

impl World {
    /// Build a `World` from a [`Scene`] document.
    pub fn from_scene(scene: &Scene) -> Self {
        let mut world = World {
            ambient_temp_c: scene.ambient_temp_c,
            entities: Vec::new(),
            obstacles: Vec::new(),
            width_m: 100.0,
            depth_m: 100.0,
        };

        let mut next_id: u32 = 0;

        for node in &scene.nodes {
            let pos = node.world_position(Vec3::zero());
            match &node.kind {
                NodeKind::Terrain { size, .. } => {
                    world.width_m = size[0];
                    world.depth_m = size[1];
                }
                NodeKind::Box { size } => {
                    world.obstacles.push(Obstacle { position: pos, size: *size });
                }
                NodeKind::Cylinder { radius_m, height_m } => {
                    let d = radius_m * 2.0;
                    world.obstacles.push(Obstacle { position: pos, size: [d, d, *height_m] });
                }
                NodeKind::Zombie { ambient_offset_c } => {
                    world.entities.push(Entity::zombie(next_id, pos, scene.ambient_temp_c, *ambient_offset_c));
                    next_id += 1;
                }
                NodeKind::HunterSpawn => {
                    world.entities.push(Entity::hunter(next_id, pos));
                    next_id += 1;
                }
                NodeKind::Light { .. } => {} // lighting not yet used in detection model
            }
        }

        world
    }

    /// Iterate over all living hunters.
    pub fn hunters(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter(|e| e.kind == EntityKind::Hunter && e.alive)
    }

    /// Iterate over all living zombies.
    pub fn zombies(&self) -> impl Iterator<Item = &Entity> {
        self.entities.iter().filter(|e| e.kind == EntityKind::Zombie && e.alive)
    }

    pub fn zombie_count(&self) -> usize { self.zombies().count() }
}
