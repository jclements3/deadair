use serde::{Deserialize, Serialize};

/// Nightly forecast (SRS FR-WX1, SDD §7A). Shown at camp before committing;
/// each variant reshapes detection physics AND the expected-value calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Forecast {
    Clear,     // clear/cold: best thermal, deepest pre-dawn crossover
    Overcast,  // dim eye/NV, average thermal
    Fog,       // NV poor, thermal slightly reduced, pest activity HIGH
    Rain,      // thermal collapses, pests shelter (LOW activity)
    ColdSnap,  // clear + colder: crisper thermal, faster battery drain
    HeatWave,  // warm ambient: thermal poor all night, NV night
    PreStorm,  // surge night: everything good, activity VERY high
}

/// Multipliers/settings a forecast applies to the night's systems.
/// 1.0 = neutral. These feed the thermal sim, the optics renderer,
/// the spawn director, and the camp forecast panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeatherMods {
    /// Scales object-vs-background contrast in the thermal sim.
    pub thermal_contrast: f32,
    /// How early/deep the crossover bottoms out: added depth to the
    /// late-night contrast collapse (positive = harsher crossover).
    pub crossover_depth: f32,
    /// NV image quality multiplier (fog/rain scatter).
    pub nv_visibility: f32,
    /// Naked-eye visibility multiplier (cloud cover vs. moon).
    pub eye_visibility: f32,
    /// Pest spawn/activity multiplier — the revenue dial.
    pub pest_activity: f32,
    /// Battery drain multiplier (cold increases it).
    pub battery_drain: f32,
    /// Trip-hazard severity multiplier (rain-slick banks).
    pub hazard_severity: f32,
    /// Ambient temperature offset in °F applied to the night curve.
    pub ambient_offset_f: f32,
    /// Rain wetting rate (0 = dry night) — pulls exposed surfaces to ambient.
    pub wetting_rate: f32,
}

impl Forecast {
    pub const ALL: [Forecast; 7] = [
        Forecast::Clear,
        Forecast::Overcast,
        Forecast::Fog,
        Forecast::Rain,
        Forecast::ColdSnap,
        Forecast::HeatWave,
        Forecast::PreStorm,
    ];

    /// Modifier table from SDD §7A / SRS FR-WX2.
    pub fn mods(self) -> WeatherMods {
        use Forecast::*;
        match self {
            Clear => WeatherMods {
                thermal_contrast: 1.15,
                crossover_depth: 0.25, // deep radiative crossover late
                nv_visibility: 1.0,
                eye_visibility: 1.0,
                pest_activity: 1.0,
                battery_drain: 1.2,
                hazard_severity: 1.0,
                ambient_offset_f: -4.0,
                wetting_rate: 0.0,
            },
            Overcast => WeatherMods {
                thermal_contrast: 1.0,
                crossover_depth: 0.0,
                nv_visibility: 0.8,
                eye_visibility: 0.55,
                pest_activity: 1.0,
                battery_drain: 1.0,
                hazard_severity: 1.0,
                ambient_offset_f: 0.0,
                wetting_rate: 0.0,
            },
            Fog => WeatherMods {
                thermal_contrast: 0.9,
                crossover_depth: 0.05,
                nv_visibility: 0.45,
                eye_visibility: 0.4,
                pest_activity: 1.5,
                battery_drain: 1.0,
                hazard_severity: 1.15,
                ambient_offset_f: 0.0,
                wetting_rate: 0.1,
            },
            Rain => WeatherMods {
                thermal_contrast: 0.5,
                crossover_depth: 0.35, // collapses early
                nv_visibility: 0.5,
                eye_visibility: 0.45,
                pest_activity: 0.4, // sheltering
                battery_drain: 1.0,
                hazard_severity: 1.5,
                ambient_offset_f: -2.0,
                wetting_rate: 1.0,
            },
            ColdSnap => WeatherMods {
                thermal_contrast: 1.3,
                crossover_depth: 0.3,
                nv_visibility: 1.0,
                eye_visibility: 1.0,
                pest_activity: 0.85,
                battery_drain: 1.6, // cold murders batteries (FR-O4)
                hazard_severity: 1.1,
                ambient_offset_f: -12.0,
                wetting_rate: 0.0,
            },
            HeatWave => WeatherMods {
                thermal_contrast: 0.55, // warm ground ≈ warm bodies
                crossover_depth: 0.15,
                nv_visibility: 1.0,
                eye_visibility: 1.0,
                pest_activity: 1.3,
                battery_drain: 1.0,
                hazard_severity: 1.0,
                ambient_offset_f: 14.0,
                wetting_rate: 0.0,
            },
            PreStorm => WeatherMods {
                thermal_contrast: 1.05,
                crossover_depth: 0.1,
                nv_visibility: 0.9,
                eye_visibility: 0.7,
                pest_activity: 2.2, // the jackpot night
                battery_drain: 1.0,
                hazard_severity: 1.0,
                ambient_offset_f: 2.0,
                wetting_rate: 0.0,
            },
        }
    }

    /// Camp forecast panel blurb (FR-U6).
    pub fn blurb(self) -> &'static str {
        use Forecast::*;
        match self {
            Clear => "Clear and cool. Excellent thermal early; deep crossover before dawn.",
            Overcast => "Overcast. Dim for the eye and NV; thermal average.",
            Fog => "Fog. NV badly scattered; pests are out in force.",
            Rain => "Rain. Thermal contrast collapses; pests shelter. Consider skipping.",
            ColdSnap => "Cold snap. Crisp thermal, brutal battery drain.",
            HeatWave => "Heat wave. Warm ground hides warm bodies — an NV night.",
            PreStorm => "Pre-storm surge. Activity spikes ahead of the front. Jackpot night.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_collapses_thermal_and_activity() {
        let m = Forecast::Rain.mods();
        assert!(m.thermal_contrast <= 0.5);
        assert!(m.pest_activity < 0.5);
        assert!(m.wetting_rate > 0.9);
    }

    #[test]
    fn heat_wave_is_the_nv_inversion() {
        let m = Forecast::HeatWave.mods();
        // Thermal poor but NV unaffected — keeps NV relevant late-game.
        assert!(m.thermal_contrast < 0.7);
        assert!(m.nv_visibility >= 1.0);
    }

    #[test]
    fn cold_snap_drains_batteries_fastest() {
        let cold = Forecast::ColdSnap.mods().battery_drain;
        for f in Forecast::ALL {
            assert!(f.mods().battery_drain <= cold);
        }
    }

    #[test]
    fn pre_storm_is_peak_activity() {
        let surge = Forecast::PreStorm.mods().pest_activity;
        for f in Forecast::ALL {
            assert!(f.mods().pest_activity <= surge);
        }
    }
}
