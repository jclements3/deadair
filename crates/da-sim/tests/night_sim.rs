//! Integration tests: the full sim loop — shots, noise propagation, AI
//! reactions, hazards, and end-to-end determinism.

use da_core::weather::Forecast;
use da_sim::{
    Command, Hazard, HazardKind, Light, NoiseEvent, NoiseKind, RifleConfig, ShotOutcome,
    Sim, SimEvent, Species,
};
use glam::Vec3;

fn sim_with(rifle: RifleConfig) -> Sim {
    Sim::new(1234, rifle, Forecast::Overcast.mods())
}

fn tier3_matched() -> RifleConfig {
    let mut r = RifleConfig::tier3();
    r.matched_pellets = true;
    r
}

/// Aim from the player's eye at an animal's head center.
fn head_dir(sim: &Sim, id: da_core::EntityId) -> Vec3 {
    (sim.animal(id).unwrap().target().head.center - sim.player.pos).normalize()
}

#[test]
fn headshot_kills_pest_and_queues_bounty() {
    let mut sim = sim_with(tier3_matched());
    let rat = sim.spawn(Species::Rat, Vec3::new(8.0, 0.0, 0.0));
    let dir = head_dir(&sim, rat);
    let outcome = sim.fire(dir).unwrap();
    assert!(matches!(outcome, ShotOutcome::Kill { .. }), "got {outcome:?}");
    assert!(!sim.animal(rat).unwrap().alive);

    let events = sim.drain_events();
    assert!(events.iter().any(|e| matches!(
        e,
        SimEvent::KillConfirmed { species: Species::Rat, bounty_eligible: true, .. }
    )));
    // Discharge noise and warm barrel always accompany a shot.
    assert!(events.iter().any(|e| matches!(e, SimEvent::NoiseMade { kind: NoiseKind::Discharge, .. })));
    assert!(events
        .iter()
        .any(|e| matches!(e, SimEvent::HeatResidue { kind: da_sim::HeatKind::Barrel, .. })));
}

#[test]
fn body_shot_wounds_and_alerts_nearby_pests() {
    let mut sim = sim_with(tier3_matched());
    let possum = sim.spawn(Species::Possum, Vec3::new(12.0, 0.0, 0.0));
    let bystander = sim.spawn(Species::Rat, Vec3::new(14.0, 0.0, 2.0));

    // Aim at the rear body sphere — well clear of the head collider.
    let body = sim.animal(possum).unwrap().target().body[1].center;
    let dir = (body - sim.player.pos).normalize();
    let outcome = sim.fire(dir).unwrap();
    assert!(matches!(outcome, ShotOutcome::Wound { .. }), "got {outcome:?}");

    let events = sim.drain_events();
    assert!(events.iter().any(|e| matches!(
        e,
        SimEvent::Wounded { species: Species::Possum, .. }
    )));
    // No bounty on a wound.
    assert!(!events.iter().any(|e| matches!(e, SimEvent::KillConfirmed { .. })));
    // Victim flees and the alert pulse spooks the bystander (FR-A3).
    assert!(sim.animal(possum).unwrap().is_fleeing());
    assert!(sim.animal(bystander).unwrap().is_fleeing());
    assert!(events
        .iter()
        .any(|e| matches!(e, SimEvent::PestFled { id, .. } if *id == bystander)));
}

#[test]
fn backstop_warning_and_conversion_through_the_sim() {
    let mut sim = sim_with(tier3_matched());
    // Prone shooter: the classic no-backstop line of fire.
    sim.player.pos = Vec3::new(0.0, 0.5, 0.0);
    let coon = sim.spawn(Species::Raccoon, Vec3::new(10.0, 0.0, 0.0));
    // Cow parked behind the raccoon, inside lethal range (~37 m).
    let _cow = sim.spawn(Species::Cow, Vec3::new(17.0, 0.0, 0.0));

    let dir = head_dir(&sim, coon);
    assert!(sim.check_backstop(dir), "reticle must warn: friendly behind target");

    let outcome = sim.fire(dir).unwrap();
    assert!(
        matches!(outcome, ShotOutcome::FriendlyHit { species: Species::Cow, .. }),
        "hit converts to FriendlyHit (FR-A8), got {outcome:?}"
    );
    let events = sim.drain_events();
    assert!(events.iter().any(|e| matches!(
        e,
        SimEvent::FriendlyHit { species: Species::Cow, .. }
    )));
    assert!(sim.animal(coon).unwrap().alive, "no bounty laundering through a cow");
}

#[test]
fn discharge_noise_makes_pests_inside_radius_flee() {
    let mut sim = sim_with(tier3_matched());
    let near = sim.spawn(Species::Rat, Vec3::new(30.0, 0.0, 0.0));
    let far = sim.spawn(Species::Rat, Vec3::new(500.0, 0.0, 0.0));

    // Fire at nothing; unmoderated Tier 3 discharge carries ~87 m.
    sim.fire(Vec3::new(0.0, 0.2, 1.0).normalize());
    sim.tick(0.1); // noise propagates on the next tick
    assert!(sim.animal(near).unwrap().is_fleeing());
    assert!(!sim.animal(far).unwrap().is_fleeing());
}

#[test]
fn zombie_ignores_body_shots_dies_to_head() {
    let mut sim = sim_with(tier3_matched());
    let z = sim.spawn(Species::Zombie, Vec3::new(10.0, 0.0, 0.0));

    // Body shot: stagger, no death.
    let body = sim.animal(z).unwrap().target().body[0].center;
    let outcome = sim.fire((body - sim.player.pos).normalize()).unwrap();
    assert!(matches!(outcome, ShotOutcome::ZombieStagger { .. }), "got {outcome:?}");
    assert!(sim.animal(z).unwrap().alive, "FR-A7: body shots never kill zombies");
    assert!(sim.animal(z).unwrap().is_staggered(sim.time));

    // Headshot: destroyed.
    let outcome = sim.fire(head_dir(&sim, z)).unwrap();
    assert!(matches!(outcome, ShotOutcome::ZombieDestroyed { .. }), "got {outcome:?}");
    assert!(!sim.animal(z).unwrap().alive);
    let events = sim.drain_events();
    assert!(events.iter().any(|e| matches!(e, SimEvent::ZombieStaggered { .. })));
    assert!(events.iter().any(|e| matches!(e, SimEvent::ZombieDestroyed { .. })));
    // Zombies never pay (FR-A7): no KillConfirmed for them.
    assert!(!events.iter().any(|e| matches!(e, SimEvent::KillConfirmed { .. })));
}

#[test]
fn zombie_converges_on_noise_within_60_seconds() {
    let mut sim = sim_with(tier3_matched());
    let z = sim.spawn(Species::Zombie, Vec3::new(25.0, 0.0, 0.0));
    let src = Vec3::new(0.0, 0.0, 0.0);
    let start_dist = (sim.animal(z).unwrap().pos - src).length();

    sim.emit_noise(NoiseEvent { pos: src, radius_m: 60.0, kind: NoiseKind::Discharge });
    for _ in 0..600 {
        sim.tick(0.1); // 60 s of sim time
    }
    let end_dist = (sim.animal(z).unwrap().pos - src).length();
    assert!(
        end_dist < 5.0,
        "zombie should close on the noise source: {start_dist} -> {end_dist}"
    );
}

#[test]
fn zombie_contact_damage_hurts_player() {
    let mut sim = sim_with(tier3_matched());
    // Zombie already in the player's face.
    sim.spawn(Species::Zombie, Vec3::new(0.5, 0.0, 0.0));
    let hp0 = sim.player.health.hp;
    let events = sim.tick(0.1);
    assert!(sim.player.health.hp < hp0);
    assert!(events.iter().any(|e| matches!(
        e,
        SimEvent::PlayerDamaged { cause: da_sim::DamageCause::ZombieContact, .. }
    )));
}

#[test]
fn possum_freezes_when_lit_and_barely_reacts_to_noise() {
    let mut sim = sim_with(tier3_matched());
    let lit = sim.spawn(Species::Possum, Vec3::new(5.0, 0.0, 5.0));
    let unlit = sim.spawn(Species::Possum, Vec3::new(30.0, 0.0, 5.0));
    sim.light = Some(Light { pos: Vec3::new(5.0, 0.0, 5.0), radius_m: 3.0 });

    sim.tick(0.2);
    assert!(sim.animal(lit).unwrap().is_frozen(), "possum freezes in the beam");
    let frozen_pos = sim.animal(lit).unwrap().pos;

    // A noise both can hear: lit possum at 10 m only listens to
    // 30 m x 0.3 = 9 m while frozen; unlit possum at 15 m bolts.
    sim.emit_noise(NoiseEvent {
        pos: Vec3::new(15.0, 0.0, 5.0),
        radius_m: 30.0,
        kind: NoiseKind::Discharge,
    });
    sim.tick(0.2);
    let lit_p = sim.animal(lit).unwrap();
    assert!(lit_p.is_frozen() && !lit_p.is_fleeing(), "freeze lowers flee response");
    assert_eq!(lit_p.pos, frozen_pos, "frozen possum holds still");
    assert!(sim.animal(unlit).unwrap().is_fleeing(), "unlit possum flees the same noise");
}

#[test]
fn raccoon_group_memory_raises_flee_threshold_after_witnessed_kill() {
    let mut sim = sim_with(tier3_matched());
    let victim = sim.spawn_animal(Species::Raccoon, Vec3::new(12.0, 0.0, 0.0), Some(1), vec![]);
    let witness = sim.spawn_animal(Species::Raccoon, Vec3::new(12.0, 0.0, 18.0), Some(1), vec![]);
    // Different group, also nearby: must NOT learn from this death.
    let stranger = sim.spawn_animal(Species::Raccoon, Vec3::new(12.0, 0.0, -18.0), Some(2), vec![]);

    assert_eq!(sim.animal(witness).unwrap().flee_radius_multiplier(), 1.0);
    let outcome = sim.fire(head_dir(&sim, victim)).unwrap();
    assert!(matches!(outcome, ShotOutcome::Kill { .. }), "got {outcome:?}");

    let w = sim.animal(witness).unwrap();
    assert!(
        w.flee_radius_multiplier() > 1.0,
        "FR-A4: witnessed death raises the group's flee threshold"
    );
    assert_eq!(sim.animal(stranger).unwrap().flee_radius_multiplier(), 1.0);

    // Behavioral proof: a noise short of the base radius now spooks the
    // witness (12 m distance vs 10 m radius, but 10 x 1.75 = 17.5 m).
    sim.drain_events();
    let wpos = sim.animal(witness).unwrap().pos;
    sim.emit_noise(NoiseEvent {
        pos: wpos + Vec3::new(12.0, 0.0, 0.0),
        radius_m: 10.0,
        kind: NoiseKind::Other,
    });
    sim.tick(0.1);
    assert!(sim.animal(witness).unwrap().is_fleeing());
}

#[test]
fn hazard_trip_damage_scales_with_weather_severity() {
    let run = |forecast: Forecast| -> f32 {
        let mut sim = Sim::new(42, tier3_matched(), forecast.mods());
        sim.add_hazard(Hazard {
            kind: HazardKind::Water,
            pos: Vec3::new(3.0, 0.0, 0.0),
            radius: 1.5,
        });
        // Sprint into the creek: trip chance saturates at 1.0 -> always trips.
        sim.move_player(Vec3::new(3.0, 0.0, 0.0), 0.5);
        let events = sim.drain_events();
        events
            .iter()
            .find_map(|e| match e {
                SimEvent::PlayerDamaged { amount, cause: da_sim::DamageCause::Hazard(_) } => {
                    Some(*amount)
                }
                _ => None,
            })
            .expect("sprinting into water at saturated trip chance must hurt")
    };
    let dry = run(Forecast::Overcast); // severity 1.0
    let wet = run(Forecast::Rain); // severity 1.5
    assert!(wet > dry, "rain-slick hazards hit harder: {wet} vs {dry}");
}

#[test]
fn pump_strokes_emit_movement_noise() {
    let mut sim = sim_with(RifleConfig::tier1());
    sim.pump(2.5); // two full strokes at 1.1 s
    let events = sim.drain_events();
    let strokes = events
        .iter()
        .filter(|e| matches!(e, SimEvent::NoiseMade { kind: NoiseKind::PumpStroke, .. }))
        .count();
    assert_eq!(strokes, 2);
    // Dry fire before pumping enough is refused with an event.
    let mut empty = sim_with(RifleConfig::tier1());
    assert!(empty.fire(Vec3::X).is_none());
    assert!(empty.drain_events().iter().any(|e| matches!(e, SimEvent::DryFire)));
}

#[test]
fn rat_respawns_after_kill() {
    let mut sim = sim_with(tier3_matched());
    let rat = sim.spawn(Species::Rat, Vec3::new(8.0, 0.0, 0.0));
    sim.fire(head_dir(&sim, rat));
    assert!(!sim.animal(rat).unwrap().alive);
    for _ in 0..300 {
        sim.tick(0.1); // 30 s > 25 s respawn timer
    }
    assert!(sim.animal(rat).unwrap().alive, "population pressure: rats come back");
}

#[test]
fn determinism_same_seed_same_script_identical_event_log() {
    let build = |seed: u64| {
        let mut sim = Sim::new(seed, RifleConfig::tier1(), Forecast::Fog.mods());
        sim.spawn(Species::Rat, Vec3::new(9.0, 0.0, 1.0));
        sim.spawn(Species::Possum, Vec3::new(15.0, 0.0, -4.0));
        sim.spawn_animal(
            Species::Raccoon,
            Vec3::new(20.0, 0.0, 6.0),
            Some(1),
            vec![Vec3::new(25.0, 0.0, 6.0), Vec3::new(20.0, 0.0, 12.0)],
        );
        sim.spawn(Species::Zombie, Vec3::new(-12.0, 0.0, 3.0));
        sim.spawn(Species::Dog, Vec3::new(6.0, 0.0, -8.0));
        sim.add_hazard(Hazard {
            kind: HazardKind::Wire,
            pos: Vec3::new(2.0, 0.0, 0.0),
            radius: 1.0,
        });
        sim
    };
    let script = vec![
        Command::Tick { dt: 0.5 },
        Command::Pump { dt: 9.0 }, // 8 strokes, capped
        Command::Move { delta: Vec3::new(2.0, 0.0, 0.0), dt: 1.0 },
        Command::Tick { dt: 0.5 },
        Command::Fire { dir: Vec3::new(1.0, -0.15, 0.11).normalize() },
        Command::Tick { dt: 2.0 },
        Command::EmitNoise { pos: Vec3::new(5.0, 0.0, 5.0), radius_m: 50.0 },
        Command::Tick { dt: 5.0 },
        Command::Tick { dt: 5.0 },
    ];
    let log_a = build(777).run_script(&script);
    let log_b = build(777).run_script(&script);
    assert!(!log_a.is_empty(), "script should produce events");
    assert_eq!(log_a, log_b, "same seed + same commands = identical event log");

    // A different seed diverges (shot dispersion moves the pellet impact).
    let log_c = build(778).run_script(&script);
    assert_ne!(log_a, log_c, "different seed should change the night");
}
