//! Thermal preview environment for the editor viewport.
//!
//! The game runs the real 1 Hz [`da_thermal::ThermalSim`] over entities;
//! the editor scrubs night-`t` freely, so it needs an *instant* estimate
//! of what each surface reads at time `t` — no integration history.
//!
//! # Approximation (documented, deliberate)
//!
//! For a node whose effective state carries a
//! [`da_graph::ThermalAttach`], the display temperature is:
//!
//! ```text
//! temp = ambient_at(t, forecast)
//!      + (base_temp - DUSK_AMBIENT_F) * solar_decay(t)   // stored day heat bleeding off
//!      - sky_exposure * 10.0    (Clear / ColdSnap only)  // radiative sky cooling
//! ```
//!
//! `thermal_mass` is ignored here (it shapes the *rate* the real
//! integrator converges at, not the instant snapshot), and there is no
//! metabolic case — pests/zombies aren't in the zone graph. A later pass
//! will run the real `ThermalSim` over NodeIds; this keeps the scrubber
//! honest enough that metal roofs read below ambient on clear nights and
//! everything converges toward ambient at the pre-dawn crossover.

use da_core::Forecast;
use da_graph::ThermalAttach;
use da_thermal::curve::DUSK_AMBIENT_F;
use da_thermal::{ambient_at, solar_decay};

/// °F of radiative sky cooling at full sky exposure on a radiative night.
pub const RADIATIVE_SKY_DROP_F: f32 = 10.0;

/// How far below ambient the sky reads in the thermal optic.
pub const SKY_DELTA_F: f32 = 45.0;

/// One night-time preview environment: `t` (0 = dusk, 1 = dawn) plus the
/// forecast, with the ambient temperature precomputed.
#[derive(Debug, Clone, Copy)]
pub struct PreviewEnv {
    /// Normalized night time, clamped to `[0, 1]`.
    pub t: f32,
    /// The night's forecast.
    pub forecast: Forecast,
    /// Ambient air temperature at `t`, °F.
    pub ambient_f: f32,
}

impl PreviewEnv {
    /// Environment at night-time `t` under `forecast`.
    pub fn new(t: f32, forecast: Forecast) -> Self {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        Self {
            t,
            forecast,
            ambient_f: ambient_at(t, forecast).0,
        }
    }

    /// Sky temperature for the draw list — uniform cold, well below
    /// ambient.
    pub fn sky_temp_f(&self) -> f32 {
        self.ambient_f - SKY_DELTA_F
    }

    /// Moonlight factor for eye/NV, derived from the forecast's
    /// eye-visibility modifier.
    pub fn moonlight(&self) -> f32 {
        (0.45 * self.forecast.mods().eye_visibility).clamp(0.0, 1.0)
    }

    /// True on nights with strong radiative sky cooling (clear skies).
    pub fn radiative_night(&self) -> bool {
        matches!(self.forecast, Forecast::Clear | Forecast::ColdSnap)
    }

    /// Approximate display temperature (°F) for a surface with the given
    /// thermal attachment; surfaces with no attachment read exactly
    /// ambient (and so vanish in the thermal optic). See the module docs
    /// for the approximation.
    pub fn display_temp_f(&self, thermal: Option<&ThermalAttach>) -> f32 {
        let Some(a) = thermal else {
            return self.ambient_f;
        };
        let stored = (a.base_temp.0 - DUSK_AMBIENT_F) * solar_decay(self.t);
        let radiative = if self.radiative_night() {
            a.sky_exposure.clamp(0.0, 1.0) * RADIATIVE_SKY_DROP_F
        } else {
            0.0
        };
        self.ambient_f + stored - radiative
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_core::TempF;

    /// The attach da-param puts on metal roofs (ThermalProfile::metal_roof
    /// through material::attach): base_temp = dusk equilibrium, full sky
    /// exposure.
    fn metal_roof() -> ThermalAttach {
        ThermalAttach {
            base_temp: TempF(DUSK_AMBIENT_F),
            thermal_mass: 100.0,
            sky_exposure: 1.0,
        }
    }

    #[test]
    fn metal_roof_below_ambient_under_clear_not_under_overcast() {
        let roof = metal_roof();

        let clear = PreviewEnv::new(0.5, Forecast::Clear);
        assert!(
            clear.display_temp_f(Some(&roof)) < clear.ambient_f,
            "clear night: radiative cooling pulls the roof below ambient"
        );

        let overcast = PreviewEnv::new(0.5, Forecast::Overcast);
        assert!(
            overcast.display_temp_f(Some(&roof)) >= overcast.ambient_f,
            "overcast: clouds block radiative cooling; roof not below ambient"
        );
    }

    #[test]
    fn no_attachment_reads_exactly_ambient() {
        for &f in &Forecast::ALL {
            for t in [0.0_f32, 0.5, 0.85, 1.0] {
                let env = PreviewEnv::new(t, f);
                assert_eq!(env.display_temp_f(None), env.ambient_f);
            }
        }
    }

    #[test]
    fn stored_day_heat_decays_over_the_night() {
        // Sun-warmed mass starts well above ambient at dusk and converges
        // toward ambient by the crossover.
        let warm = ThermalAttach {
            base_temp: TempF(DUSK_AMBIENT_F + 20.0),
            thermal_mass: 900.0,
            sky_exposure: 0.0,
        };
        let dusk = PreviewEnv::new(0.0, Forecast::Overcast);
        let late = PreviewEnv::new(0.85, Forecast::Overcast);
        let dusk_delta = dusk.display_temp_f(Some(&warm)) - dusk.ambient_f;
        let late_delta = late.display_temp_f(Some(&warm)) - late.ambient_f;
        assert!((dusk_delta - 20.0).abs() < 1e-4, "full store at dusk");
        assert!(late_delta > 0.0 && late_delta < 3.0, "mostly bled off late");
    }
}
