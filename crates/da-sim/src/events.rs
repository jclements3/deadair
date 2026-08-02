//! Events out of the sim — the decoupling seam. Economy, render, audio,
//! and thermal layers consume these without reaching into sim state.

use da_core::EntityId;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::hazard::HazardKind;
use crate::hit::Species;
use crate::noise::NoiseKind;

/// Residual-heat classes for the thermal layer (FR-T4, SDD §2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeatKind {
    /// Fired barrel (warm ~90 s).
    Barrel,
    /// Pellet strike on terrain (~5 s).
    PelletImpact,
    /// Animal rested >30 s — a bedding spot, the tracking mechanic.
    Bedding,
}

/// Why the player took damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DamageCause {
    /// Tripped/fell in a hazard.
    Hazard(HazardKind),
    /// Zombie contact damage.
    ZombieContact,
}

/// Everything the sim reports per tick. Ordering within a tick is
/// deterministic for a given seed and command script.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SimEvent {
    /// Pest headshot inside lethal range. `bounty_eligible` tags pests;
    /// license checks happen in the economy layer.
    KillConfirmed {
        /// Victim.
        id: EntityId,
        /// Victim species.
        species: Species,
        /// Pest kill (zombies never pay — FR-A7).
        bounty_eligible: bool,
        /// Impact point (for residue/pickup).
        pos: Vec3,
    },
    /// Pest wounded: no bounty, victim flees, neighbours alerted (FR-A3).
    Wounded {
        /// Victim.
        id: EntityId,
        /// Victim species.
        species: Species,
        /// Impact point.
        pos: Vec3,
    },
    /// Protected animal struck (directly or via backstop) — money and
    /// reputation penalty upstream (FR-A6/A8).
    FriendlyHit {
        /// Victim.
        id: EntityId,
        /// Victim species.
        species: Species,
    },
    /// Zombie headshot.
    ZombieDestroyed {
        /// Victim.
        id: EntityId,
    },
    /// Zombie body hit — brief movement pause only.
    ZombieStaggered {
        /// Victim.
        id: EntityId,
    },
    /// Player lost health.
    PlayerDamaged {
        /// Hit points lost.
        amount: f32,
        /// Source.
        cause: DamageCause,
    },
    /// A noise rang out (discharge, pump stroke, ...).
    NoiseMade {
        /// Source position.
        pos: Vec3,
        /// Reaction radius (m).
        radius_m: f32,
        /// Source classification.
        kind: NoiseKind,
    },
    /// Residual heat spawned for the thermal layer.
    HeatResidue {
        /// Residue class.
        kind: HeatKind,
        /// World position.
        pos: Vec3,
    },
    /// A pest broke into flight (noise, light, or alert pulse).
    PestFled {
        /// The fleeing animal.
        id: EntityId,
        /// Its species.
        species: Species,
    },
    /// Shot connected with nothing living.
    Missed {
        /// Terrain impact, if any (heat residue already emitted).
        impact: Option<Vec3>,
    },
    /// Trigger pulled on an empty plant (0 pumps / empty reservoir).
    DryFire,
}
