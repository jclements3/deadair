//! Required scenario tests for da-econ, driven through the public API.

use da_core::Forecast;
use da_econ::{
    business::{Business, EconError},
    contract::{expected_night_value, Contract, ContractBoard, ContractStatus, WeatherBonus},
    ledger::{KillRecord, NightLedger},
    license::License,
    save::{load_from_ron, save_to_ron},
    species::Species,
    store::{Accessory, OpticModel, RifleModel, REGULATOR_RETROFIT_CENTS},
    CAMP_FEE_CENTS,
};

/// SDD §7.5 example night: 11 rats + 2 possums = $138 bounties, cat
/// penalty −$150, operating costs −$31, NET −$43, balance $612.
#[test]
fn pnl_reproduces_sdd_7_5_example() {
    let mut b = Business::new();
    // License B for possums (no gates beyond cash).
    b.buy_license(License::B).unwrap();
    // Balance before the night must be $655 so that 655 − 43 = 612.
    b.cash_cents = 65_500;

    let mut ledger = NightLedger::new(
        7,
        "Grain Co-op",
        Forecast::Overcast,
        RifleModel::MultiPump,
        Some(OpticModel::NvBasic),
    );
    for _ in 0..11 {
        assert_eq!(ledger.record_kill(Species::Rat, &b), KillRecord::Bounty(800));
    }
    for _ in 0..2 {
        assert_eq!(
            ledger.record_kill(Species::Possum, &b),
            KillRecord::Bounty(2_500)
        );
    }
    assert_eq!(
        ledger.record_kill(Species::Cat, &b),
        KillRecord::FriendlyHit
    );
    // 28 shots on the Tier 1 pump + 4 h of NV: camp $15 + pellets $1
    // + battery $8 + maintenance $7 = $31.
    ledger.record_shots(28);
    ledger.record_optic_hours(4.0);

    let pnl = b.settle_night(&ledger);
    assert_eq!(pnl.bounties_cents, 13_800);
    assert_eq!(pnl.penalties, vec![("Penalty (cat)".to_string(), 15_000)]);
    assert_eq!(pnl.operating_costs_cents, 3_100);
    assert_eq!(pnl.net_cents, -4_300);
    assert_eq!(pnl.balance_after_cents, 61_200);
    assert_eq!(b.cash_cents, 61_200);

    let screen = pnl.to_string();
    assert!(screen.contains("NIGHT 7 — GRAIN CO-OP"), "{screen}");
    assert!(screen.contains("Bounties (11 rats, 2 possums)"), "{screen}");
    assert!(screen.contains("+$138"), "{screen}");
    assert!(screen.contains("Penalty (cat)"), "{screen}");
    assert!(screen.contains("-$150"), "{screen}");
    assert!(screen.contains("Operating costs"), "{screen}");
    assert!(screen.contains("-$31"), "{screen}");
    assert!(screen.contains("NET"), "{screen}");
    assert!(screen.contains("-$43"), "{screen}");
    assert!(screen.contains("Balance: $612"), "{screen}");
}

/// FR-B6: bankruptcy requires cash < 0 AND no sellable assets.
#[test]
fn bankruptcy_needs_negative_cash_and_no_assets() {
    let mut b = Business::new();
    b.buy_rifle(RifleModel::MultiPump).unwrap();
    b.cash_cents = -5_000;
    // Negative cash but the rifle is sellable: not bankrupt yet.
    assert!(!b.is_bankrupt());
    // Sell the rifle; cash climbs back above zero: still not bankrupt.
    b.sell_equipment(0).unwrap();
    assert!(b.cash_cents > 0);
    assert!(!b.is_bankrupt());
    // Now go negative with nothing left.
    b.cash_cents = -1;
    assert!(b.is_bankrupt());
    // Zero cash with nothing sellable is broke, not bankrupt.
    b.cash_cents = 0;
    assert!(!b.is_bankrupt());
}

/// FR-B3 / SDD §7.4: a raccoon without License C is poaching — $0 and a
/// reputation penalty.
#[test]
fn raccoon_without_license_c_is_poaching() {
    let mut b = Business::new();
    b.cash_cents = 100_000;
    b.buy_license(License::B).unwrap();
    assert!(!b.can_hunt(Species::Raccoon));

    let mut ledger = NightLedger::new(
        3,
        "River Farm",
        Forecast::Clear,
        RifleModel::MultiPump,
        None,
    );
    assert_eq!(
        ledger.record_kill(Species::Raccoon, &b),
        KillRecord::Poached
    );
    let cash_before = b.cash_cents;
    let rep_before = b.rep("River Farm");
    let pnl = b.settle_night(&ledger);
    // $0 for the animal; only the camp fee moves the cash.
    assert_eq!(pnl.bounties_cents, 0);
    assert_eq!(b.cash_cents, cash_before - CAMP_FEE_CENTS);
    assert!(b.rep("River Farm") < rep_before);
    assert!(pnl.penalties.iter().any(|(l, c)| l.contains("Poaching") && *c == 0));
}

/// SDD §7.2 retrofit paths: unregulated Tier 2 + retrofit undercuts Tier 3
/// outright; the regulated Tier 2 variant + retrofit is the trap.
#[test]
fn retrofit_price_paths() {
    // Smart path: $450 + $300 = $750 < $850.
    let smart = RifleModel::UnregulatedPcp.price_cents() + REGULATOR_RETROFIT_CENTS;
    assert_eq!(smart, 75_000);
    assert!(smart < RifleModel::RegulatedPcp.price_cents());

    // Trap path: $600 + $300 = $900 > $850, and the store warns about it.
    let trap = RifleModel::RegulatedTier2Variant.price_cents() + REGULATOR_RETROFIT_CENTS;
    assert_eq!(trap, 90_000);
    assert!(trap > RifleModel::RegulatedPcp.price_cents());
    assert!(RifleModel::RegulatedTier2Variant.warning().is_some());

    // Both paths actually produce a Tier 3 rifle in play.
    for start in [RifleModel::UnregulatedPcp, RifleModel::RegulatedTier2Variant] {
        let mut b = Business::new();
        b.cash_cents = 200_000;
        let before = b.cash_cents;
        b.buy_rifle(start).unwrap();
        b.retrofit_regulator().unwrap();
        assert_eq!(b.best_rifle_tier(), 3);
        assert_eq!(before - b.cash_cents, start.price_cents() + REGULATOR_RETROFIT_CENTS);
    }
}

/// FR-E2: dropping below a contract's reputation requirement cancels it,
/// and the board hides contracts the player can't take.
#[test]
fn reputation_below_threshold_cancels_contract() {
    let mut b = Business::new();
    b.adjust_rep("Grain Co-op", 30);

    let board = ContractBoard {
        contracts: vec![
            Contract::new(1, "Grain Co-op", "silos", Species::Rat, 10, 5, 20, None),
            // Hidden: rep too low.
            Contract::new(2, "Grain Co-op", "silos", Species::Rat, 10, 5, 60, None),
            // Hidden: no possum license yet.
            Contract::new(3, "Grain Co-op", "barn", Species::Possum, 5, 5, 0, None),
        ],
    };
    let visible: Vec<u32> = board.visible(&b).iter().map(|c| c.id).collect();
    assert_eq!(visible, vec![1]);

    b.accept_contract(board.contracts[0].clone()).unwrap();
    assert_eq!(b.contracts()[0].status, ContractStatus::Accepted);

    // Reputation collapses below the requirement (e.g. a shot cow).
    b.adjust_rep("Grain Co-op", -25);
    let cancelled = b.enforce_reputation();
    assert_eq!(cancelled, vec![1]);
    assert_eq!(b.contracts()[0].status, ContractStatus::Cancelled);
}

/// FR-WX3: skipping a night charges the $15 camp fee and burns one
/// contract-deadline night.
#[test]
fn skip_night_charges_camp_fee_and_burns_deadline() {
    let mut b = Business::new();
    let contract = Contract::new(1, "Grain Co-op", "silos", Species::Rat, 10, 3, 0, None);
    b.accept_contract(contract).unwrap();
    let cash_before = b.cash_cents;
    let night_before = b.night;

    let pnl = b.skip_night();
    assert_eq!(b.cash_cents, cash_before - CAMP_FEE_CENTS);
    assert_eq!(pnl.net_cents, -CAMP_FEE_CENTS);
    assert_eq!(pnl.operating_costs_cents, CAMP_FEE_CENTS);
    assert_eq!(b.night, night_before + 1);
    assert_eq!(b.contracts()[0].deadline_nights, 2);

    // Skip until the deadline expires: contract fails, reputation dips.
    b.skip_night();
    b.skip_night();
    assert_eq!(b.contracts()[0].status, ContractStatus::Failed);
    assert!(b.rep("Grain Co-op") < 0);
}

/// SDD §10: whole business state round-trips through versioned RON.
#[test]
fn save_round_trips_through_ron() {
    let mut b = Business::new();
    b.cash_cents = 100_000;
    b.buy_rifle(RifleModel::UnregulatedPcp).unwrap();
    b.buy_optic(OpticModel::Headlamp).unwrap();
    b.buy_accessory(Accessory::PelletTin).unwrap();
    b.buy_license(License::B).unwrap();
    b.adjust_rep("Grain Co-op", 40);
    b.adjust_rep(da_econ::TOWN_CLIENT, 10);
    b.accept_contract(Contract::new(
        7,
        "Grain Co-op",
        "silos",
        Species::Rat,
        12,
        4,
        10,
        Some(WeatherBonus {
            forecast: Forecast::PreStorm,
            multiplier: 1.5,
        }),
    ))
    .unwrap();

    let text = save_to_ron(&b).unwrap();
    assert!(text.contains("version: 1"));
    let loaded = load_from_ron(&text).unwrap();
    assert_eq!(loaded, b);
}

/// FR-WX3/§7A: the forecast-panel expected value ranks PreStorm > Clear >
/// Rain for a rat contract.
#[test]
fn expected_value_ranks_forecasts() {
    let contract = Contract::new(1, "Grain Co-op", "silos", Species::Rat, 10, 5, 0, None);
    let pre = expected_night_value(Forecast::PreStorm, &contract);
    let clear = expected_night_value(Forecast::Clear, &contract);
    let rain = expected_night_value(Forecast::Rain, &contract);
    assert!(pre > clear && clear > rain, "pre={pre} clear={clear} rain={rain}");
}

/// FR-WX4: a weather-bonus contract multiplies the bounty on matching
/// nights, and quota completion pays reputation.
#[test]
fn weather_bonus_contract_pays_more_and_completes() {
    let mut b = Business::new();
    b.accept_contract(Contract::new(
        1,
        "Grain Co-op",
        "silos",
        Species::Rat,
        3,
        5,
        0,
        Some(WeatherBonus {
            forecast: Forecast::PreStorm,
            multiplier: 1.5,
        }),
    ))
    .unwrap();

    let mut ledger = NightLedger::new(
        1,
        "Grain Co-op",
        Forecast::PreStorm,
        RifleModel::MultiPump,
        None,
    );
    for _ in 0..3 {
        ledger.record_kill(Species::Rat, &b);
    }
    let pnl = b.settle_night(&ledger);
    // 3 rats × $8 × 1.5 = $36 instead of $24.
    assert_eq!(pnl.bounties_cents, 3_600);
    assert_eq!(b.contracts()[0].status, ContractStatus::Completed);
    assert_eq!(b.rep("Grain Co-op"), 10);
}

/// Friendly fire also drags reputation down with the client.
#[test]
fn friendly_hit_costs_money_and_reputation() {
    let mut b = Business::new();
    b.adjust_rep("River Farm", 50);
    let mut ledger = NightLedger::new(
        2,
        "River Farm",
        Forecast::Clear,
        RifleModel::MultiPump,
        None,
    );
    ledger.record_kill(Species::Dog, &b);
    let pnl = b.settle_night(&ledger);
    assert_eq!(pnl.penalties[0].1, 15_000);
    assert_eq!(b.rep("River Farm"), 35);
}
