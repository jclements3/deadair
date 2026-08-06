//! The per-object thermal integrator (SDD §2.1): a coarse 1 Hz simulation
//! that evolves every registered object's display temperature across the
//! night, plus residual heat event bookkeeping (SDD §2.3).

use std::collections::HashMap;

use da_core::{Forecast, NodeId, TempF};

use crate::curve::{ambient_at, solar_decay};
use crate::heat::HeatEvent;
use crate::profile::ThermalProfile;

/// Internal simulation tick, seconds (SDD §2.1: 1 Hz is sufficient).
pub const TICK_DT: f32 = 1.0;

/// Full-sky radiative cooling depth on clear nights, °F below ambient at
/// `sky_exposure = 1.0` once the ramp completes.
pub const RADIATIVE_COEFF_F: f32 = 10.0;

/// Normalized night time over which radiative cooling ramps to full effect
/// (dusk surfaces are still shedding solar heat, not yet sky-cooled).
pub const RADIATIVE_RAMP_T: f32 = 0.3;

/// Wetness gained per second at `wetting_rate = 1.0` and full sky exposure
/// (fully soaked in ~2 minutes of hard rain).
pub const WETTING_PER_SEC: f32 = 1.0 / 120.0;

/// Fraction-per-second pull of a fully wet, non-metabolic surface toward
/// ambient — the rain contrast collapse (FR-T3).
pub const WETNESS_PULL_RATE: f32 = 0.05;

/// Maximum °F a fully wet metabolic body's surface reads below its base
/// temperature (fur/skin evaporative chill).
pub const WET_METABOLIC_PULL_F: f32 = 4.0;

/// Largest single `step` dt honored, seconds (guards runaway catch-up loops).
const MAX_STEP_DT: f32 = 4.0 * 3600.0;

/// Dynamic thermal state of one registered object.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ThermalState {
    /// Temperature the thermal optic renders for this object.
    pub display_temp: TempF,
    /// Rain accumulation, `0..=1`; pulls the surface toward ambient fast.
    pub wetness: f32,
}

#[derive(Debug, Clone)]
struct Slot {
    profile: ThermalProfile,
    prev: ThermalState,
    cur: ThermalState,
}

/// The night's thermal simulation. Owns a [`ThermalState`] per registered
/// object (keyed by [`NodeId`]) and the live residual [`HeatEvent`]s.
///
/// Call [`ThermalSim::step`] every frame with the frame dt and the current
/// normalized night time; internally the sim accumulates time and updates at
/// 1 Hz. For frame-rate rendering, interpolate with
/// [`ThermalSim::sampled_temp`].
#[derive(Debug, Clone)]
pub struct ThermalSim {
    forecast: Forecast,
    slots: HashMap<NodeId, Slot>,
    heat: Vec<HeatEvent>,
    accum: f32,
}

impl ThermalSim {
    /// New simulation for a night under `forecast`.
    pub fn new(forecast: Forecast) -> Self {
        Self {
            forecast,
            slots: HashMap::new(),
            heat: Vec::new(),
            accum: 0.0,
        }
    }

    /// The forecast this night runs under.
    pub fn forecast(&self) -> Forecast {
        self.forecast
    }

    /// Change the forecast mid-night (weather fronts, debug).
    pub fn set_forecast(&mut self, forecast: Forecast) {
        self.forecast = forecast;
    }

    /// Register (or replace) an object. `t` is the normalized night time at
    /// registration; the initial display temperature is the object's natural
    /// temperature at that moment (metabolic bodies at their base temp,
    /// scenery at ambient plus remaining stored solar heat).
    pub fn register(&mut self, id: NodeId, profile: ThermalProfile, t: f32) {
        let ambient = ambient_at(t, self.forecast);
        let display = if profile.metabolic {
            profile.base_temp.unwrap_or(ambient)
        } else {
            let solar = profile.initial_solar_gain_f
                * solar_decay(t)
                * self.forecast.mods().thermal_contrast;
            TempF(ambient.0 + solar)
        };
        let state = ThermalState {
            display_temp: display,
            wetness: 0.0,
        };
        self.slots.insert(
            id,
            Slot {
                profile,
                prev: state,
                cur: state,
            },
        );
    }

    /// Remove an object from the simulation.
    pub fn unregister(&mut self, id: NodeId) {
        self.slots.remove(&id);
    }

    /// Whether `id` is registered.
    pub fn contains(&self, id: NodeId) -> bool {
        self.slots.contains_key(&id)
    }

    /// Number of registered objects.
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    /// True when no objects are registered.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Advance the simulation. `dt` is real seconds since the last call; `t`
    /// is the current normalized night time (from [`da_core::NightClock`]).
    /// Time accumulates internally and whole 1 s ticks are flushed, so this
    /// is safe to call at any frame rate. Non-finite or non-positive `dt` is
    /// ignored.
    pub fn step(&mut self, dt: f32, t: f32) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        self.accum += dt.min(MAX_STEP_DT);
        while self.accum >= TICK_DT {
            self.accum -= TICK_DT;
            self.tick(TICK_DT, t);
        }
    }

    /// Fraction `0..1` of the way from the last flushed tick to the next —
    /// pass as `alpha` to [`ThermalSim::sampled_temp`] for smooth rendering.
    pub fn tick_alpha(&self) -> f32 {
        (self.accum / TICK_DT).clamp(0.0, 1.0)
    }

    /// One 1 Hz update: every object lerps toward its target temperature
    /// (SDD §2.1) and heat events decay.
    fn tick(&mut self, dt: f32, t: f32) {
        let mods = self.forecast.mods();
        let ambient = ambient_at(t, self.forecast);
        // Radiative sky cooling only under clear skies (Clear / ColdSnap):
        // this is what drags metal roofs below ambient (SDD §7A).
        let radiative_sky = matches!(self.forecast, Forecast::Clear | Forecast::ColdSnap);
        let radiative_ramp = (t / RADIATIVE_RAMP_T).clamp(0.0, 1.0);
        let solar = solar_decay(t) * mods.thermal_contrast;

        for slot in self.slots.values_mut() {
            slot.prev = slot.cur;
            let p = &slot.profile;
            let exposure = p.sky_exposure.clamp(0.0, 1.0);

            // Rain accumulation (FR-T3): sheltered surfaces wet slower.
            if mods.wetting_rate > 0.0 {
                slot.cur.wetness = (slot.cur.wetness
                    + mods.wetting_rate * WETTING_PER_SEC * exposure * dt)
                    .clamp(0.0, 1.0);
            }

            let target = if p.metabolic {
                // Warm body holds its base temp; wet fur reads slightly cool.
                let base = p.base_temp.unwrap_or(ambient);
                TempF(base.0 - slot.cur.wetness * WET_METABOLIC_PULL_F)
            } else {
                // Ambient-coupled: ambient + remaining stored solar heat,
                // minus radiative loss to a clear sky.
                let mut temp = ambient.0 + p.initial_solar_gain_f * solar;
                if radiative_sky {
                    temp -= exposure * RADIATIVE_COEFF_F * radiative_ramp;
                }
                TempF(temp)
            };

            let k = (dt / p.thermal_mass.max(1e-3)).clamp(0.0, 1.0);
            slot.cur.display_temp = slot.cur.display_temp.lerp(target, k);

            // Wet surfaces collapse toward ambient regardless of mass.
            if !p.metabolic && slot.cur.wetness > 0.0 {
                let pull = (slot.cur.wetness * WETNESS_PULL_RATE * dt).clamp(0.0, 1.0);
                slot.cur.display_temp = slot.cur.display_temp.lerp(ambient, pull);
            }
        }

        for e in &mut self.heat {
            e.decay(dt);
        }
        self.heat.retain(|e| !e.is_dead());
    }

    /// Current (last-tick) display temperature of `id`.
    pub fn display_temp(&self, id: NodeId) -> Option<TempF> {
        self.slots.get(&id).map(|s| s.cur.display_temp)
    }

    /// Full dynamic state of `id`.
    pub fn state(&self, id: NodeId) -> Option<&ThermalState> {
        self.slots.get(&id).map(|s| &s.cur)
    }

    /// The static profile `id` was registered with.
    pub fn profile(&self, id: NodeId) -> Option<&ThermalProfile> {
        self.slots.get(&id).map(|s| &s.profile)
    }

    /// Display temperature interpolated between the previous and current
    /// 1 Hz ticks (`alpha` in `0..=1`, typically [`ThermalSim::tick_alpha`])
    /// so frame-rate rendering never sees the 1 Hz stairstep.
    pub fn sampled_temp(&self, id: NodeId, alpha: f32) -> Option<TempF> {
        let a = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
        self.slots
            .get(&id)
            .map(|s| s.prev.display_temp.lerp(s.cur.display_temp, a))
    }

    /// Spawn a residual heat event (bedding, footfall, barrel, pellet).
    pub fn spawn_heat(&mut self, event: HeatEvent) {
        if !event.is_dead() {
            self.heat.push(event);
        }
    }

    /// All heat events still visible to the thermal optic, with their
    /// current intensities.
    pub fn live_heat(&self) -> impl Iterator<Item = &HeatEvent> {
        self.heat.iter().filter(|e| e.is_visible())
    }

    /// Every event still simulated (including faded-below-visible ones not
    /// yet culled).
    pub fn heat_events(&self) -> &[HeatEvent] {
        &self.heat
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn register_and_query() {
        let mut sim = ThermalSim::new(Forecast::Overcast);
        let id = NodeId(7);
        assert!(sim.is_empty());
        sim.register(id, ThermalProfile::pest(), 0.0);
        assert!(sim.contains(id));
        assert_eq!(sim.len(), 1);
        assert_eq!(sim.display_temp(id), Some(TempF(101.0)));
        assert!(sim.profile(id).is_some());
        sim.unregister(id);
        assert!(!sim.contains(id));
        assert!(sim.display_temp(id).is_none());
    }

    #[test]
    fn step_accumulates_sub_second_dt() {
        let mut sim = ThermalSim::new(Forecast::Clear);
        let id = NodeId(1);
        sim.register(id, ThermalProfile::grass(), 0.0);
        let start = sim.display_temp(id).unwrap();
        // 0.25 s frames: no tick until a full second accumulates.
        sim.step(0.25, 0.0);
        sim.step(0.25, 0.0);
        sim.step(0.25, 0.0);
        assert_eq!(sim.display_temp(id), Some(start));
        assert!(sim.tick_alpha() > 0.7);
        sim.step(0.25, 0.001);
        assert!(sim.tick_alpha() < 0.1);
    }

    #[test]
    fn bad_dt_is_ignored() {
        let mut sim = ThermalSim::new(Forecast::Clear);
        let id = NodeId(1);
        sim.register(id, ThermalProfile::grass(), 0.0);
        let start = sim.display_temp(id).unwrap();
        sim.step(f32::NAN, 0.5);
        sim.step(-3.0, 0.5);
        sim.step(0.0, 0.5);
        assert_eq!(sim.display_temp(id), Some(start));
    }

    #[test]
    fn sampled_temp_interpolates_between_ticks() {
        let mut sim = ThermalSim::new(Forecast::Overcast);
        let id = NodeId(2);
        // Grass registered artificially hot so one tick moves it visibly.
        let mut p = ThermalProfile::grass();
        p.thermal_mass = 2.0; // one 1 s tick moves halfway to target
        sim.register(id, p, 0.0);
        let before = sim.display_temp(id).unwrap();
        sim.step(1.0, 0.4); // jump t so target is well below current display
        let after = sim.display_temp(id).unwrap();
        assert!(after.0 < before.0);
        let s0 = sim.sampled_temp(id, 0.0).unwrap();
        let s1 = sim.sampled_temp(id, 1.0).unwrap();
        let mid = sim.sampled_temp(id, 0.5).unwrap();
        assert_eq!(s0, before);
        assert_eq!(s1, after);
        let expect = (before.0 + after.0) * 0.5;
        assert!((mid.0 - expect).abs() < 1e-4);
    }

    #[test]
    fn heat_events_tick_and_cull() {
        let mut sim = ThermalSim::new(Forecast::Clear);
        sim.spawn_heat(HeatEvent::pellet_impact(Vec3::ZERO));
        assert_eq!(sim.live_heat().count(), 1);
        for s in 0..60 {
            sim.step(1.0, s as f32 / 2400.0);
        }
        assert_eq!(sim.live_heat().count(), 0);
        assert!(sim.heat_events().is_empty(), "dead events culled");
    }
}
