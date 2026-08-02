//! da-econ — DeadAir business simulation (SDD §7/§7A, SRS §3.6–§3.8).
//!
//! Everything with a dollar sign lives here: species bounties, the license
//! ladder, store catalogs (rifles, optics, accessories), the player's
//! business state, contracts, the nightly P&L ledger, sell-back /
//! bankruptcy rules, weather expected-value, and versioned RON save files.
//!
//! Money is tracked in integer **cents** (`Cents`) so ledger math is exact.

pub mod business;
pub mod contract;
pub mod ledger;
pub mod license;
pub mod save;
pub mod species;
pub mod store;

pub use business::{Business, EconError, ItemKind, OwnedItem};
pub use contract::{expected_night_value, Contract, ContractBoard, ContractStatus, WeatherBonus};
pub use ledger::{KillRecord, NightLedger, PnLStatement};
pub use license::{License, RepRequirement};
pub use species::Species;
pub use store::{Accessory, OpticModel, OpticSpec, RifleModel};

/// Money in integer cents. $1 = 100.
pub type Cents = i64;

/// Camp fee charged every night, hunted or skipped (SDD §7.5, FR-WX3).
pub const CAMP_FEE_CENTS: Cents = 1_500;
/// Battery wear per hour of optic use, before the optic's battery
/// multiplier (SDD §7.5).
pub const BATTERY_WEAR_CENTS_PER_HR: Cents = 200;
/// Cash fine for hitting a friendly animal (SDD §7.5 example: cat −$150).
pub const FRIENDLY_HIT_FINE_CENTS: Cents = 15_000;
/// Reputation lost with the night's client per friendly hit.
pub const FRIENDLY_HIT_REP_PENALTY: i32 = 15;
/// Reputation lost with the night's client per poached (unlicensed) kill.
/// Poaching pays $0 (SDD §7.4).
pub const POACHING_REP_PENALTY: i32 = 10;
/// Equipment sells back at this percent of what was paid (FR-B6).
pub const SELLBACK_RATE_PCT: Cents = 60;
/// Starting investment (SDD §7.1).
pub const STARTING_CASH_CENTS: Cents = 120_000;
/// Reserved client name for the town (License D reputation gate).
pub const TOWN_CLIENT: &str = "Town";
/// Pellets per tin (SDD §7.1: "Pellet tin (500)").
pub const PELLET_TIN_CAPACITY: u32 = 500;

/// Format cents as dollars: `61200 → "$612"`, `-4300 → "-$43"`,
/// `3150 → "$31.50"`.
pub fn fmt_dollars(cents: Cents) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let a = cents.abs();
    if a % 100 == 0 {
        format!("{sign}${}", a / 100)
    } else {
        format!("{sign}${}.{:02}", a / 100, a % 100)
    }
}

/// Like [`fmt_dollars`] but positive values carry an explicit `+`
/// (P&L statement style: `+$138`).
pub fn fmt_signed(cents: Cents) -> String {
    if cents > 0 {
        format!("+{}", fmt_dollars(cents))
    } else {
        fmt_dollars(cents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn money_formatting() {
        assert_eq!(fmt_dollars(61_200), "$612");
        assert_eq!(fmt_dollars(-4_300), "-$43");
        assert_eq!(fmt_dollars(3_150), "$31.50");
        assert_eq!(fmt_dollars(0), "$0");
        assert_eq!(fmt_signed(13_800), "+$138");
        assert_eq!(fmt_signed(-15_000), "-$150");
        assert_eq!(fmt_signed(0), "$0");
    }
}
