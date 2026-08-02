//! Thermal imaging optic with physically-honest detection modelling.
//!
//! Real thermal cameras are characterised by their **Noise Equivalent
//! Temperature Difference** (NETD) — typically 25–100 mK for commercial/mil
//! units — and their instantaneous field of view (IFOV).  This module
//! implements a simplified but honest model of those physics so that "cold
//! zombies" (ΔT ≈ 0.8 °C above ambient) are genuinely harder to spot than
//! living humans (ΔT ≈ 28 °C above ambient at 5 °C ambient).

use serde::{Deserialize, Serialize};
use crate::entity::Entity;

/// A thermal imaging optic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalOptics {
    /// Noise Equivalent Temperature Difference in millikelvin (mK).
    /// Lower values = higher sensitivity.  25 mK is high-end; 80 mK is budget.
    pub netd_mk: f32,

    /// Full horizontal field of view in degrees (e.g. 14 ° for a 35 mm lens).
    pub fov_deg: f32,

    /// Maximum useful detection range in metres.
    pub max_range_m: f32,

    /// Atmospheric thermal attenuation coefficient in km⁻¹.
    /// Clear night ≈ 0.05 km⁻¹; humid/foggy ≈ 0.3 km⁻¹.
    pub atm_atten_per_km: f32,
}

impl ThermalOptics {
    /// Budget unit (≈ Pulsar Axion / FLIR Scout).
    pub fn budget() -> Self {
        Self {
            netd_mk: 80.0,
            fov_deg: 20.0,
            max_range_m: 300.0,
            atm_atten_per_km: 0.05,
        }
    }

    /// Military-grade cooled core.
    pub fn military_grade() -> Self {
        Self {
            netd_mk: 25.0,
            fov_deg: 14.0,
            max_range_m: 1_000.0,
            atm_atten_per_km: 0.05,
        }
    }

    /// Atmospheric transmission fraction at `d_m` metres using Beer–Lambert law:
    /// τ(d) = exp(−α · d), where α = `atm_atten_per_km` / 1000 m⁻¹.
    pub fn transmission(&self, d_m: f32) -> f32 {
        let alpha_per_m = self.atm_atten_per_km / 1_000.0;
        (-alpha_per_m * d_m).exp()
    }

    /// Signal-to-noise ratio for detecting `target` from `observer`.
    ///
    /// The observer is facing `heading_deg` (0 ° = +X / East, CCW positive).
    /// Returns `None` if the target is outside the FOV or beyond `max_range_m`.
    pub fn signal_to_noise(
        &self,
        observer: &Entity,
        heading_deg: f32,
        target: &Entity,
        ambient_c: f32,
    ) -> Option<f32> {
        let dx = target.position.x - observer.position.x;
        let dy = target.position.y - observer.position.y;
        let d = (dx * dx + dy * dy).sqrt();

        if d > self.max_range_m || d < 1e-3 {
            return None;
        }

        // Angular offset between observer heading and target bearing (degrees).
        let target_bearing_deg = dy.atan2(dx).to_degrees();
        let mut angle_diff = (target_bearing_deg - heading_deg).abs() % 360.0;
        if angle_diff > 180.0 {
            angle_diff = 360.0 - angle_diff;
        }
        if angle_diff > self.fov_deg / 2.0 {
            return None;
        }

        // Apparent temperature contrast after atmospheric absorption.
        let delta_t_c = (target.temperature_c - ambient_c).abs();
        let effective_signal = delta_t_c * self.transmission(d);

        // NETD in °C (1 K = 1 °C for differences).
        let netd_c = self.netd_mk / 1_000.0;

        Some(effective_signal / netd_c)
    }

    /// Detection probability (0.0–1.0) on a single observation frame.
    ///
    /// Modelled as a logistic function over SNR: P ≈ 1 at SNR ≥ 5,
    /// P = 0.5 at SNR = 1.5, P ≈ 0 at SNR < 0.1.
    pub fn detection_probability(
        &self,
        observer: &Entity,
        heading_deg: f32,
        target: &Entity,
        ambient_c: f32,
    ) -> f32 {
        match self.signal_to_noise(observer, heading_deg, target, ambient_c) {
            None => 0.0,
            Some(snr) => {
                let k = 2.0_f32;
                let mid = 1.5_f32;
                1.0 / (1.0 + (-k * (snr - mid)).exp())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vec::Vec3;

    fn observer() -> Entity {
        Entity::hunter(0, Vec3::new(0.0, 0.0, 0.0))
    }

    #[test]
    fn warm_human_has_high_snr() {
        let optics = ThermalOptics::budget();
        let human = Entity::hunter(1, Vec3::new(50.0, 0.0, 0.0));
        // Human at ~33.5 °C in 5 °C ambient: ΔT = 28.5 °C
        let snr = optics.signal_to_noise(&observer(), 0.0, &human, 5.0).unwrap();
        assert!(snr > 10.0, "Expected high SNR for warm human, got {snr}");
    }

    #[test]
    fn cold_zombie_has_low_snr() {
        let optics = ThermalOptics::budget();
        // Zombie at +0.1 °C above ambient (nearly ambient — dead body equilibrated)
        let zombie = Entity::zombie(1, Vec3::new(50.0, 0.0, 0.0), 5.0, 0.1);
        // ΔT = 0.1 °C, NETD = 80 mK → SNR ≈ 1.25 (just above threshold)
        let snr = optics.signal_to_noise(&observer(), 0.0, &zombie, 5.0).unwrap();
        assert!(snr < 3.0, "Cold zombie SNR should be low, got {snr:.3}");
    }

    #[test]
    fn cold_zombie_harder_to_detect_than_human() {
        let optics = ThermalOptics::budget();
        let human  = Entity::hunter(1, Vec3::new(50.0, 0.0, 0.0));
        // Zombie ΔT ≈ 0.1 °C — nearly ambient; human ΔT ≈ 28.5 °C
        let zombie = Entity::zombie(2, Vec3::new(50.0, 0.0, 0.0), 5.0, 0.1);
        let p_human = optics.detection_probability(&observer(), 0.0, &human,  5.0);
        let p_zombie = optics.detection_probability(&observer(), 0.0, &zombie, 5.0);
        assert!(p_human > p_zombie,
            "Human (P={p_human:.3}) should be easier to detect than cold zombie (P={p_zombie:.3})");
    }

    #[test]
    fn out_of_range_returns_zero() {
        let optics = ThermalOptics::budget();
        let human = Entity::hunter(1, Vec3::new(500.0, 0.0, 0.0)); // 500 m, max 300
        assert_eq!(optics.detection_probability(&observer(), 0.0, &human, 5.0), 0.0);
    }

    #[test]
    fn outside_fov_returns_zero() {
        let optics = ThermalOptics::budget(); // FOV = 20 °
        // Target is at 90 ° from heading (dead perpendicular) — outside FOV
        let human = Entity::hunter(1, Vec3::new(0.0, 50.0, 0.0));
        let p = optics.detection_probability(&observer(), 0.0, &human, 5.0);
        assert_eq!(p, 0.0);
    }
}
