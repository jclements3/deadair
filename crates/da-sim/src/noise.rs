//! Noise events (SDD §5.3, FR-W7): every discharge and pump stroke rings a
//! bell in the dark. Pests inside the radius flee; zombies inside it
//! pathfind toward the source for 60 seconds.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Moderator noise-radius multiplier — a 70% reduction, satisfying the
/// FR-W7 "at least 70%" bound exactly.
pub const MODERATOR_FACTOR: f32 = 0.3;

/// What made the noise (species react identically; render/audio may not).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseKind {
    /// Rifle discharge.
    Discharge,
    /// Multi-pump stroke (movement noise).
    PumpStroke,
    /// Anything else the caller wants heard (footfall, dropped gear...).
    Other,
}

/// A propagating noise: everything within `radius_m` of `pos` reacts on
/// the next sim tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct NoiseEvent {
    /// Source position.
    pub pos: Vec3,
    /// Reaction radius in meters.
    pub radius_m: f32,
    /// Source classification.
    pub kind: NoiseKind,
}

impl NoiseEvent {
    /// Is `p` inside the reaction radius (scaled by `mult`)?
    pub fn reaches(&self, p: Vec3, mult: f32) -> bool {
        (p - self.pos).length() < self.radius_m * mult
    }
}

/// Discharge noise radius (m) from muzzle energy: louder with power/tier,
/// cut to 30% by a moderator (FR-W7).
pub fn discharge_noise_radius_m(energy_fpe: f32, moderated: bool) -> f32 {
    let base = 30.0 + 2.2 * energy_fpe.max(0.0);
    if moderated {
        base * MODERATOR_FACTOR
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moderator_cuts_radius_at_least_70_percent() {
        for e in [6.0_f32, 12.8, 22.0, 26.0, 45.0] {
            let loud = discharge_noise_radius_m(e, false);
            let quiet = discharge_noise_radius_m(e, true);
            assert!(
                quiet <= loud * 0.3 + 1e-4,
                "FR-W7: moderated {quiet} must be <=30% of {loud}"
            );
        }
    }

    #[test]
    fn louder_with_more_energy() {
        assert!(
            discharge_noise_radius_m(45.0, false) > discharge_noise_radius_m(12.8, false)
        );
    }

    #[test]
    fn reaches_respects_multiplier() {
        let n = NoiseEvent { pos: Vec3::ZERO, radius_m: 10.0, kind: NoiseKind::Discharge };
        let p = Vec3::new(12.0, 0.0, 0.0);
        assert!(!n.reaches(p, 1.0));
        assert!(n.reaches(p, 1.5)); // spooked raccoon group hears farther
        assert!(!n.reaches(Vec3::new(4.0, 0.0, 0.0), 0.3)); // frozen possum barely listens
    }
}
