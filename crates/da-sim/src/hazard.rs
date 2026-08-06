//! Hazards and player health (SRS §3.5, FR-H1..H4).
//!
//! The optics detail lives elsewhere; the sim only needs to answer "can
//! this channel see a trip hazard?" — NV yes, thermal no (FR-H2).

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Minimal optic channel tag for hazard visibility queries. The full
/// optics ladder (sensors, batteries, palettes) lives in the render layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Optic {
    /// Naked eye — sees hazards faintly (moon-dependent scaling is the
    /// render layer's job; the sim answers "physically visible": yes).
    Eye,
    /// Night vision — sees all terrain and hazards.
    Nv,
    /// Thermal — terrain hazards have no ΔT: NOT visible (FR-H2).
    Thermal,
}

/// Trip-hazard classes (FR-H1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HazardKind {
    /// Groundhog hole, post hole.
    Hole,
    /// Field wire runs.
    Wire,
    /// Downed limbs.
    Limb,
    /// Slick creek banks.
    CreekBank,
    /// Drowning-adjacent water hazard — worst of the lot.
    Water,
}

impl HazardKind {
    /// Base probability of tripping when walking into the hazard at
    /// normal walking speed, before weather scaling.
    pub fn trip_chance(self) -> f32 {
        match self {
            HazardKind::Hole => 0.45,
            HazardKind::Wire => 0.50,
            HazardKind::Limb => 0.30,
            HazardKind::CreekBank => 0.35,
            HazardKind::Water => 0.60,
        }
    }

    /// Base damage of a trip/fall, before weather and speed scaling.
    pub fn base_damage(self) -> f32 {
        match self {
            HazardKind::Hole => 8.0,
            HazardKind::Wire => 6.0,
            HazardKind::Limb => 5.0,
            HazardKind::CreekBank => 10.0,
            HazardKind::Water => 14.0,
        }
    }
}

/// A placed hazard volume.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Hazard {
    /// Class.
    pub kind: HazardKind,
    /// Center, world meters.
    pub pos: Vec3,
    /// Trigger radius (m).
    pub radius: f32,
}

impl Hazard {
    /// FR-H2: trip hazards render in NV and (moon-dependent) naked eye,
    /// but never reliably in thermal.
    pub fn visible_in(&self, optic: Optic) -> bool {
        match optic {
            Optic::Eye | Optic::Nv => true,
            Optic::Thermal => false,
        }
    }

    /// Is a ground point inside the hazard (horizontal test)?
    pub fn contains(&self, p: Vec3) -> bool {
        let d = p - self.pos;
        Vec3::new(d.x, 0.0, d.z).length() < self.radius
    }
}

/// Player health pool (FR-H1). Recovers only at camp (FR-H3): the sim
/// exposes [`Health::heal_full`] and the camp layer decides when.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Health {
    /// Current hit points.
    pub hp: f32,
    /// Maximum hit points.
    pub max: f32,
}

impl Health {
    /// Full pool of `max` hp.
    pub fn new(max: f32) -> Self {
        Self { hp: max, max }
    }

    /// Apply damage, clamped at zero.
    pub fn damage(&mut self, amount: f32) {
        self.hp = (self.hp - amount.max(0.0)).max(0.0);
    }

    /// FR-H4: zero health ends the night.
    pub fn is_dead(&self) -> bool {
        self.hp <= 0.0
    }

    /// Camp-only full recovery (FR-H3).
    pub fn heal_full(&mut self) {
        self.hp = self.max;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hazard_invisible_in_thermal_visible_in_nv() {
        let h = Hazard { kind: HazardKind::Wire, pos: Vec3::ZERO, radius: 1.0 };
        assert!(h.visible_in(Optic::Nv), "FR-H2: NV sees hazards");
        assert!(h.visible_in(Optic::Eye));
        assert!(!h.visible_in(Optic::Thermal), "FR-H2: thermal must NOT see hazards");
    }

    #[test]
    fn health_clamps_and_reports_death() {
        let mut h = Health::new(100.0);
        h.damage(30.0);
        assert_eq!(h.hp, 70.0);
        h.damage(500.0);
        assert!(h.is_dead());
        assert_eq!(h.hp, 0.0);
        h.heal_full();
        assert_eq!(h.hp, 100.0);
    }

    #[test]
    fn contains_is_horizontal() {
        let h = Hazard { kind: HazardKind::Hole, pos: Vec3::new(5.0, 0.0, 5.0), radius: 1.0 };
        assert!(h.contains(Vec3::new(5.5, 1.6, 5.0))); // eye height irrelevant
        assert!(!h.contains(Vec3::new(7.0, 0.0, 5.0)));
    }
}
