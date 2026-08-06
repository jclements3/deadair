//! Drag-free parabolic ballistics (SDD §5.1): readable, learnable, cheap.
//!
//! The pellet flies a straight line at muzzle velocity; gravity contributes
//! a parabolic drop `0.5 * g * t^2` scaled by the pellet's drop constant.
//! Higher power → higher velocity → flatter trajectory (FR-W5).

use da_core::Rng;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::weapon::FPE_TO_J;

/// Gravitational acceleration, m/s².
pub const GRAVITY: f32 = 9.81;

/// Muzzle velocity (m/s) from energy (FPE) and pellet mass (grams).
pub fn muzzle_velocity_mps(energy_fpe: f32, pellet_mass_g: f32) -> f32 {
    let e_j = energy_fpe.max(0.0) * FPE_TO_J;
    let m_kg = (pellet_mass_g * 1e-3).max(1e-6);
    (2.0 * e_j / m_kg).sqrt()
}

/// Parabolic drop (m) at `range_m` for a shot at `velocity_mps`.
/// `drop_scale` is the pellet variant's drop-constant multiplier.
pub fn drop_at(range_m: f32, velocity_mps: f32, drop_scale: f32) -> f32 {
    if velocity_mps <= 0.0 {
        return f32::INFINITY;
    }
    let t = range_m / velocity_mps;
    0.5 * GRAVITY * t * t * drop_scale
}

/// Holdover solution for a given range.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AimSolution {
    /// Aim this many meters above the desired impact point.
    pub holdover_m: f32,
    /// The same holdover as an angular correction (mrad) for reticle use.
    pub holdover_mrad: f32,
    /// Pellet time of flight, seconds.
    pub time_of_flight_s: f32,
}

/// Compute the holdover that puts the pellet on target at `range_m`.
pub fn aim_solution(range_m: f32, velocity_mps: f32, drop_scale: f32) -> AimSolution {
    let drop = drop_at(range_m, velocity_mps, drop_scale);
    AimSolution {
        holdover_m: drop,
        holdover_mrad: if range_m > 0.0 { drop / range_m * 1000.0 } else { 0.0 },
        time_of_flight_s: if velocity_mps > 0.0 { range_m / velocity_mps } else { f32::INFINITY },
    }
}

/// Lethal-range table: humane-kill distance (m) as a function of muzzle
/// energy, scaled per pellet variant. Tuned so Tier 1 (≈13 FPE) is a
/// ~30 m rat gun and Tier 4 (45 FPE) reaches ~55 m.
pub fn lethal_range_m(energy_fpe: f32, lethal_scale: f32) -> f32 {
    8.0 * energy_fpe.max(0.0).sqrt() * lethal_scale
}

/// Deterministically perturb an aim direction inside a cone of
/// `max_angle_rad` (dispersion radius). Uniform over the disc, so group
/// density is highest at center.
pub fn perturb_direction(dir: Vec3, max_angle_rad: f32, rng: &mut Rng) -> Vec3 {
    let d = dir.normalize_or_zero();
    if d == Vec3::ZERO || max_angle_rad <= 0.0 {
        return d;
    }
    let ortho = if d.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = d.cross(ortho).normalize();
    let v = d.cross(u);
    let ang = max_angle_rad * rng.f32().sqrt();
    let theta = rng.range(0.0, std::f32::consts::TAU);
    (d + u * (ang * theta.cos()) + v * (ang * theta.sin())).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weapon::{PowerPlant, PowerSetting, RifleConfig};

    #[test]
    fn drop_increases_with_range_and_decreases_with_power() {
        let r = RifleConfig::tier3();
        let d10 = r.drop_at(10.0).unwrap();
        let d30 = r.drop_at(30.0).unwrap();
        let d50 = r.drop_at(50.0).unwrap();
        assert!(d10 < d30 && d30 < d50, "drop grows with range");

        // Same rifle, LOW vs HIGH power: higher power flattens trajectory.
        let mut low = RifleConfig::tier3();
        let mut high = RifleConfig::tier3();
        if let PowerPlant::RegulatedPcp { power, .. } = &mut low.plant {
            *power = PowerSetting::Low;
        }
        if let PowerPlant::RegulatedPcp { power, .. } = &mut high.plant {
            *power = PowerSetting::High;
        }
        assert!(
            high.drop_at(30.0).unwrap() < low.drop_at(30.0).unwrap(),
            "FR-W5: higher power flattens trajectory"
        );
    }

    #[test]
    fn aim_solution_matches_drop() {
        let sol = aim_solution(40.0, 250.0, 1.0);
        assert!((sol.holdover_m - drop_at(40.0, 250.0, 1.0)).abs() < 1e-6);
        assert!((sol.time_of_flight_s - 0.16).abs() < 1e-3);
        assert!(sol.holdover_mrad > 0.0);
    }

    #[test]
    fn lethal_range_scales_with_energy_and_pellet() {
        assert!(lethal_range_m(45.0, 1.0) > lethal_range_m(12.8, 1.0));
        assert!(lethal_range_m(26.0, 1.12) > lethal_range_m(26.0, 1.0));
        let r = RifleConfig::tier1(); // unpumped
        assert_eq!(r.lethal_range_m(), 0.0);
        assert!(!r.lethal_at(5.0));
    }

    #[test]
    fn perturbation_stays_inside_cone_and_is_deterministic() {
        let mut a = da_core::Rng::new(11);
        let mut b = da_core::Rng::new(11);
        let dir = Vec3::new(0.2, -0.1, 1.0).normalize();
        for _ in 0..500 {
            let pa = perturb_direction(dir, 0.004, &mut a);
            let pb = perturb_direction(dir, 0.004, &mut b);
            assert_eq!(pa, pb);
            let cos = pa.dot(dir).clamp(-1.0, 1.0);
            assert!(cos.acos() <= 0.004 * 1.01, "inside dispersion cone");
        }
    }
}
