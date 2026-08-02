//! Full-night integration tests for the thermal simulation (SDD §2,
//! SRS FR-T1..T4). A standard night is 2400 real seconds (NightClock
//! standard: 10 in-game hours over 40 minutes); tests step the sim at its
//! native 1 Hz.

use da_core::clock::CROSSOVER_T;
use da_core::{Forecast, NodeId, TempF};
use da_thermal::{
    ambient_at, contrast, detection_range_factor, HeatEvent, ThermalProfile, ThermalSim,
};
use glam::Vec3;

const NIGHT_SECS: u32 = 2400;

fn t_at(sec: u32) -> f32 {
    sec as f32 / NIGHT_SECS as f32
}

/// Step `sim` one second at a time through `[from, to)` night-seconds.
fn run(sim: &mut ThermalSim, from: u32, to: u32) {
    for s in from..to {
        sim.step(1.0, t_at(s + 1));
    }
}

// (a) Zombie invisibility rule (SDD §4.1): display temp == ambient within
// epsilon at every t, under every forecast.
#[test]
fn zombie_equals_ambient_all_night_every_forecast() {
    for f in Forecast::ALL {
        let mut sim = ThermalSim::new(f);
        let id = NodeId(1);
        sim.register(id, ThermalProfile::zombie(), 0.0);
        assert!(
            (sim.display_temp(id).unwrap() - ambient_at(0.0, f)).abs() < 0.1,
            "{f:?} at dusk"
        );
        for s in 0..NIGHT_SECS {
            let t = t_at(s + 1);
            sim.step(1.0, t);
            let dt = sim.display_temp(id).unwrap() - ambient_at(t, f);
            assert!(dt.abs() < 0.1, "{f:?} at t={t}: zombie off ambient by {dt}");
        }
    }
}

// (b) Metabolic pest holds ~101 F all night (dry forecasts; rain applies a
// small deliberate wet-fur chill).
#[test]
fn pest_holds_body_temp_all_night() {
    for f in Forecast::ALL.into_iter().filter(|f| f.mods().wetting_rate == 0.0) {
        let mut sim = ThermalSim::new(f);
        let id = NodeId(1);
        sim.register(id, ThermalProfile::pest(), 0.0);
        for s in 0..NIGHT_SECS {
            sim.step(1.0, t_at(s + 1));
            let temp = sim.display_temp(id).unwrap();
            assert!(
                (temp - TempF::PEST_BODY).abs() < 0.75,
                "{f:?}: pest read {temp:?} at {s}s"
            );
        }
    }
}

// (c) Contrast curve shape (FR-T2): dusk > mid-night > crossover, with
// partial recovery toward dawn — under every forecast.
#[test]
fn contrast_curve_shape() {
    for f in Forecast::ALL {
        let dusk = contrast(0.05, f);
        let mid = contrast(0.5, f);
        let cross = contrast(CROSSOVER_T, f);
        let late = contrast(0.95, f);
        assert!(dusk > mid, "{f:?}: dusk {dusk} !> mid {mid}");
        assert!(mid > cross, "{f:?}: mid {mid} !> crossover {cross}");
        assert!(late > cross, "{f:?}: dawn-side {late} !> crossover {cross}");
    }
}

// Crossover is deeper under Clear (crossover_depth 0.25) than Overcast (0.0).
#[test]
fn clear_sky_deepens_crossover() {
    let clear_dip = contrast(0.5, Forecast::Clear) - contrast(CROSSOVER_T, Forecast::Clear);
    let over_dip = contrast(0.5, Forecast::Overcast) - contrast(CROSSOVER_T, Forecast::Overcast);
    assert!(clear_dip > over_dip);
}

// (d) Rain collapse (FR-T3): two objects with different stored solar heat
// converge toward each other much faster under Rain than under Clear.
#[test]
fn rain_collapses_object_separation() {
    let sep_after = |f: Forecast| -> (f32, f32) {
        let mut sim = ThermalSim::new(f);
        let (rock, grass) = (NodeId(1), NodeId(2));
        sim.register(rock, ThermalProfile::rock(), 0.0);
        sim.register(grass, ThermalProfile::grass(), 0.0);
        let initial =
            (sim.display_temp(rock).unwrap() - sim.display_temp(grass).unwrap()).abs();
        run(&mut sim, 0, 300);
        let fin = (sim.display_temp(rock).unwrap() - sim.display_temp(grass).unwrap()).abs();
        (initial, fin)
    };
    let (clear_init, clear_fin) = sep_after(Forecast::Clear);
    let (rain_init, rain_fin) = sep_after(Forecast::Rain);
    assert!(clear_init > 5.0 && rain_init > 2.0, "objects start separated");
    assert!(
        rain_fin < 0.25 * clear_fin,
        "rain separation {rain_fin} should be far below clear {clear_fin}"
    );
    assert!(rain_fin < 1.0, "rain drives objects near-uniform: {rain_fin}");
    // And wetness actually accumulated.
    let mut sim = ThermalSim::new(Forecast::Rain);
    sim.register(NodeId(1), ThermalProfile::grass(), 0.0);
    run(&mut sim, 0, 300);
    assert!(sim.state(NodeId(1)).unwrap().wetness > 0.9);
}

// (e) Radiative cooling gate: metal roof reads below ambient by mid-night
// under Clear, but NOT under Overcast (SDD §7A).
#[test]
fn metal_roof_below_ambient_only_under_clear_sky() {
    let mid = NIGHT_SECS / 2; // t = 0.5
    let roof_vs_ambient = |f: Forecast| -> f32 {
        let mut sim = ThermalSim::new(f);
        let id = NodeId(1);
        sim.register(id, ThermalProfile::metal_roof(), 0.0);
        run(&mut sim, 0, mid);
        sim.display_temp(id).unwrap() - ambient_at(t_at(mid), f)
    };
    let clear = roof_vs_ambient(Forecast::Clear);
    let overcast = roof_vs_ambient(Forecast::Overcast);
    assert!(clear < -1.0, "clear-sky roof should read below ambient, got {clear:+.2}");
    assert!(overcast > 0.0, "overcast roof stays above ambient, got {overcast:+.2}");
    // ColdSnap is also clear-sky: radiative term applies there too.
    let snap = roof_vs_ambient(Forecast::ColdSnap);
    assert!(snap < -1.0, "cold-snap roof should read below ambient, got {snap:+.2}");
}

// (f) Residual heat lifetimes (FR-T4): barrel ~90 s, pellet ~5 s.
#[test]
fn heat_event_lifetimes() {
    let seconds_visible = |ev: HeatEvent| -> u32 {
        let mut sim = ThermalSim::new(Forecast::Clear);
        sim.spawn_heat(ev);
        let mut secs = 0;
        while sim.live_heat().count() > 0 && secs < 1000 {
            sim.step(1.0, t_at(secs + 1));
            secs += 1;
        }
        secs
    };
    let barrel = seconds_visible(HeatEvent::barrel(Vec3::new(1.0, 0.0, 2.0)));
    assert!((80..=100).contains(&barrel), "barrel visible {barrel}s, want ~90");
    let pellet = seconds_visible(HeatEvent::pellet_impact(Vec3::ZERO));
    assert!((4..=8).contains(&pellet), "pellet visible {pellet}s, want ~5");
}

// (g) Detection range multiplier at the crossover under Clear: 50-70% cut.
#[test]
fn detection_factor_at_crossover_clear() {
    let f = detection_range_factor(CROSSOVER_T, Forecast::Clear);
    assert!(
        (0.3..0.5).contains(&f),
        "crossover detection factor {f} outside 0.3..0.5"
    );
    // Early night is near full range.
    assert!(detection_range_factor(0.05, Forecast::Clear) > 0.9);
    // Never leaves its documented band.
    for forecast in Forecast::ALL {
        for i in 0..=100 {
            let v = detection_range_factor(i as f32 / 100.0, forecast);
            assert!((0.3..=1.0).contains(&v));
        }
    }
}

// Extra: high-mass objects (rock, water) cool slower than grass — the dusk
// warm-blob fake-out that decays across the night (FR-T1).
#[test]
fn thermal_mass_orders_cooling_speed() {
    let mut sim = ThermalSim::new(Forecast::Overcast);
    let (rock, grass, water) = (NodeId(1), NodeId(2), NodeId(3));
    sim.register(rock, ThermalProfile::rock(), 0.0);
    sim.register(grass, ThermalProfile::grass(), 0.0);
    sim.register(water, ThermalProfile::water(), 0.0);
    let start = |sim: &ThermalSim, id| sim.display_temp(id).unwrap();
    let (r0, g0, w0) = (start(&sim, rock), start(&sim, grass), start(&sim, water));
    run(&mut sim, 0, 600); // to t = 0.25
    let amb = ambient_at(t_at(600), Forecast::Overcast);
    let excess = |now: TempF, was: TempF| -> f32 {
        // Fraction of the initial above-ambient excess still held.
        ((now - amb) / (was - amb)).clamp(-1.0, 2.0)
    };
    let rock_kept = excess(sim.display_temp(rock).unwrap(), r0);
    let grass_kept = excess(sim.display_temp(grass).unwrap(), g0);
    let water_kept = excess(sim.display_temp(water).unwrap(), w0);
    assert!(water_kept > rock_kept, "water {water_kept} vs rock {rock_kept}");
    assert!(rock_kept > grass_kept, "rock {rock_kept} vs grass {grass_kept}");
}

// Extra: fog barely touches thermal while gutting NV (FR-T3 second clause).
#[test]
fn fog_is_kind_to_thermal() {
    let mid_fog = contrast(0.5, Forecast::Fog);
    let mid_over = contrast(0.5, Forecast::Overcast);
    assert!(mid_fog > 0.8 * mid_over, "fog thermal within 20% of overcast");
    let mods = Forecast::Fog.mods();
    assert!(mods.nv_visibility < 0.5, "NV badly scattered in fog");
}

// Extra: sub-second stepping matches whole-second stepping (accumulator).
#[test]
fn frame_rate_independence() {
    let build = || {
        let mut sim = ThermalSim::new(Forecast::Clear);
        sim.register(NodeId(1), ThermalProfile::metal_roof(), 0.0);
        sim
    };
    let mut coarse = build();
    run(&mut coarse, 0, 600);
    let mut fine = build();
    let mut acc = 0.0f32;
    while acc < 600.0 {
        acc += 0.25;
        fine.step(0.25, acc / NIGHT_SECS as f32);
    }
    let a = coarse.display_temp(NodeId(1)).unwrap();
    let b = fine.display_temp(NodeId(1)).unwrap();
    assert!((a - b).abs() < 0.2, "coarse {a:?} vs fine {b:?}");
}
