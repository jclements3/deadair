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

        // Nearest live pest head — from the POSE-TRUE rig colliders, the
        // same ones the shot resolves against (a real player aims at what
        // they see; the canonical layouts no longer decide hits).
        let eye = h.sim.player.pos;
        let target = h
            .rig_targets()
            .into_iter()
            .filter(|t| !t.species.is_friendly() && t.species != da_sim::Species::Zombie)
            .min_by(|a, b| {
                a.pos
                    .distance(eye)
                    .partial_cmp(&b.pos.distance(eye))
                    .expect("finite")
            })
            .map(|t| (t.clone(), t.pos));
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

fn stocked(business: &mut Business) {
    business
        .buy_accessory(da_econ::Accessory::PelletTin)
        .expect("tin");
    business
        .buy_accessory(da_econ::Accessory::PelletTin)
        .expect("tin");
}

#[test]
fn campaign_runs_nights_and_money_flows() {
    let mut business = Business::new();
    stocked(&mut business);
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

#[test]
fn audio_and_eyeshine_come_alive_during_a_night() {
    let mut business = Business::new();
    stocked(&mut business);
    let mut h = NightHunt::new(&zone(), Forecast::Clear, &business, 21, Mounted::NvBasic)
        .expect("hunt boots");

    // Close on the nearest pest until it is within earshot — a rat's rustle
    // only carries a few metres, which is the point of the channel.
    for _ in 0..120 {
        let eye = h.sim.player.pos;
        let Some(target) = h
            .sim
            .animals
            .iter()
            .filter(|a| a.alive && a.is_targetable())
            .map(|a| a.pos)
            .min_by(|a, b| {
                a.distance(eye)
                    .partial_cmp(&b.distance(eye))
                    .expect("finite")
            })
        else {
            break;
        };
        let to = target - eye;
        if to.length() < 3.0 {
            h.tick(1.0, Vec3::ZERO, true);
            break;
        }
        let dir = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
        h.tick(1.0, dir * 4.0, true);
    }

    assert!(
        !h.subtitles.is_empty(),
        "a populated farm must be audible — audio is the fourth optic"
    );

    let dl = h.draw_list();
    assert!(
        !dl.eyeshine.is_empty(),
        "living animals in IR reach must return eyeshine"
    );

    // The invariant that matters: no zombie ever contributes eyeshine.
    let zombie_heads: Vec<_> = h
        .sim
        .animals
        .iter()
        .filter(|a| a.species == da_sim::Species::Zombie)
        .map(|a| a.pos)
        .collect();
    for z in &zombie_heads {
        for e in &dl.eyeshine {
            let horizontal = (e.pos - *z) * Vec3::new(1.0, 0.0, 1.0);
            assert!(
                horizontal.length() > 0.5,
                "a zombie must never retro-reflect: eyeshine at {:?} sits on a zombie at {z:?}",
                e.pos
            );
        }
    }
}

#[test]
fn residual_heat_reaches_the_thermal_view() {
    let mut business = Business::new();
    stocked(&mut business);
    let mut h = NightHunt::new(&zone(), Forecast::Clear, &business, 33, Mounted::Thermal(1))
        .expect("hunt boots");
    h.sim.pump(10.0);
    // A discharge leaves a hot barrel trace (SDD §2.3 / FR-T4). Fire at the
    // sky: barrel heat is about the discharge, not the impact, and a sky
    // shot can never be refused by the backstop rule whatever the spawn
    // layout happens to be.
    h.fire(Vec3::new(0.0, 1.0, 0.2).normalize(), &business);
    h.tick(0.5, Vec3::ZERO, true);
    let dl = h.draw_list();
    assert!(
        dl.heat_decals.iter().any(|d| d.delta_f > 1.0),
        "firing must leave visible residual heat in the thermal view"
    );
}
