//! Power plants, rifle tiers, pellets — SDD §5.1, SRS §3.1 (FR-W1..W8) and
//! §3.7 (FR-B2).
//!
//! Energy is tracked in FPE (foot-pounds — the unit airgun culture talks in);
//! ballistics converts to SI internally.

use serde::{Deserialize, Serialize};

/// Foot-pounds of energy → joules.
pub const FPE_TO_J: f32 = 1.3558;

/// Power wheel on regulated PCPs (FR-W3). LOW = more shots, short lethal
/// range; HIGH = fewer shots, full lethal range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PowerSetting {
    Low,
    Med,
    High,
}

impl PowerSetting {
    /// Fraction of the rifle's base muzzle energy delivered.
    pub fn energy_scale(self) -> f32 {
        match self {
            PowerSetting::Low => 0.55,
            PowerSetting::Med => 0.80,
            PowerSetting::High => 1.0,
        }
    }

    /// Air consumed per shot relative to a MED shot (FR-W2: proportional
    /// to power).
    pub fn air_scale(self) -> f32 {
        match self {
            PowerSetting::Low => 0.7,
            PowerSetting::Med => 1.0,
            PowerSetting::High => 1.45,
        }
    }
}

/// Pellet caliber. Tiers 1–3 are .22; tier 4 is the premium .25.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Caliber {
    Cal22,
    Cal25,
}

impl Caliber {
    /// Nominal pellet mass in grams for the Standard variant.
    pub fn pellet_mass_g(self) -> f32 {
        match self {
            Caliber::Cal22 => 1.04, // ~16 gr
            Caliber::Cal25 => 1.63, // ~25 gr
        }
    }
}

/// Pellet variants trade trajectory against terminal performance (FR-E3):
/// they adjust mass (hence velocity), the drop constant, and the
/// lethal-range table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PelletVariant {
    /// Baseline domed pellet.
    Standard,
    /// Lighter: faster and flatter early, sheds energy sooner.
    Light,
    /// Heavier: slower, slightly better carry and lethal range.
    Heavy,
}

impl PelletVariant {
    /// Mass multiplier applied to the caliber's nominal pellet mass.
    pub fn mass_scale(self) -> f32 {
        match self {
            PelletVariant::Standard => 1.0,
            PelletVariant::Light => 0.85,
            PelletVariant::Heavy => 1.2,
        }
    }

    /// Multiplier on the parabolic drop constant (BC proxy).
    pub fn drop_scale(self) -> f32 {
        match self {
            PelletVariant::Standard => 1.0,
            PelletVariant::Light => 1.08,
            PelletVariant::Heavy => 0.93,
        }
    }

    /// Multiplier on the lethal-range table.
    pub fn lethal_scale(self) -> f32 {
        match self {
            PelletVariant::Standard => 1.0,
            PelletVariant::Light => 0.9,
            PelletVariant::Heavy => 1.12,
        }
    }
}

/// The rifle ladder (SDD §7.2, FR-B2). Each tier changes how the game
/// plays; tier 4 is the premium .25 with the highest energy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RifleTier {
    /// Tier 1 — multi-pump .22. Cheap, slow, infinite "air".
    T1,
    /// Tier 2 — unregulated PCP .22. Fill-curve skill mechanic.
    T2,
    /// Tier 3 — regulated PCP .22. Flat velocity, moderator mount.
    T3,
    /// Tier 4 — premium PCP .25. Highest energy, License D targets.
    T4,
}

impl RifleTier {
    /// All tiers in ladder order.
    pub const ALL: [RifleTier; 4] = [RifleTier::T1, RifleTier::T2, RifleTier::T3, RifleTier::T4];

    /// Base (full-power) muzzle energy in FPE. For the multi-pump this is
    /// the ceiling at max pumps; actual energy comes from the pump count.
    pub fn base_energy_fpe(self) -> f32 {
        match self {
            RifleTier::T1 => 12.8, // 8 pumps x 1.6
            RifleTier::T2 => 22.0,
            RifleTier::T3 => 26.0,
            RifleTier::T4 => 45.0,
        }
    }

    /// Intrinsic dispersion (milliradians, radius) of the platform.
    pub fn base_dispersion_mrad(self) -> f32 {
        match self {
            RifleTier::T1 => 4.0,
            RifleTier::T2 => 2.6,
            RifleTier::T3 => 1.8,
            RifleTier::T4 => 1.2,
        }
    }

    /// Dispersion multiplier when shooting pellets matched to the rifle
    /// (SDD §5.1: per-tier accuracy bonus).
    pub fn matched_pellet_bonus(self) -> f32 {
        match self {
            RifleTier::T1 => 0.88,
            RifleTier::T2 => 0.85,
            RifleTier::T3 => 0.82,
            RifleTier::T4 => 0.78,
        }
    }

    /// Caliber for the tier.
    pub fn caliber(self) -> Caliber {
        match self {
            RifleTier::T4 => Caliber::Cal25,
            _ => Caliber::Cal22,
        }
    }
}

/// Fill fraction where the unregulated velocity curve peaks.
pub const UNREG_SWEET_SPOT: f32 = 0.55;

/// Power plant models per SDD §5.1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PowerPlant {
    /// Tier 1: pump strokes are the "air". Firing consumes all pumps;
    /// re-pumping is a timed, interruptible action (see
    /// [`RifleConfig::pump`]) that emits movement noise per stroke.
    MultiPump {
        /// Strokes currently in the gun.
        pumps: u8,
        /// Mechanical stroke limit.
        max_pumps: u8,
        /// FPE added per stroke.
        energy_per_pump: f32,
        /// Seconds per stroke.
        pump_time_sec: f32,
        /// Noise radius (m) of each stroke.
        pump_noise_radius_m: f32,
    },
    /// Tier 2: velocity (and accuracy) ride the fill curve — sweet spot
    /// mid-fill, penalties at full and low fill.
    UnregulatedPcp {
        /// Reservoir fill, 0..1 (FR-W1).
        fill_pct: f32,
        /// Shots per full fill.
        capacity_shots: u16,
    },
    /// Tier 3/4: flat velocity while `fill_pct > reg_setpoint`; the power
    /// wheel trades shot count against lethal range (FR-W2/W3).
    RegulatedPcp {
        /// Reservoir fill, 0..1.
        fill_pct: f32,
        /// Shots per full fill at MED power.
        capacity_shots: u16,
        /// Fill fraction below which the regulator can no longer hold
        /// setpoint and velocity droops.
        reg_setpoint: f32,
        /// Selected power setting.
        power: PowerSetting,
    },
}

impl PowerPlant {
    /// The Tier 2 skill mechanic: dispersion multiplier across the
    /// unregulated fill curve. 1.0 at the sweet spot, rising toward both
    /// full and empty.
    pub fn velocity_curve(fill_pct: f32) -> f32 {
        let x = ((fill_pct - UNREG_SWEET_SPOT) / 0.45).abs().min(1.5);
        1.0 + 0.9 * x * x
    }

    /// True if the plant can deliver a shot right now.
    pub fn can_fire(&self) -> bool {
        match self {
            PowerPlant::MultiPump { pumps, .. } => *pumps > 0,
            PowerPlant::UnregulatedPcp { fill_pct, capacity_shots } => {
                *fill_pct >= 1.0 / *capacity_shots as f32
            }
            PowerPlant::RegulatedPcp { fill_pct, capacity_shots, power, .. } => {
                *fill_pct >= power.air_scale() / *capacity_shots as f32
            }
        }
    }

    /// Muzzle energy (FPE) the *next* shot would deliver, or `None` if the
    /// plant cannot fire. Does not consume anything.
    pub fn muzzle_energy_fpe(&self, tier_base_fpe: f32) -> Option<f32> {
        if !self.can_fire() {
            return None;
        }
        Some(match self {
            PowerPlant::MultiPump { pumps, energy_per_pump, .. } => {
                *pumps as f32 * energy_per_pump
            }
            PowerPlant::UnregulatedPcp { fill_pct, .. } => {
                // Velocity droops away from the sweet spot; energy ~ v^2.
                let x = ((*fill_pct - UNREG_SWEET_SPOT) / 0.45).abs().min(1.5);
                let vf = 1.0 - 0.08 * x * x;
                tier_base_fpe * vf * vf
            }
            PowerPlant::RegulatedPcp { fill_pct, reg_setpoint, power, .. } => {
                let base = tier_base_fpe * power.energy_scale();
                if *fill_pct > *reg_setpoint {
                    base // flat while above setpoint
                } else {
                    let droop = (*fill_pct / *reg_setpoint).clamp(0.0, 1.0);
                    base * (0.6 + 0.4 * droop)
                }
            }
        })
    }

    /// Dispersion multiplier from the plant state (1.0 = neutral).
    pub fn dispersion_modifier(&self) -> f32 {
        match self {
            PowerPlant::MultiPump { .. } => 1.0,
            PowerPlant::UnregulatedPcp { fill_pct, .. } => Self::velocity_curve(*fill_pct),
            PowerPlant::RegulatedPcp { fill_pct, reg_setpoint, .. } => {
                if *fill_pct > *reg_setpoint {
                    1.0
                } else {
                    let droop = (*fill_pct / *reg_setpoint).clamp(0.0, 1.0);
                    1.0 + 1.2 * (1.0 - droop)
                }
            }
        }
    }

    /// Reservoir fill 0..1 (multi-pump reports pump fraction) — HUD helper.
    pub fn fill_fraction(&self) -> f32 {
        match self {
            PowerPlant::MultiPump { pumps, max_pumps, .. } => {
                *pumps as f32 / (*max_pumps).max(1) as f32
            }
            PowerPlant::UnregulatedPcp { fill_pct, .. }
            | PowerPlant::RegulatedPcp { fill_pct, .. } => *fill_pct,
        }
    }

    /// Camp refill (FR-W6): tops off air / leaves pumps untouched.
    pub fn refill(&mut self) {
        match self {
            PowerPlant::MultiPump { .. } => {}
            PowerPlant::UnregulatedPcp { fill_pct, .. }
            | PowerPlant::RegulatedPcp { fill_pct, .. } => *fill_pct = 1.0,
        }
    }
}

/// A complete rifle: tier, power plant, ammunition, moderator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RifleConfig {
    /// Platform tier (drives base energy/dispersion/caliber).
    pub tier: RifleTier,
    /// Power plant state.
    pub plant: PowerPlant,
    /// Loaded pellet variant.
    pub pellet: PelletVariant,
    /// Pellets matched to this rifle (accuracy bonus, SDD §5.1).
    pub matched_pellets: bool,
    /// Moderator fitted (FR-W7: noise radius x0.3).
    pub moderator: bool,
    /// Partial-stroke progress for the multi-pump (seconds into the
    /// current stroke). Interrupting pumping keeps completed strokes.
    #[serde(default)]
    pump_progress: f32,
}

impl RifleConfig {
    /// Construct with a plant matching the tier's stock configuration.
    pub fn new(tier: RifleTier, plant: PowerPlant) -> Self {
        Self {
            tier,
            plant,
            pellet: PelletVariant::Standard,
            matched_pellets: false,
            moderator: false,
            pump_progress: 0.0,
        }
    }

    /// Tier 1 multi-pump .22 (starts unpumped).
    pub fn tier1() -> Self {
        Self::new(
            RifleTier::T1,
            PowerPlant::MultiPump {
                pumps: 0,
                max_pumps: 8,
                energy_per_pump: 1.6,
                pump_time_sec: 1.1,
                pump_noise_radius_m: 12.0,
            },
        )
    }

    /// Tier 2 unregulated PCP .22, full fill.
    pub fn tier2() -> Self {
        Self::new(
            RifleTier::T2,
            PowerPlant::UnregulatedPcp { fill_pct: 1.0, capacity_shots: 30 },
        )
    }

    /// Tier 3 regulated PCP .22, full fill, MED power.
    pub fn tier3() -> Self {
        Self::new(
            RifleTier::T3,
            PowerPlant::RegulatedPcp {
                fill_pct: 1.0,
                capacity_shots: 45,
                reg_setpoint: 0.35,
                power: PowerSetting::Med,
            },
        )
    }

    /// Tier 4 premium PCP .25, full fill, HIGH power.
    pub fn tier4() -> Self {
        Self::new(
            RifleTier::T4,
            PowerPlant::RegulatedPcp {
                fill_pct: 1.0,
                capacity_shots: 40,
                reg_setpoint: 0.30,
                power: PowerSetting::High,
            },
        )
    }

    /// Energy the next shot would deliver (FPE), if the rifle can fire.
    pub fn muzzle_energy_fpe(&self) -> Option<f32> {
        self.plant.muzzle_energy_fpe(self.tier.base_energy_fpe())
    }

    /// Loaded pellet mass in grams.
    pub fn pellet_mass_g(&self) -> f32 {
        self.tier.caliber().pellet_mass_g() * self.pellet.mass_scale()
    }

    /// Muzzle velocity of the next shot (m/s), if the rifle can fire.
    pub fn muzzle_velocity_mps(&self) -> Option<f32> {
        self.muzzle_energy_fpe()
            .map(|e| crate::ballistics::muzzle_velocity_mps(e, self.pellet_mass_g()))
    }

    /// Fire: consumes all pumps / air proportional to power (FR-W2),
    /// returning the delivered muzzle energy in FPE.
    pub fn fire(&mut self) -> Option<f32> {
        let energy = self.muzzle_energy_fpe()?;
        match &mut self.plant {
            PowerPlant::MultiPump { pumps, .. } => {
                *pumps = 0; // fire consumes ALL pumps
                self.pump_progress = 0.0;
            }
            PowerPlant::UnregulatedPcp { fill_pct, capacity_shots } => {
                *fill_pct = (*fill_pct - 1.0 / *capacity_shots as f32).max(0.0);
            }
            PowerPlant::RegulatedPcp { fill_pct, capacity_shots, power, .. } => {
                *fill_pct =
                    (*fill_pct - power.air_scale() / *capacity_shots as f32).max(0.0);
            }
        }
        Some(energy)
    }

    /// Advance the (interruptible) pumping action by `dt` seconds and
    /// return the number of strokes completed during this call. No-op for
    /// PCPs or when at max pumps. The caller emits stroke noise.
    pub fn pump(&mut self, dt: f32) -> u8 {
        let PowerPlant::MultiPump { pumps, max_pumps, pump_time_sec, .. } = &mut self.plant
        else {
            return 0;
        };
        if *pumps >= *max_pumps {
            return 0;
        }
        self.pump_progress += dt.max(0.0);
        let mut strokes = 0u8;
        while self.pump_progress >= *pump_time_sec && *pumps < *max_pumps {
            self.pump_progress -= *pump_time_sec;
            *pumps += 1;
            strokes += 1;
        }
        if *pumps >= *max_pumps {
            self.pump_progress = 0.0;
        }
        strokes
    }

    /// Effective dispersion (mrad, radius): tier baseline x fill-curve
    /// modifier x matched-pellet bonus.
    pub fn dispersion_mrad(&self) -> f32 {
        let matched = if self.matched_pellets {
            self.tier.matched_pellet_bonus()
        } else {
            1.0
        };
        self.tier.base_dispersion_mrad() * self.plant.dispersion_modifier() * matched
    }

    /// Dispersion radius (m) at a given range.
    pub fn dispersion_radius_m(&self, range_m: f32) -> f32 {
        self.dispersion_mrad() / 1000.0 * range_m
    }

    /// Lethal range (m) of the next shot; 0 if the rifle cannot fire.
    pub fn lethal_range_m(&self) -> f32 {
        self.muzzle_energy_fpe()
            .map(|e| crate::ballistics::lethal_range_m(e, self.pellet.lethal_scale()))
            .unwrap_or(0.0)
    }

    /// Whether a hit at `range_m` is lethal for this configuration.
    pub fn lethal_at(&self, range_m: f32) -> bool {
        range_m <= self.lethal_range_m()
    }

    /// Parabolic drop (m) at `range_m` for the next shot.
    pub fn drop_at(&self, range_m: f32) -> Option<f32> {
        self.muzzle_velocity_mps()
            .map(|v| crate::ballistics::drop_at(range_m, v, self.pellet.drop_scale()))
    }

    /// Holdover solution at `range_m` for the next shot.
    pub fn aim_solution(&self, range_m: f32) -> Option<crate::ballistics::AimSolution> {
        self.muzzle_velocity_mps()
            .map(|v| crate::ballistics::aim_solution(range_m, v, self.pellet.drop_scale()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_pump_fires_only_pumped_and_consumes_all() {
        let mut r = RifleConfig::tier1();
        assert!(!r.plant.can_fire());
        assert_eq!(r.fire(), None); // 0 pumps -> no shot

        // 3.4 s at 1.1 s/stroke -> 3 strokes, partial progress kept.
        assert_eq!(r.pump(3.4), 3);
        let e = r.fire().expect("pumped rifle fires");
        assert!((e - 3.0 * 1.6).abs() < 1e-4);
        // Fire consumed ALL pumps (and reset partial progress).
        assert!(!r.plant.can_fire());
        assert_eq!(r.fire(), None);
    }

    #[test]
    fn multi_pump_caps_at_max_and_is_interruptible() {
        let mut r = RifleConfig::tier1();
        // Interrupt after 2 strokes; completed strokes persist.
        assert_eq!(r.pump(2.2), 2);
        assert_eq!(r.pump(100.0), 6); // caps at 8 total
        assert_eq!(r.pump(5.0), 0);
        let e = r.fire().unwrap();
        assert!((e - 12.8).abs() < 1e-3);
    }

    #[test]
    fn unregulated_accuracy_best_mid_fill_worse_at_extremes() {
        let sweet = PowerPlant::velocity_curve(UNREG_SWEET_SPOT);
        let full = PowerPlant::velocity_curve(1.0);
        let low = PowerPlant::velocity_curve(0.1);
        assert!(sweet < full, "sweet spot must beat full fill");
        assert!(sweet < low, "sweet spot must beat low fill");
        assert!((sweet - 1.0).abs() < 1e-6);

        // Through the rifle: dispersion follows the curve.
        let mut r = RifleConfig::tier2();
        let at = |r: &mut RifleConfig, f: f32| {
            if let PowerPlant::UnregulatedPcp { fill_pct, .. } = &mut r.plant {
                *fill_pct = f;
            }
            r.dispersion_mrad()
        };
        let d_sweet = at(&mut r, UNREG_SWEET_SPOT);
        assert!(d_sweet < at(&mut r, 1.0));
        assert!(d_sweet < at(&mut r, 0.1));
    }

    #[test]
    fn regulated_flat_above_setpoint_then_degrades() {
        let mut r = RifleConfig::tier3();
        let set = |r: &mut RifleConfig, f: f32| {
            if let PowerPlant::RegulatedPcp { fill_pct, .. } = &mut r.plant {
                *fill_pct = f;
            }
        };
        set(&mut r, 1.0);
        let e_full = r.muzzle_energy_fpe().unwrap();
        let d_full = r.dispersion_mrad();
        set(&mut r, 0.40); // still above 0.35 setpoint
        assert_eq!(r.muzzle_energy_fpe().unwrap(), e_full, "flat energy above setpoint");
        assert_eq!(r.dispersion_mrad(), d_full, "flat accuracy above setpoint");
        set(&mut r, 0.20); // below setpoint
        assert!(r.muzzle_energy_fpe().unwrap() < e_full, "energy droops off the reg");
        assert!(r.dispersion_mrad() > d_full, "accuracy degrades off the reg");
    }

    #[test]
    fn air_consumption_proportional_to_power() {
        let fill_after = |power: PowerSetting| {
            let mut r = RifleConfig::tier3();
            if let PowerPlant::RegulatedPcp { power: p, .. } = &mut r.plant {
                *p = power;
            }
            r.fire().unwrap();
            r.plant.fill_fraction()
        };
        let low = fill_after(PowerSetting::Low);
        let med = fill_after(PowerSetting::Med);
        let high = fill_after(PowerSetting::High);
        assert!(high < med && med < low, "FR-W2: HIGH burns more air than LOW");
    }

    #[test]
    fn tier4_is_the_energy_king() {
        for t in RifleTier::ALL {
            assert!(t.base_energy_fpe() <= RifleTier::T4.base_energy_fpe());
        }
        assert_eq!(RifleTier::T4.caliber(), Caliber::Cal25);
        assert_eq!(RifleTier::T3.caliber(), Caliber::Cal22);
    }

    #[test]
    fn rifle_config_ron_round_trip() {
        let mut r = RifleConfig::tier3();
        r.moderator = true;
        r.matched_pellets = true;
        r.pellet = PelletVariant::Heavy;
        let text = ron::to_string(&r).unwrap();
        let back: RifleConfig = ron::from_str(&text).unwrap();
        assert_eq!(r, back);
    }
}
