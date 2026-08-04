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
