//! Entity types and their temperature model.
//!
//! The key insight driving deadair's thermal mechanic: zombies are dead and have
//! equilibrated to near-ambient temperature.  A human surface temperature is
//! ~33–35 °C; a zombie is only ~0.5–1.5 °C above ambient due to residual
//! decomposition heat.  That tiny ΔT is what makes thermal optics fallible.

use serde::{Deserialize, Serialize};
use crate::vec::Vec3;

/// The broad category of an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntityKind {
    Hunter,
    Zombie,
    Obstacle,
}

/// A live entity in the world: hunter, zombie, or obstacle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: u32,
    pub kind: EntityKind,
    pub position: Vec3,
    /// Surface temperature in degrees Celsius.
    /// Humans: ~33.5 °C.  Zombies: ambient + small offset.
    pub temperature_c: f32,
    pub alive: bool,
}

impl Entity {
    /// Create a hunter with a human surface temperature.
    pub fn hunter(id: u32, position: Vec3) -> Self {
        Self {
            id,
            kind: EntityKind::Hunter,
            position,
            temperature_c: 33.5,
            alive: true,
        }
    }

    /// Create a zombie at ambient temperature plus a small decomposition offset.
    ///
    /// `ambient_c` is the scene's background temperature; `offset_c` is the
    /// additional warmth from decomposition chemistry (typically 0.5–1.5 °C).
    pub fn zombie(id: u32, position: Vec3, ambient_c: f32, offset_c: f32) -> Self {
        Self {
            id,
            kind: EntityKind::Zombie,
            position,
            temperature_c: ambient_c + offset_c,
            alive: true,
        }
    }

    /// Override temperature (builder-style).
    pub fn with_temperature(mut self, temp_c: f32) -> Self {
        self.temperature_c = temp_c;
        self
    }
}
