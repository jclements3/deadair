//! Thermal preview for the editor viewport.
//!
//! [`ThermalPreview`] runs the **real** 1 Hz [`da_thermal::ThermalSim`] —
//! the same integrator the game uses — over the zone's thermal-attached
//! nodes, so the viewport shows true integration history (thermal mass,
//! radiative sky cooling, the pre-dawn crossover) rather than a snapshot
//! formula.
//!
//! # Scrubbing
//!
//! The night-`t` slider is free-running, but the sim only moves forward.
//! [`ThermalPreview::seek`] therefore:
//!
//! - **forward** — advances incrementally from the cached `last_t`, in
//!   chunks of [`SEEK_CHUNK_SEC`] so the ambient curve is re-evaluated as
//!   it goes;
//! - **backward, or on a forecast/scene change** — re-registers everything
//!   at `t = 0` and re-runs from dusk.
//!
//! A whole night is [`NIGHT_REAL_SECONDS`] of play (da-core's standard
//! 10-hour night compressed into 40 real minutes), i.e. ~2400 ticks — a
//! few milliseconds. [`MAX_SEEK_TICKS`] caps the work anyway: past that
//! budget the sim steps in coarser-than-1 s slices, trading a little
//! integration accuracy for a responsive slider.
//!
//! # Fallback
//!
//! [`PreviewEnv::display_temp_f`] keeps the old closed-form estimate for
//! nodes that are *not* registered in the sim (and for tests that want a
//! snapshot with no history):
//!
//! ```text
//! temp = ambient_at(t, forecast)
//!      + (base_temp - DUSK_AMBIENT_F) * solar_decay(t)   // stored day heat bleeding off
//!      - sky_exposure * 10.0    (Clear / ColdSnap only)  // radiative sky cooling
//! ```

use da_core::{Forecast, NodeId, TempF};
use da_graph::{Scene, ThermalAttach};
use da_thermal::curve::DUSK_AMBIENT_F;
use da_thermal::{ambient_at, solar_decay, ThermalProfile, ThermalSim};

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

/// Anything that can answer "what does this node read in the thermal
/// optic?" — the closed-form [`PreviewEnv`] or the real
/// [`ThermalPreview`].
pub trait TempSource {
    /// Display temperature (°F) of `node`, whose effective state carries
    /// `thermal` (or `None` for an ambient-coupled surface).
    fn temp_f(&self, node: NodeId, thermal: Option<&ThermalAttach>) -> f32;
}

impl TempSource for PreviewEnv {
    fn temp_f(&self, _node: NodeId, thermal: Option<&ThermalAttach>) -> f32 {
        self.display_temp_f(thermal)
    }
}

/// Real seconds of play that represent one whole night
/// (`da_core::NightClock::standard`): the sim's `dt` is real seconds, so
/// the editor's `t` slider maps onto this span, exactly like a session.
pub const NIGHT_REAL_SECONDS: f32 = 40.0 * 60.0;

/// Seek granularity: the ambient curve is re-sampled every this many
/// simulated seconds while catching up.
pub const SEEK_CHUNK_SEC: f32 = 30.0;

/// Upper bound on simulated seconds (= 1 Hz ticks) integrated by one
/// [`ThermalPreview::seek`] call. A full night is ~2400 ticks, well inside
/// this budget; the cap exists so that a caller who stretches
/// [`NIGHT_REAL_SECONDS`] gets a truncated catch-up rather than a stalled
/// UI.
pub const MAX_SEEK_TICKS: f32 = 20_000.0;

/// Recover a full [`ThermalProfile`] from the graph's slimmer
/// [`ThermalAttach`] by matching the nearest da-thermal preset on
/// (thermal_mass, sky_exposure) — the attach carries no solar-gain figure,
/// the preset supplies it. Mirrors `darkair::convert::profile_from_attach`
/// so the editor and the game agree.
pub fn profile_from_attach(attach: &ThermalAttach) -> ThermalProfile {
    let presets = [
        ThermalProfile::metal_roof(),
        ThermalProfile::rock(),
        ThermalProfile::grass(),
        ThermalProfile::water(),
        ThermalProfile::tree(),
        ThermalProfile::building_wall(),
        ThermalProfile::glass(),
    ];
    let dist = |p: &ThermalProfile| {
        let dm = (p.thermal_mass.ln() - attach.thermal_mass.max(0.1).ln()).abs();
        let ds = (p.sky_exposure - attach.sky_exposure).abs() * 3.0;
        dm + ds
    };
    let mut best = presets[0];
    let mut best_d = dist(&best);
    for p in &presets[1..] {
        let d = dist(p);
        if d < best_d {
            best_d = d;
            best = *p;
        }
    }
    // Keep the attach's authored numbers; the preset only contributes the
    // solar-gain estimate.
    ThermalProfile {
        thermal_mass: attach.thermal_mass,
        sky_exposure: attach.sky_exposure,
        ..best
    }
}

/// The editor's thermal state: the environment at the scrubbed `t` plus a
/// live [`ThermalSim`] holding every thermal-attached node in the zone.
pub struct ThermalPreview {
    env: PreviewEnv,
    sim: ThermalSim,
    /// What was registered, so a backward scrub can rebuild from dusk.
    registered: Vec<(NodeId, ThermalProfile)>,
    /// Night time the sim has been integrated up to.
    last_t: f32,
}

impl ThermalPreview {
    /// An empty preview at dusk under `forecast`.
    pub fn new(forecast: Forecast) -> Self {
        Self {
            env: PreviewEnv::new(0.0, forecast),
            sim: ThermalSim::new(forecast),
            registered: Vec::new(),
            last_t: 0.0,
        }
    }

    /// The environment (ambient, sky, moonlight) at the current `t`.
    pub fn env(&self) -> &PreviewEnv {
        &self.env
    }

    /// Ambient air temperature at the current `t`, °F.
    pub fn ambient_f(&self) -> f32 {
        self.env.ambient_f
    }

    /// Number of nodes under simulation.
    pub fn len(&self) -> usize {
        self.registered.len()
    }

    /// True when no node in the zone carries a thermal attach.
    pub fn is_empty(&self) -> bool {
        self.registered.is_empty()
    }

    /// Register every node whose *effective* state carries a
    /// [`ThermalAttach`], replacing any previous registration, and rewind
    /// the sim to dusk. Call after loading or re-expanding a zone.
    pub fn set_scene(&mut self, scene: &Scene) {
        self.registered.clear();
        for node in scene.nodes() {
            if let Some(attach) = scene.effective_state(node.id()).thermal {
                self.registered
                    .push((node.id(), profile_from_attach(&attach)));
            }
        }
        self.rewind();
    }

    /// Register one node with an explicit profile (metabolic bodies —
    /// pests, the player — that the zone graph has no attach for).
    pub fn register_profile(&mut self, node: NodeId, profile: ThermalProfile) {
        self.registered.retain(|(id, _)| *id != node);
        self.registered.push((node, profile));
        self.sim.register(node, profile, self.last_t);
    }

    /// Rebuild the sim at dusk from the current registration list.
    fn rewind(&mut self) {
        self.sim = ThermalSim::new(self.env.forecast);
        for (id, profile) in &self.registered {
            self.sim.register(*id, *profile, 0.0);
        }
        self.last_t = 0.0;
    }

    /// Scrub to night time `t` under `forecast`, running the real sim.
    /// Forward scrubs are incremental; backward scrubs (and any forecast
    /// change) re-run from dusk. See the module docs.
    pub fn seek(&mut self, t: f32, forecast: Forecast) {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        if forecast != self.env.forecast {
            self.env = PreviewEnv::new(0.0, forecast);
            self.rewind();
        }
        if t < self.last_t {
            self.rewind();
        }
        let span = ((t - self.last_t) * NIGHT_REAL_SECONDS).min(MAX_SEEK_TICKS);
        if span > 0.0 {
            let chunk = SEEK_CHUNK_SEC;
            let mut done = 0.0_f32;
            while done < span {
                let step = chunk.min(span - done);
                done += step;
                // Evaluate the environment at the end of the slice.
                let t_now = self.last_t + (done / NIGHT_REAL_SECONDS);
                self.sim.step(step, t_now.clamp(0.0, 1.0));
            }
            self.last_t = t;
        }
        self.env = PreviewEnv::new(t, forecast);
    }

    /// Display temperature of `node` from the sim, interpolated across the
    /// current tick. Unregistered nodes fall back to the closed-form
    /// [`PreviewEnv::display_temp_f`] estimate.
    pub fn display_temp_f(&self, node: NodeId, thermal: Option<&ThermalAttach>) -> f32 {
        match self.sim.sampled_temp(node, self.sim.tick_alpha()) {
            Some(TempF(f)) => f,
            None => self.env.display_temp_f(thermal),
        }
    }

    /// Minimum and maximum display temperature over the simulated nodes,
    /// for the viewport readout. `None` when nothing is registered.
    pub fn min_max_f(&self) -> Option<(f32, f32)> {
        let mut range: Option<(f32, f32)> = None;
        for (id, _) in &self.registered {
            let Some(TempF(v)) = self.sim.display_temp(*id) else {
                continue;
            };
            range = Some(match range {
                Some((lo, hi)) => (lo.min(v), hi.max(v)),
                None => (v, v),
            });
        }
        range
    }
}

impl TempSource for ThermalPreview {
    fn temp_f(&self, node: NodeId, thermal: Option<&ThermalAttach>) -> f32 {
        self.display_temp_f(node, thermal)
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

    // ------------------------------------------------------------------
    // Real ThermalSim preview
    // ------------------------------------------------------------------

    use da_graph::{Drawable, Scene, Shape, StateSet};
    use da_thermal::ThermalProfile;

    /// A scene with one metal-roof-attached geode.
    fn roof_scene() -> (Scene, da_core::NodeId) {
        let mut scene = Scene::new();
        let geode = scene.add_geode(scene.root()).expect("geode");
        scene
            .set_state(geode, StateSet::new().with_thermal(metal_roof()))
            .expect("state");
        scene
            .add_drawable(geode, Drawable::new(Shape::Sphere { radius: 1.0 }))
            .expect("drawable");
        (scene, geode)
    }

    #[test]
    fn attach_matching_recovers_the_metal_roof_preset() {
        let p = profile_from_attach(&metal_roof());
        assert!(p.initial_solar_gain_f > 15.0, "solar gain from preset: {p:?}");
        assert_eq!(p.sky_exposure, 1.0, "authored numbers survive");
        assert!(!p.metabolic);
    }

    #[test]
    fn real_sim_puts_a_metal_roof_below_ambient_under_clear_not_overcast() {
        let (scene, roof) = roof_scene();

        let mut clear = ThermalPreview::new(Forecast::Clear);
        clear.set_scene(&scene);
        assert_eq!(clear.len(), 1, "the attached node is registered");
        clear.seek(0.5, Forecast::Clear);
        let t_clear = clear.display_temp_f(roof, Some(&metal_roof()));
        assert!(
            t_clear < clear.ambient_f() - 1.0,
            "clear mid-night: roof {t_clear:.1} vs ambient {:.1}",
            clear.ambient_f()
        );

        let mut overcast = ThermalPreview::new(Forecast::Overcast);
        overcast.set_scene(&scene);
        overcast.seek(0.5, Forecast::Overcast);
        let t_over = overcast.display_temp_f(roof, Some(&metal_roof()));
        assert!(
            t_over >= overcast.ambient_f() - 0.1,
            "overcast: no radiative drop ({t_over:.1} vs {:.1})",
            overcast.ambient_f()
        );
    }

    #[test]
    fn pest_profile_holds_body_temperature_all_night() {
        let mut prev = ThermalPreview::new(Forecast::Clear);
        let pest = da_core::NodeId(4242);
        prev.register_profile(pest, ThermalProfile::pest());
        for t in [0.1_f32, 0.5, 0.85, 1.0] {
            prev.seek(t, Forecast::Clear);
            let temp = prev.display_temp_f(pest, None);
            assert!(
                (temp - 101.0).abs() < 1.5,
                "pest at t={t}: {temp:.1} °F (expected ~101)"
            );
        }
    }

    #[test]
    fn forward_scrub_is_incremental_and_matches_a_fresh_run() {
        let (scene, roof) = roof_scene();
        let mut stepped = ThermalPreview::new(Forecast::Clear);
        stepped.set_scene(&scene);
        for t in [0.1_f32, 0.2, 0.3, 0.4, 0.6] {
            stepped.seek(t, Forecast::Clear);
        }
        let mut direct = ThermalPreview::new(Forecast::Clear);
        direct.set_scene(&scene);
        direct.seek(0.6, Forecast::Clear);

        let a = stepped.display_temp_f(roof, None);
        let b = direct.display_temp_f(roof, None);
        assert!((a - b).abs() < 0.5, "incremental {a:.2} vs fresh {b:.2}");
    }

    #[test]
    fn backward_scrub_reruns_from_dusk() {
        let (scene, roof) = roof_scene();
        let mut prev = ThermalPreview::new(Forecast::Clear);
        prev.set_scene(&scene);
        prev.seek(0.3, Forecast::Clear);
        let early = prev.display_temp_f(roof, None);
        prev.seek(0.9, Forecast::Clear);
        prev.seek(0.3, Forecast::Clear);
        let again = prev.display_temp_f(roof, None);
        assert!(
            (early - again).abs() < 1e-3,
            "scrubbing back is deterministic: {early:.3} vs {again:.3}"
        );
    }

    #[test]
    fn changing_forecast_rebuilds_the_sim() {
        let (scene, roof) = roof_scene();
        let mut prev = ThermalPreview::new(Forecast::Clear);
        prev.set_scene(&scene);
        prev.seek(0.6, Forecast::Clear);
        let clear = prev.display_temp_f(roof, None);
        prev.seek(0.6, Forecast::Overcast);
        let overcast = prev.display_temp_f(roof, None);
        assert!(overcast > clear, "{overcast:.1} should be warmer than {clear:.1}");
    }

    #[test]
    fn min_max_readout_spans_the_registered_nodes() {
        let (scene, _) = roof_scene();
        let mut prev = ThermalPreview::new(Forecast::Clear);
        prev.set_scene(&scene);
        prev.register_profile(da_core::NodeId(999), ThermalProfile::pest());
        prev.seek(0.5, Forecast::Clear);
        let (lo, hi) = prev.min_max_f().expect("two registered nodes");
        assert!(lo < prev.ambient_f(), "cold roof is the minimum");
        assert!(hi > 95.0, "the pest is the maximum: {hi:.1}");
    }

    #[test]
    fn empty_preview_has_no_range_and_falls_back_to_the_estimate() {
        let mut prev = ThermalPreview::new(Forecast::Clear);
        prev.seek(0.4, Forecast::Clear);
        assert!(prev.is_empty());
        assert!(prev.min_max_f().is_none());
        let roof = metal_roof();
        assert_eq!(
            prev.display_temp_f(da_core::NodeId(1), Some(&roof)),
            prev.env().display_temp_f(Some(&roof)),
            "unregistered nodes use the closed-form estimate"
        );
    }
}
