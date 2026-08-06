//! Per-object static thermal descriptions (SDD §2.1, SRS FR-T1).

use da_core::TempF;
use serde::{Deserialize, Serialize};

/// Static thermal description carried by every renderable entity.
///
/// The dynamic side (current display temperature, wetness) lives in
/// [`crate::ThermalState`], owned by [`crate::ThermalSim`].
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ThermalProfile {
    /// Internal temperature a metabolic body holds. `None` = ambient-coupled:
    /// zombies and inert objects have no internal heat source and ride the
    /// ambient curve plus whatever daytime solar heat they stored.
    pub base_temp: Option<TempF>,
    /// `true` for warm bodies that actively hold `base_temp` (pests, dogs,
    /// the player). `false` for zombies and scenery.
    pub metabolic: bool,
    /// Seconds-scale resistance to temperature change: the time constant of
    /// the lerp toward the target temperature. Rock high, grass low.
    pub thermal_mass: f32,
    /// Fraction of the object facing open sky, `0..=1`. Drives radiative
    /// cooling below ambient on clear nights (metal roofs, frosted grass)
    /// and how fast rain wets the surface.
    pub sky_exposure: f32,
    /// Stored daytime heat above ambient at dusk, °F. Sun-baked rock and
    /// metal high, shaded grass low. Decays over the night via
    /// [`crate::solar_decay`].
    pub initial_solar_gain_f: f32,
}

impl ThermalProfile {
    /// Warm-bodied pest (rat, possum, raccoon): holds ~101 °F all night.
    pub fn pest() -> Self {
        Self {
            base_temp: Some(TempF::PEST_BODY),
            metabolic: true,
            thermal_mass: 15.0,
            sky_exposure: 0.15,
            initial_solar_gain_f: 0.0,
        }
    }

    /// Zombie: ambient-coupled, **must equal ambient at all times** — the
    /// invisibility rule (SDD §4.1). No stored solar heat, no sky exposure,
    /// negligible thermal mass so it tracks the ambient curve exactly.
    /// Its invisibility in thermal is emergent, never a renderer special case.
    pub fn zombie() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 1.0,
            sky_exposure: 0.0,
            initial_solar_gain_f: 0.0,
        }
    }

    /// Thin metal roofing: big daytime solar gain, cools fast, fully sky
    /// exposed — reads *below* ambient on clear nights (SDD §7A).
    pub fn metal_roof() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 100.0,
            sky_exposure: 1.0,
            initial_solar_gain_f: 22.0,
        }
    }

    /// Sun-baked rock: large solar store, very slow to cool — the warm blob
    /// that fools new thermal owners at dusk.
    pub fn rock() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 2400.0,
            sky_exposure: 0.7,
            initial_solar_gain_f: 15.0,
        }
    }

    /// Grass/low vegetation: almost no stored heat, cools within minutes,
    /// high sky exposure (frosts first on clear nights).
    pub fn grass() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 60.0,
            sky_exposure: 0.9,
            initial_solar_gain_f: 2.5,
        }
    }

    /// Open water: enormous thermal mass; stays near its dusk temperature
    /// far longer than anything else in the scene.
    pub fn water() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 9000.0,
            sky_exposure: 0.5,
            initial_solar_gain_f: 5.0,
        }
    }

    /// Tree canopy/trunk: moderate mass, mostly self-shading.
    pub fn tree() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 900.0,
            sky_exposure: 0.25,
            initial_solar_gain_f: 4.0,
        }
    }

    /// Masonry/wood building wall: slow-cooling solar store, partly sheltered
    /// by its own eaves.
    pub fn building_wall() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 2500.0,
            sky_exposure: 0.35,
            initial_solar_gain_f: 12.0,
        }
    }

    /// Window glass: light, cools quickly, high sky exposure. (Thermal-opaque
    /// to the optic — the renderer draws the pane's own surface temperature.)
    pub fn glass() -> Self {
        Self {
            base_temp: None,
            metabolic: false,
            thermal_mass: 240.0,
            sky_exposure: 0.85,
            initial_solar_gain_f: 6.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zombie_is_pure_ambient() {
        let z = ThermalProfile::zombie();
        assert!(z.base_temp.is_none());
        assert!(!z.metabolic);
        assert_eq!(z.initial_solar_gain_f, 0.0);
        assert_eq!(z.sky_exposure, 0.0);
    }

    #[test]
    fn pest_holds_body_heat() {
        let p = ThermalProfile::pest();
        assert!(p.metabolic);
        assert_eq!(p.base_temp, Some(TempF(101.0)));
    }

    #[test]
    fn ron_round_trip() {
        let p = ThermalProfile::metal_roof();
        let s = ron::to_string(&p).expect("serialize");
        let back: ThermalProfile = ron::from_str(&s).expect("deserialize");
        assert_eq!(p, back);
    }
}
