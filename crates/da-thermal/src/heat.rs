//! Residual heat events (SDD §2.3, SRS FR-T4): decaying warm decals rendered
//! in thermal only. They double as a tracking mechanic.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// ΔT (°F above ambient) below which a heat decal is no longer visible in
/// the thermal optic.
pub const HEAT_VISIBLE_F: f32 = 0.75;

/// ΔT below which an event is culled from the simulation entirely.
pub const HEAT_CULL_F: f32 = 0.25;

/// A localized patch of residual warmth: bedding spot, fresh track, fired
/// barrel, spent pellet. Intensity decays exponentially each tick.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HeatEvent {
    /// World position of the decal, meters.
    pub pos: Vec3,
    /// Current intensity: °F above local ambient.
    pub intensity_f: f32,
    /// Exponential decay rate, 1/s (`intensity *= exp(-rate * dt)`).
    pub decay_rate: f32,
}

impl HeatEvent {
    /// Event with explicit intensity and decay rate.
    pub fn new(pos: Vec3, intensity_f: f32, decay_rate: f32) -> Self {
        Self {
            pos,
            intensity_f: intensity_f.max(0.0),
            decay_rate: decay_rate.max(0.0),
        }
    }

    /// Event tuned so it stays visible for roughly `visible_secs`.
    fn lasting(pos: Vec3, intensity_f: f32, visible_secs: f32) -> Self {
        let rate = (intensity_f / HEAT_VISIBLE_F).max(1.0).ln() / visible_secs.max(1e-3);
        Self::new(pos, intensity_f, rate)
    }

    /// Bedding spot left by an animal that rested >30 s: modest warmth,
    /// readable for about two minutes.
    pub fn bedding(pos: Vec3) -> Self {
        Self::lasting(pos, 12.0, 120.0)
    }

    /// Single footfall on cold ground: faint and brief (~12 s).
    pub fn footfall(pos: Vec3) -> Self {
        Self::lasting(pos, 3.0, 12.0)
    }

    /// Fired rifle barrel: hot, visible ~90 s (SDD §2.3).
    pub fn barrel(pos: Vec3) -> Self {
        Self::lasting(pos, 90.0, 90.0)
    }

    /// Pellet impact: sharp but tiny, visible ~5 s.
    pub fn pellet_impact(pos: Vec3) -> Self {
        Self::lasting(pos, 25.0, 5.0)
    }

    /// Advance the decay by `dt` seconds.
    pub fn decay(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.intensity_f *= (-self.decay_rate * dt).exp();
        }
    }

    /// Whether the decal still reads in the thermal optic.
    pub fn is_visible(&self) -> bool {
        self.intensity_f >= HEAT_VISIBLE_F
    }

    /// Whether the event has decayed far enough to be culled.
    pub fn is_dead(&self) -> bool {
        !(self.intensity_f >= HEAT_CULL_F) // also culls NaN
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visible_duration(mut e: HeatEvent) -> u32 {
        let mut secs = 0;
        while e.is_visible() && secs < 10_000 {
            e.decay(1.0);
            secs += 1;
        }
        secs
    }

    #[test]
    fn tuned_durations() {
        let p = Vec3::ZERO;
        let barrel = visible_duration(HeatEvent::barrel(p));
        assert!((80..=100).contains(&barrel), "barrel {barrel}s");
        let pellet = visible_duration(HeatEvent::pellet_impact(p));
        assert!((4..=8).contains(&pellet), "pellet {pellet}s");
        let bedding = visible_duration(HeatEvent::bedding(p));
        assert!((100..=140).contains(&bedding), "bedding {bedding}s");
        let foot = visible_duration(HeatEvent::footfall(p));
        assert!((8..=16).contains(&foot), "footfall {foot}s");
    }

    #[test]
    fn dead_after_visibility_ends() {
        let mut e = HeatEvent::pellet_impact(Vec3::ZERO);
        for _ in 0..60 {
            e.decay(1.0);
        }
        assert!(!e.is_visible());
        assert!(e.is_dead());
    }
}
