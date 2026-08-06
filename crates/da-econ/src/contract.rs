//! Contracts and the contract board (SRS FR-E2, FR-WX4) plus the
//! expected-value helper for the camp forecast panel (SDD §7A, FR-WX3).

use crate::{business::Business, species::Species, CAMP_FEE_CENTS};
use da_core::Forecast;
use serde::{Deserialize, Serialize};

/// Weather bonus rider on a contract (FR-WX4): kills of the contract
/// species pay `multiplier`× bounty on nights with the matching forecast
/// (e.g. rat surge in the grain co-op just before a storm).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeatherBonus {
    /// Forecast that triggers the bonus.
    pub forecast: Forecast,
    /// Bounty multiplier while triggered (e.g. 1.5).
    pub multiplier: f32,
}

/// Lifecycle of a contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractStatus {
    /// On the board, not yet taken.
    Offered,
    /// Accepted by the player; deadline is ticking.
    Accepted,
    /// Quota met.
    Completed,
    /// Deadline expired before the quota was met.
    Failed,
    /// Pulled because the player's reputation dropped below the requirement.
    Cancelled,
}

/// A pest-control contract (FR-E2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Contract {
    /// Unique id.
    pub id: u32,
    /// Client name (a farm, or [`crate::TOWN_CLIENT`]).
    pub client: String,
    /// Zone the contract covers.
    pub zone: String,
    /// Target species.
    pub species: Species,
    /// Confirmed kills required.
    pub quota: u32,
    /// Confirmed kills so far.
    pub progress: u32,
    /// Nights remaining. Every settled or skipped night burns one (FR-WX3).
    pub deadline_nights: u32,
    /// Minimum reputation with `client` to accept — and to keep — the job.
    pub rep_required: i32,
    /// Optional weather bonus rider (FR-WX4).
    pub bounty_bonus: Option<WeatherBonus>,
    /// Current lifecycle state.
    pub status: ContractStatus,
}

impl Contract {
    /// New offered contract with zero progress.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: u32,
        client: &str,
        zone: &str,
        species: Species,
        quota: u32,
        deadline_nights: u32,
        rep_required: i32,
        bounty_bonus: Option<WeatherBonus>,
    ) -> Self {
        Self {
            id,
            client: client.to_string(),
            zone: zone.to_string(),
            species,
            quota,
            progress: 0,
            deadline_nights,
            rep_required,
            bounty_bonus,
            status: ContractStatus::Offered,
        }
    }

    /// Kills still needed.
    pub fn remaining(&self) -> u32 {
        self.quota.saturating_sub(self.progress)
    }
}

/// The camp contract board. Listings are gated by reputation and license
/// (no point advertising raccoon work to a License A operator).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContractBoard {
    /// All posted contracts.
    pub contracts: Vec<Contract>,
}

impl ContractBoard {
    /// Contracts this player can actually see and take: still offered,
    /// reputation at or above the requirement, and species covered by an
    /// owned license.
    pub fn visible<'a>(&'a self, business: &Business) -> Vec<&'a Contract> {
        self.contracts
            .iter()
            .filter(|c| {
                c.status == ContractStatus::Offered
                    && business.rep(&c.client) >= c.rep_required
                    && business.can_hunt(c.species)
            })
            .collect()
    }

    /// Remove a contract from the board by id (e.g. after acceptance).
    pub fn take(&mut self, id: u32) -> Option<Contract> {
        let idx = self.contracts.iter().position(|c| c.id == id)?;
        Some(self.contracts.remove(idx))
    }
}

/// Expected dollar value of hunting tonight under `forecast` against
/// `contract` — the "should I skip?" number on the camp forecast panel
/// (SDD §7A, FR-WX3). Compare against `-$15` (the cost of skipping).
///
/// Model: encounters scale with the forecast's `pest_activity` modifier;
/// each expected kill pays the species bounty (times the contract's
/// weather-bonus multiplier when the forecast matches) and costs a little
/// in pellets/maintenance; the camp fee is paid regardless.
pub fn expected_night_value(forecast: Forecast, contract: &Contract) -> f32 {
    /// Baseline encounters per night at pest_activity = 1.0.
    const BASE_ENCOUNTERS: f32 = 8.0;
    /// Estimated variable cost (pellets, maintenance, battery) per kill.
    const VARIABLE_COST_PER_KILL: f32 = 0.50;

    let mods = forecast.mods();
    let bounty = contract.species.bounty_cents().unwrap_or(0) as f32 / 100.0;
    let mult = match contract.bounty_bonus {
        Some(b) if b.forecast == forecast => b.multiplier,
        _ => 1.0,
    };
    let expected_kills = BASE_ENCOUNTERS * mods.pest_activity;
    expected_kills * bounty * mult
        - expected_kills * VARIABLE_COST_PER_KILL
        - CAMP_FEE_CENTS as f32 / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rat_contract() -> Contract {
        Contract::new(1, "Grain Co-op", "co-op silos", Species::Rat, 10, 5, 0, None)
    }

    #[test]
    fn ev_ranks_prestorm_over_clear_over_rain() {
        let c = rat_contract();
        let pre = expected_night_value(Forecast::PreStorm, &c);
        let clear = expected_night_value(Forecast::Clear, &c);
        let rain = expected_night_value(Forecast::Rain, &c);
        assert!(pre > clear, "PreStorm {pre} should beat Clear {clear}");
        assert!(clear > rain, "Clear {clear} should beat Rain {rain}");
    }

    #[test]
    fn weather_bonus_only_applies_on_matching_forecast() {
        let mut c = rat_contract();
        let base = expected_night_value(Forecast::PreStorm, &c);
        c.bounty_bonus = Some(WeatherBonus {
            forecast: Forecast::PreStorm,
            multiplier: 1.5,
        });
        let boosted = expected_night_value(Forecast::PreStorm, &c);
        let clear = expected_night_value(Forecast::Clear, &c);
        assert!(boosted > base);
        // Bonus must not leak into non-matching forecasts.
        assert_eq!(
            clear,
            expected_night_value(Forecast::Clear, &rat_contract())
        );
    }
}
