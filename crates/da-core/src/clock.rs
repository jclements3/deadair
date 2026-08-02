use serde::{Deserialize, Serialize};

/// The dusk→dawn session clock (SRS FR-S1..S3).
///
/// Game nights run on an accelerated clock. All simulation systems key off
/// normalized night time `t ∈ [0, 1]` (0 = dusk, 1 = dawn); the HUD converts
/// to a wall-clock countdown. The thermal contrast curve, spawn schedules,
/// and the pre-dawn crossover are all functions of `t`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NightClock {
    /// Simulated night length in in-game hours (dusk to dawn).
    pub night_hours: f32,
    /// Real seconds of play representing the whole night.
    pub real_seconds: f32,
    /// Elapsed real seconds.
    elapsed: f32,
}

/// Normalized night time at which thermal crossover bottoms out (SDD §2.2).
pub const CROSSOVER_T: f32 = 0.85;

impl NightClock {
    /// Default session: a 10-hour night compressed into 40 real minutes.
    pub fn standard() -> Self {
        Self::new(10.0, 40.0 * 60.0)
    }

    pub fn new(night_hours: f32, real_seconds: f32) -> Self {
        Self {
            night_hours,
            real_seconds,
            elapsed: 0.0,
        }
    }

    /// Advance by real dt seconds. Clamps at dawn.
    pub fn tick(&mut self, dt: f32) {
        self.elapsed = (self.elapsed + dt).min(self.real_seconds);
    }

    /// Normalized night time, 0 = dusk, 1 = dawn.
    pub fn t(&self) -> f32 {
        self.elapsed / self.real_seconds
    }

    pub fn is_dawn(&self) -> bool {
        self.elapsed >= self.real_seconds
    }

    /// In-game hours remaining until dawn.
    pub fn hours_to_dawn(&self) -> f32 {
        (1.0 - self.t()) * self.night_hours
    }

    /// HUD countdown, e.g. "02:14" (SDD §8).
    pub fn hud_countdown(&self) -> String {
        let h = self.hours_to_dawn();
        let hh = h.floor() as u32;
        let mm = ((h - h.floor()) * 60.0).floor() as u32;
        format!("{hh:02}:{mm:02}")
    }

    /// Jump directly to a normalized time (editor preview / tests).
    pub fn seek(&mut self, t: f32) {
        self.elapsed = t.clamp(0.0, 1.0) * self.real_seconds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_at_dusk_ends_at_dawn() {
        let mut c = NightClock::new(10.0, 100.0);
        assert_eq!(c.t(), 0.0);
        assert!(!c.is_dawn());
        c.tick(50.0);
        assert!((c.t() - 0.5).abs() < 1e-6);
        c.tick(60.0); // overshoot clamps
        assert_eq!(c.t(), 1.0);
        assert!(c.is_dawn());
    }

    #[test]
    fn hud_countdown_formats() {
        let mut c = NightClock::new(10.0, 100.0);
        c.tick(50.0); // halfway: 5.0 hours left
        assert_eq!(c.hud_countdown(), "05:00");
        c.seek(0.775); // 2.25 h left
        assert_eq!(c.hud_countdown(), "02:15");
    }

    #[test]
    fn seek_clamps() {
        let mut c = NightClock::standard();
        c.seek(1.5);
        assert!(c.is_dawn());
        c.seek(-0.5);
        assert_eq!(c.t(), 0.0);
    }
}
