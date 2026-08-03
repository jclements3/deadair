//! Headless full-campaign integration test (SRS goal statement: "a stranger
//! can play DeadAir start to bankruptcy-or-riches"). A scripted player runs
//! whole nights on the real Home Farm zone: expand from parametric source,
//! thermal sim ticking dusk→dawn, AI acting, aimed shots resolved, economy
//! settling a P&L every night. No GPU, no window.

use da_core::Forecast;
use da_econ::Business;
use deadair::hunt::{Mounted, NightHunt};
use glam::Vec3;

fn zone() -> String {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/zones/home_farm.zone.ron"
    )
    .to_string()
}

/// Walk toward the nearest live pest and take head shots when in range.
/// Returns kills made this night.
fn scripted_night(h: &mut NightHunt, business: &Business) -> u32 {
    let mut kills = 0;
    // Play the night in 2-second slices until dawn.
    while !h.over {
        h.tick(2.0, Vec3::ZERO, false);

        // Keep the multi-pump charged.
        if !h.sim.rifle.plant.can_fire() {
            h.sim.pump(10.0);
        }

        // Nearest live pest head.
        let eye = h.sim.player.pos;
        let target = h
            .sim
            .animals
            .iter()
            .filter(|a| a.alive && !a.species.is_friendly() && a.species != da_sim::Species::Zombie)
            .min_by(|a, b| {
                a.pos
                    .distance(eye)
                    .partial_cmp(&b.pos.distance(eye))
                    .expect("finite")
            })
            .map(|a| (a.target(), a.pos));
        let Some((tgt, pos)) = target else { break };

        let dist = pos.distance(eye);
        if dist > h.sim.rifle.lethal_range_m() * 0.8 {
            // March toward it.
            let dir = (pos - eye).normalize_or_zero();
            h.tick(2.0, Vec3::new(dir.x, 0.0, dir.z) * 4.0, false);
            continue;
        }
        let dir = (tgt.head.center - eye).normalize_or_zero();
        if h.sim.check_backstop(dir) {
            // Sidestep and try again next slice.
            h.tick(1.0, Vec3::new(1.5, 0.0, 0.0), false);
            continue;
        }
        let before = h.ledger.confirmed_kills().len();
        h.fire(dir, business);
        if h.ledger.confirmed_kills().len() > before {
            kills += 1;
        }
    }
    kills
}

#[test]
fn campaign_runs_nights_and_money_flows() {
    let mut business = Business::new();
    let start_cash = business.cash_cents;
    let mut total_kills = 0u32;
    let mut nets = Vec::new();

    for night in 0..3 {
        let mut h = NightHunt::new(
            &zone(),
            Forecast::Clear,
            &business,
            1000 + night,
            Mounted::Headlamp,
        )
        .expect("hunt boots");
        total_kills += scripted_night(&mut h, &business);
        let st = business.settle_night(&h.ledger);
        nets.push(st.net_cents);
    }

    assert!(total_kills > 0, "scripted player must land kills");
    // Accounting identity: cash delta == sum of nightly nets.
    let net_sum: i64 = nets.iter().sum();
    assert_eq!(
        business.cash_cents - start_cash,
        net_sum,
        "P&L must reconcile with the bank balance"
    );
    assert_eq!(business.night, 4, "night counter starts at 1 and advances per settle");
    assert!(!business.is_bankrupt(), "3 careful nights shouldn't bankrupt");
}

#[test]
fn skipped_nights_bleed_camp_fees_toward_bankruptcy() {
    let mut business = Business::new();
    let start = business.cash_cents;
    for _ in 0..4 {
        business.skip_night();
    }
    assert_eq!(start - business.cash_cents, 4 * da_econ::CAMP_FEE_CENTS);
}

#[test]
fn dawn_always_ends_the_night() {
    let business = Business::new();
    let mut h = NightHunt::new(&zone(), Forecast::Rain, &business, 7, Mounted::Headlamp)
        .expect("hunt boots");
    let mut guard = 0;
    while !h.over {
        h.tick(30.0, Vec3::new(0.0, 0.0, -1.0), true);
        guard += 1;
        assert!(guard < 200, "night must terminate");
    }
    assert!(h.clock.is_dawn());
}

#[test]
fn zombies_never_pay() {
    let mut business = Business::new();
    let mut h = NightHunt::new(&zone(), Forecast::Clear, &business, 99, Mounted::Headlamp)
        .expect("hunt boots");
    // Force-kill every zombie via ledger classification.
    let zombies: Vec<_> = h
        .sim
        .animals
        .iter()
        .filter(|a| a.species == da_sim::Species::Zombie)
        .map(|a| a.id)
        .collect();
    for _ in &zombies {
        h.ledger.record_kill(da_econ::Species::Zombie, &business);
    }
    let st = business.settle_night(&h.ledger);
    assert_eq!(st.bounties_cents, 0, "zombie 'bounties' must be zero");
}

#[test]
fn every_zone_boots_as_a_playable_night() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/zones");
    let catalog = deadair::camp::ZoneCatalog::load(dir).expect("catalog");
    let business = Business::new();
    assert_eq!(catalog.zones.len(), 6);
    for z in &catalog.zones {
        let path = format!("{dir}/{}", z.file);
        let h = NightHunt::new(&path, Forecast::Clear, &business, 5, Mounted::Headlamp)
            .unwrap_or_else(|e| panic!("{} failed to boot: {e}", z.name));
        assert!(
            !h.sim.animals.is_empty(),
            "{} spawned no animals — check species coverage",
            z.name
        );
        assert!(
            h.thermal.len() > 20,
            "{} registered too few thermal nodes: {}",
            z.name,
            h.thermal.len()
        );
        let dl = h.draw_list();
        assert!(dl.items.len() > 30, "{} draw list too thin", z.name);
    }
}
