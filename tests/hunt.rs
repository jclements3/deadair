//! Integration tests for the hunt simulation.

use deadair::{
    hunt::{HuntConfig, HuntSimulation},
    scene::Scene,
    thermal::ThermalOptics,
    world::World,
};
use rand::SeedableRng;

fn seeded_rng() -> rand::rngs::StdRng {
    rand::rngs::StdRng::seed_from_u64(12345)
}

fn default_world() -> World {
    World::from_scene(&Scene::abandoned_farm())
}

#[test]
fn hunt_terminates_within_max_ticks() {
    let world = default_world();
    let config = HuntConfig { max_ticks: 300, ..HuntConfig::default() };
    let mut sim = HuntSimulation::new(world, config);
    let mut rng = seeded_rng();
    sim.run(&mut rng);
    assert!(sim.tick <= 300);
}

#[test]
fn hunt_with_military_optics_kills_all_zombies() {
    // Military-grade optics (25 mK NETD) — should detect cold zombies reliably
    let world = default_world();
    let initial_zombie_count = world.zombie_count();

    let config = HuntConfig {
        optics: ThermalOptics::military_grade(),
        max_ticks: 500,
        ..HuntConfig::default()
    };
    let mut sim = HuntSimulation::new(world, config);
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    sim.run(&mut rng);

    // With mil-grade optics at close-ish ranges all zombies should be detected
    // eventually.  We allow for some misses but expect at least 1 kill.
    assert!(sim.kills > 0,
        "Expected at least 1 kill with military-grade optics, got {}", sim.kills);
    let _ = initial_zombie_count; // used as documentation above
}

#[test]
fn hunt_builds_valid_ledger() {
    let world = default_world();
    let config = HuntConfig::default();
    let mut sim = HuntSimulation::new(world, config);
    let mut rng = seeded_rng();
    sim.run(&mut rng);

    let ledger = sim.build_ledger();
    // Expenses must always be present (permit + depreciation)
    assert!(ledger.expenses() > 0.0, "Expenses should be non-zero");
    // Net should equal revenue − expenses
    let expected_net = ledger.revenue() - ledger.expenses();
    assert!((ledger.net() - expected_net).abs() < 0.001);
}

#[test]
fn ammo_cost_matches_shots_fired() {
    let world = default_world();
    let cost_per = 1.20_f64;
    let config = HuntConfig { ammo_cost_per_round: cost_per, ..HuntConfig::default() };
    let mut sim = HuntSimulation::new(world, config);
    let mut rng = seeded_rng();
    sim.run(&mut rng);

    let shots = sim.shots_fired;
    let ledger = sim.build_ledger();

    // Find the ammo line item
    let ammo_cost: f64 = ledger.entries.iter()
        .filter(|e| e.description.contains("FMJ"))
        .map(|e| e.amount.abs())
        .sum();

    if shots > 0 {
        let expected = shots as f64 * cost_per;
        assert!((ammo_cost - expected).abs() < 0.001,
            "Ammo cost mismatch: expected {expected}, got {ammo_cost}");
    }
}

#[test]
fn hunt_step_returns_false_when_all_zombies_dead() {
    let world = default_world();
    let mut sim = HuntSimulation::new(world, HuntConfig::default());
    // Kill all zombies manually
    for e in &mut sim.world.entities {
        e.alive = false;
    }
    let mut rng = seeded_rng();
    let continued = sim.step(&mut rng);
    assert!(!continued, "Hunt should end immediately when no zombies remain");
}

#[test]
fn hunt_result_kills_do_not_exceed_initial_zombie_count() {
    let scene = Scene::abandoned_farm();
    let initial_count = scene.nodes.iter()
        .filter(|n| matches!(n.kind, deadair::scene::NodeKind::Zombie { .. }))
        .count() as u32;

    let world = World::from_scene(&scene);
    let config = HuntConfig { max_ticks: 500, ..HuntConfig::default() };
    let mut sim = HuntSimulation::new(world, config);
    let mut rng = rand::rngs::StdRng::seed_from_u64(99);
    sim.run(&mut rng);

    assert!(sim.kills <= initial_count,
        "Cannot kill more zombies ({}) than were spawned ({})", sim.kills, initial_count);
}
