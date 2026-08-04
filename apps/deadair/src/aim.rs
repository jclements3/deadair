//! The aiming layer: mil-scale reticle math, ballistic shot solutions
//! (drop + wind drift), and target selection.
//!
//! Control scheme (design direction): left mouse selects the target, the
//! sights sit fixed at the center of the square view with graduated scale
//! axes, middle-drag pans the view to align the shot, the scroll wheel
//! zooms, and right mouse takes the shot. Range and windage are accounted
//! for **with the scale**: every shot obeys real drop and drift, and the
//! mil axes are the player's compensation tool. Owning the laser
//! rangefinder additionally displays the measured range and a computed
//! holdover chevron on the scale.

use glam::Vec3;

/// One milliradian in radians. The scale axes are graduated in mils; at
/// 100 m one mil subtends 10 cm — the natural unit for holdover.
pub const MIL: f32 = 1.0e-3;

/// Pixels per mil for a square viewport of `side_px` at `fov_y_deg`.
///
/// Zooming narrows the FOV, so the same mil spacing spreads across more
/// pixels — exactly how a real FFP (first-focal-plane) reticle behaves.
pub fn px_per_mil(side_px: f32, fov_y_deg: f32) -> f32 {
    let fov_rad = fov_y_deg.to_radians();
    side_px / fov_rad * MIL
}

/// Magnification → vertical field of view. 1× is the 60° unaided view;
/// the ladder tops out at 14.5× (the smart-scope footage's readout).
pub fn fov_for_mag(mag: f32) -> f32 {
    60.0 / mag.clamp(1.0, 14.5)
}

/// A computed firing solution along the current sight axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShotSolution {
    /// Range to the first surface under the reticle, meters.
    pub range_m: f32,
    /// Gravity drop at that range, meters (positive down).
    pub drop_m: f32,
    /// Wind drift at that range, meters (signed along `wind_side`).
    pub drift_m: f32,
    /// Drop expressed in mils below the crosshair.
    pub drop_mil: f32,
    /// Drift expressed in mils (positive = pellet pushed toward +side).
    pub drift_mil: f32,
    /// Where a pellet fired along the axis actually lands.
    pub impact: Vec3,
    /// The world direction the sim should resolve (axis bent by physics).
    pub dir: Vec3,
}

/// Compute what a pellet fired along `axis` from `eye` really does over
/// `range_m`, under horizontal wind `wind_mps`.
///
/// Drag-free time of flight `t = range / v0`; drop `½gt²` (matching
/// da-sim's ballistics table); drift approximates a light pellet's
/// crosswind sensitivity as `cross_wind × t × DRIFT_FACTOR` — pellets are
/// notoriously wind-sensitive, and the factor stands in for the drag a
/// point-mass model doesn't carry.
pub fn solve(
    eye: Vec3,
    axis: Vec3,
    range_m: f32,
    muzzle_mps: f32,
    drop_scale: f32,
    wind_mps: Vec3,
) -> ShotSolution {
    const GRAVITY: f32 = 9.81;
    /// Drag proxy: how much of the crosswind a pellet picks up.
    const DRIFT_FACTOR: f32 = 1.5;

    let axis = axis.normalize_or_zero();
    let range_m = range_m.max(1.0);
    let t = if muzzle_mps > 0.0 {
        range_m / muzzle_mps
    } else {
        f32::INFINITY
    };
    let drop_m = 0.5 * GRAVITY * t * t * drop_scale;

    // Crosswind component: the wind minus its along-axis part.
    let along = axis * wind_mps.dot(axis);
    let cross = wind_mps - along;
    let side = {
        let s = axis.cross(Vec3::Y).normalize_or_zero();
        if s.length_squared() < 0.5 {
            Vec3::X // aiming straight up/down: any horizontal basis works
        } else {
            s
        }
    };
    let drift_signed = cross.dot(side) * t * DRIFT_FACTOR;

    let aim_point = eye + axis * range_m;
    let impact = aim_point - Vec3::Y * drop_m + side * drift_signed;
    let dir = (impact - eye).normalize_or_zero();

    let mil = |m: f32| m / range_m / MIL;
    ShotSolution {
        range_m,
        drop_m,
        drift_m: drift_signed,
        drop_mil: mil(drop_m),
        drift_mil: mil(drift_signed),
        impact,
        dir,
    }
}

/// Roll the night's wind from the forecast (speed band) and a seed angle.
/// Returned vector is horizontal, in m/s.
pub fn roll_wind(forecast: da_core::Forecast, rng: &mut da_core::Rng) -> Vec3 {
    use da_core::Forecast::*;
    let (lo, hi) = match forecast {
        Fog => (0.0, 1.0),
        Clear | HeatWave => (0.0, 3.0),
        ColdSnap => (1.0, 4.0),
        Overcast => (2.0, 5.0),
        Rain => (4.0, 8.0),
        PreStorm => (6.0, 12.0),
    };
    let speed = rng.range(lo, hi);
    let angle = rng.range(0.0, std::f32::consts::TAU);
    Vec3::new(angle.cos(), 0.0, angle.sin()) * speed
}

/// Pick the selectable animal nearest the sight axis: smallest angular
/// offset from `axis`, within `max_offset_mil`. Returns index into the
/// candidate list.
pub fn pick_nearest_axis(
    eye: Vec3,
    axis: Vec3,
    candidates: &[(usize, Vec3)],
    max_offset_mil: f32,
) -> Option<usize> {
    let axis = axis.normalize_or_zero();
    let mut best: Option<(usize, f32)> = None;
    for (idx, pos) in candidates {
        let to = (*pos - eye).normalize_or_zero();
        let cos = to.dot(axis).clamp(-1.0, 1.0);
        let offset_mil = cos.acos() / MIL;
        if offset_mil <= max_offset_mil && best.map_or(true, |(_, b)| offset_mil < b) {
            best = Some((*idx, offset_mil));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_core::{Forecast, Rng};

    const V0: f32 = 250.0; // ~tier-2 .22 muzzle velocity, m/s

    #[test]
    fn drop_grows_quadratically_with_range() {
        let near = solve(Vec3::ZERO, Vec3::NEG_Z, 20.0, V0, 1.0, Vec3::ZERO);
        let far = solve(Vec3::ZERO, Vec3::NEG_Z, 60.0, V0, 1.0, Vec3::ZERO);
        assert!(far.drop_m > near.drop_m * 8.0, "3x range ≈ 9x drop");
        // Drop in mils also grows (linearly) with range — the scale matters
        // more the farther out you shoot.
        assert!(far.drop_mil > near.drop_mil * 2.5);
    }

    #[test]
    fn no_wind_means_no_drift_and_dir_bends_only_down() {
        let s = solve(Vec3::new(0.0, 1.6, 0.0), Vec3::NEG_Z, 50.0, V0, 1.0, Vec3::ZERO);
        assert_eq!(s.drift_m, 0.0);
        assert!(s.dir.y < -0.0, "pellet path bends below the axis");
        assert!(s.dir.x.abs() < 1e-6);
    }

    #[test]
    fn crosswind_drifts_downwind_and_flips_with_direction() {
        // Aiming -Z; +X wind pushes the impact toward +X.
        let s = solve(Vec3::ZERO, Vec3::NEG_Z, 50.0, V0, 1.0, Vec3::new(4.0, 0.0, 0.0));
        assert!(s.impact.x > 0.05, "impact drifted downwind: {}", s.impact.x);
        let s2 = solve(Vec3::ZERO, Vec3::NEG_Z, 50.0, V0, 1.0, Vec3::new(-4.0, 0.0, 0.0));
        assert!(s2.impact.x < -0.05);
        assert!((s.drift_m + s2.drift_m).abs() < 1e-4, "symmetric");
    }

    #[test]
    fn headwind_produces_no_lateral_drift() {
        let s = solve(Vec3::ZERO, Vec3::NEG_Z, 50.0, V0, 1.0, Vec3::new(0.0, 0.0, -6.0));
        assert!(s.drift_m.abs() < 1e-4);
    }

    #[test]
    fn ffp_reticle_scale_spreads_with_zoom() {
        let unzoomed = px_per_mil(1024.0, fov_for_mag(1.0));
        let zoomed = px_per_mil(1024.0, fov_for_mag(10.0));
        assert!((zoomed / unzoomed - 10.0).abs() < 0.01, "10x zoom = 10x px/mil");
        // At 14.5x a mil is big enough to hold with: > 10 px.
        assert!(px_per_mil(1024.0, fov_for_mag(14.5)) > 10.0);
    }

    #[test]
    fn wind_bands_follow_the_forecast() {
        let mut rng = Rng::new(7);
        for _ in 0..50 {
            assert!(roll_wind(Forecast::Fog, &mut rng).length() <= 1.0 + 1e-4);
            let storm = roll_wind(Forecast::PreStorm, &mut rng).length();
            assert!((6.0..=12.0).contains(&storm));
            let w = roll_wind(Forecast::Clear, &mut rng);
            assert_eq!(w.y, 0.0, "wind is horizontal");
        }
    }

    #[test]
    fn picking_prefers_the_target_nearest_the_axis() {
        let eye = Vec3::ZERO;
        let axis = Vec3::NEG_Z;
        let candidates = vec![
            (7usize, Vec3::new(0.5, 0.0, -40.0)),  // ~12.5 mil off
            (9usize, Vec3::new(0.1, 0.0, -40.0)),  // ~2.5 mil off
            (11usize, Vec3::new(8.0, 0.0, -40.0)), // way off
        ];
        assert_eq!(pick_nearest_axis(eye, axis, &candidates, 30.0), Some(9));
        // Tight threshold: nothing qualifies.
        assert_eq!(pick_nearest_axis(eye, axis, &candidates, 1.0), None);
    }
}

// ---------------------------------------------------------------------
// Sway and breath (spec: difficulty comes from SWAY, not spread)
// ---------------------------------------------------------------------

/// Baseline standing sway amplitude, radians (~2 mils — visible at 6×,
/// decisive at 14×). Stance/rest multipliers scale it down from here.
pub const SWAY_BASE_RAD: f32 = 0.002;

/// Slow Lissajous reticle drift. Pure function of time and seed so replays
/// are identical; the two incommensurate frequencies never quite repeat.
pub fn sway_offset(t: f32, seed: u32, amplitude_rad: f32) -> glam::Vec2 {
    let p1 = (seed % 977) as f32 * 0.13;
    let p2 = (seed % 787) as f32 * 0.29;
    let yaw = (t * 0.9 + p1).sin() + 0.45 * (t * 2.33 + p2).sin();
    let pitch = (t * 1.17 + p2).sin() + 0.45 * (t * 2.91 + p1).sin();
    glam::Vec2::new(yaw, pitch * 0.75) * amplitude_rad * (1.0 / 1.45)
}

/// Hold-breath state: damps sway to 20% for a few seconds, then the body
/// demands payment — sway overshoots while you recover.
#[derive(Debug, Clone)]
pub struct Breath {
    /// Seconds of hold remaining (refills when not holding).
    capacity: f32,
    /// Overshoot debt, decays toward zero.
    debt: f32,
}

/// Full lungs: how long a hold lasts.
pub const BREATH_CAPACITY_S: f32 = 4.0;
/// Sway multiplier while holding.
pub const BREATH_DAMP: f32 = 0.2;
/// Peak sway multiplier right after a full exhale.
pub const BREATH_OVERSHOOT: f32 = 1.6;

impl Default for Breath {
    fn default() -> Self {
        Self {
            capacity: BREATH_CAPACITY_S,
            debt: 0.0,
        }
    }
}

impl Breath {
    /// Advance one frame. `holding` is the hold-breath input.
    pub fn update(&mut self, dt: f32, holding: bool) {
        if holding {
            if self.capacity > 0.0 {
                self.capacity = (self.capacity - dt).max(0.0);
                // Debt accrues with how much breath has been spent.
                self.debt = (1.0 - self.capacity / BREATH_CAPACITY_S).min(1.0);
            }
            // Holding on empty lungs: nothing refills, the debt stands —
            // the body doesn't recover until you actually breathe.
        } else {
            self.capacity = (self.capacity + dt * 0.8).min(BREATH_CAPACITY_S);
            self.debt = (self.debt - dt / 2.5).max(0.0);
        }
    }

    /// Current sway amplitude multiplier.
    pub fn sway_factor(&self, holding: bool) -> f32 {
        if holding && self.capacity > 0.0 {
            BREATH_DAMP
        } else {
            1.0 + (BREATH_OVERSHOOT - 1.0) * self.debt
        }
    }
}

/// ADS look-sensitivity scale: the FOV ratio (spec), times a player-tunable
/// multiplier. At 6× the same hand movement covers 1/6 the arc.
pub fn ads_sensitivity_scale(fov_deg: f32, ads_multiplier: f32) -> f32 {
    (fov_deg / 60.0) * ads_multiplier
}

#[cfg(test)]
mod sway_tests {
    use super::*;

    #[test]
    fn sway_is_bounded_and_deterministic() {
        for i in 0..2000 {
            let t = i as f32 * 0.02;
            let s = sway_offset(t, 7, SWAY_BASE_RAD);
            assert!(s.length() <= SWAY_BASE_RAD * 1.5, "bounded: {s:?}");
            assert_eq!(s, sway_offset(t, 7, SWAY_BASE_RAD));
        }
        assert_ne!(sway_offset(1.0, 7, 0.002), sway_offset(1.0, 8, 0.002));
    }

    #[test]
    fn holding_breath_damps_then_overshoots() {
        let mut b = Breath::default();
        assert_eq!(b.sway_factor(false), 1.0);
        // Hold for two seconds: damped.
        for _ in 0..120 {
            b.update(1.0 / 60.0, true);
        }
        assert_eq!(b.sway_factor(true), BREATH_DAMP);
        // Hold past capacity: the damp expires even while the key is down.
        for _ in 0..180 {
            b.update(1.0 / 60.0, true);
        }
        assert!(b.sway_factor(true) > 1.4, "exhausted hold overshoots");
        // Release: overshoot decays back toward calm.
        for _ in 0..300 {
            b.update(1.0 / 60.0, false);
        }
        assert!(b.sway_factor(false) < 1.1);
    }

    #[test]
    fn ads_sensitivity_follows_fov_ratio() {
        // 6x optic: 10° FOV → one sixth the hand-to-arc rate.
        let s = ads_sensitivity_scale(10.0, 1.0);
        assert!((s - 10.0 / 60.0).abs() < 1e-6);
        assert_eq!(ads_sensitivity_scale(60.0, 1.0), 1.0);
        assert_eq!(ads_sensitivity_scale(60.0, 0.8), 0.8);
    }
}
