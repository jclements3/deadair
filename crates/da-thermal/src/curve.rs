//! Scripted night curves: ambient temperature, stored-solar decay, and the
//! scene contrast metric (SDD §2.2, SRS FR-T2).
//!
//! Everything here is a pure function of normalized night time
//! `t ∈ [0, 1]` (0 = dusk, 1 = dawn) and the night's [`Forecast`].

use da_core::clock::CROSSOVER_T;
use da_core::{Forecast, TempF};

/// Baseline ambient temperature at dusk (before the forecast offset), °F.
pub const DUSK_AMBIENT_F: f32 = 68.0;

/// Baseline pre-dawn minimum ambient (before the forecast offset), °F.
pub const PREDAWN_MIN_F: f32 = 47.0;

/// Exponential rate of the dusk→pre-dawn ambient decay (per crossover span).
pub const AMBIENT_DECAY_K: f32 = 3.0;

/// Slight ambient rebound between the crossover minimum and dawn, °F.
pub const DAWN_REBOUND_F: f32 = 1.5;

/// Exponential rate at which stored daytime solar heat bleeds off over the
/// night (per unit `t`). At the crossover (`t ≈ 0.85`) roughly 8 % remains.
pub const SOLAR_DECAY_K: f32 = 3.0;

/// Late-night floor of the contrast envelope (fraction of dusk contrast that
/// survives outside the crossover notch).
pub const CONTRAST_FLOOR: f32 = 0.25;

/// Exponential rate of the contrast envelope's dusk→dawn decay.
pub const CONTRAST_DECAY_K: f32 = 2.2;

/// Baseline fractional contrast loss at the bottom of the crossover notch;
/// `WeatherMods::crossover_depth` is added on top of this.
pub const BASE_CROSSOVER_DIP: f32 = 0.35;

/// Width (in normalized night time) of the Gaussian crossover notch.
pub const CROSSOVER_WIDTH: f32 = 0.09;

/// Ambient air temperature at normalized night time `t` under `forecast`
/// (SDD §2.2). Dusk-warm exponential decay to a pre-dawn minimum near
/// [`CROSSOVER_T`], then a slight rebound toward dawn. The forecast's
/// `ambient_offset_f` shifts the whole curve. `t` is clamped to `[0, 1]`.
pub fn ambient_at(t: f32, forecast: Forecast) -> TempF {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let range = DUSK_AMBIENT_F - PREDAWN_MIN_F;
    let decayed = |x: f32| PREDAWN_MIN_F + range * (-AMBIENT_DECAY_K * x / CROSSOVER_T).exp();
    let base = if t <= CROSSOVER_T {
        decayed(t)
    } else {
        // Continuous at the crossover; slight pre-dawn rebound after it.
        decayed(CROSSOVER_T) + DAWN_REBOUND_F * (t - CROSSOVER_T) / (1.0 - CROSSOVER_T)
    };
    TempF(base + forecast.mods().ambient_offset_f)
}

/// Fraction of an object's stored daytime solar heat remaining at night time
/// `t` (1.0 at dusk, ~8 % at the crossover). Weather scaling is applied by
/// the caller via `WeatherMods::thermal_contrast`.
pub fn solar_decay(t: f32) -> f32 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    (-SOLAR_DECAY_K * t).exp()
}

/// Normalized expected object-vs-background separation in `[0, 1]`
/// (SDD §2.2, FR-T2): high at dusk, exponentially flattening, minimum at the
/// pre-dawn crossover (`t ≈ 0.85`, deepened by the forecast's
/// `crossover_depth`), with partial recovery toward dawn. Scaled by the
/// forecast's `thermal_contrast`.
///
/// This single curve is the game's difficulty dial.
pub fn contrast(t: f32, forecast: Forecast) -> f32 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let mods = forecast.mods();
    let envelope = CONTRAST_FLOOR + (1.0 - CONTRAST_FLOOR) * (-CONTRAST_DECAY_K * t).exp();
    let x = (t - CROSSOVER_T) / CROSSOVER_WIDTH;
    let notch = ((BASE_CROSSOVER_DIP + mods.crossover_depth.max(0.0)) * (-x * x).exp()).min(0.95);
    (envelope * (1.0 - notch) * mods.thermal_contrast).clamp(0.0, 1.0)
}

/// Detection-range multiplier for HUD and AI use: maps [`contrast`] into
/// `[0.3, 1.0]`. At the crossover under a Clear sky this lands around 0.42 —
/// thermal detection ranges cut 50–70 % (SRS FR-T2).
pub fn detection_range_factor(t: f32, forecast: Forecast) -> f32 {
    (0.3 + 0.7 * contrast(t, forecast)).clamp(0.3, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambient_decays_then_rebounds() {
        let f = Forecast::Overcast; // zero offset
        let dusk = ambient_at(0.0, f);
        let cross = ambient_at(CROSSOVER_T, f);
        let dawn = ambient_at(1.0, f);
        assert!((dusk.0 - DUSK_AMBIENT_F).abs() < 1e-4);
        assert!(cross.0 < dusk.0);
        assert!(cross.0 < PREDAWN_MIN_F + 2.0, "min lands near the floor");
        assert!(dawn.0 > cross.0, "slight rebound after crossover");
        assert!(dawn.0 - cross.0 <= DAWN_REBOUND_F + 1e-4);
    }

    #[test]
    fn ambient_monotone_down_to_crossover() {
        let f = Forecast::Clear;
        let mut prev = ambient_at(0.0, f).0;
        for i in 1..=85 {
            let cur = ambient_at(i as f32 / 100.0, f).0;
            assert!(cur <= prev + 1e-5);
            prev = cur;
        }
    }

    #[test]
    fn ambient_offset_applies() {
        let cold = ambient_at(0.3, Forecast::ColdSnap).0;
        let base = ambient_at(0.3, Forecast::Overcast).0;
        assert!((base - cold - 12.0).abs() < 1e-4);
    }

    #[test]
    fn curves_handle_garbage_input() {
        for f in Forecast::ALL {
            for t in [-5.0, 2.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(ambient_at(t, f).0.is_finite());
                assert!(contrast(t, f).is_finite());
                let d = detection_range_factor(t, f);
                assert!((0.3..=1.0).contains(&d));
            }
        }
    }

    #[test]
    fn contrast_bounded() {
        for f in Forecast::ALL {
            for i in 0..=100 {
                let c = contrast(i as f32 / 100.0, f);
                assert!((0.0..=1.0).contains(&c));
            }
        }
    }
}
