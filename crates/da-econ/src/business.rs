//! Player business state: cash, equipment, licenses, reputation,
//! contracts, settlement, sell-back and bankruptcy (SRS §3.7, SDD §7).

use crate::{
    contract::{Contract, ContractStatus},
    ledger::{KillRecord, NightLedger, PnLStatement},
    license::{License, RepRequirement},
    species::Species,
    store::{Accessory, OpticModel, RifleModel, REGULATOR_RETROFIT_CENTS},
    Cents, CAMP_FEE_CENTS, FRIENDLY_HIT_FINE_CENTS, FRIENDLY_HIT_REP_PENALTY,
    POACHING_REP_PENALTY, SELLBACK_RATE_PCT, STARTING_CASH_CENTS, TOWN_CLIENT,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Reputation gained with the client when a contract completes.
pub const CONTRACT_COMPLETE_REP_BONUS: i32 = 10;
/// Reputation lost with the client when a contract expires unfinished.
pub const CONTRACT_FAIL_REP_PENALTY: i32 = 5;

/// Errors from purchases, sales, and contract operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconError {
    /// Not enough cash.
    InsufficientFunds {
        /// Price of the attempted purchase.
        need: Cents,
        /// Cash on hand.
        have: Cents,
    },
    /// A non-cash requirement (rifle tier, reputation, upgrade path) failed.
    RequirementNotMet(String),
    /// Item already owned.
    AlreadyOwned,
    /// Item not owned.
    NotOwned,
    /// Item cannot be sold (consumable).
    NotSellable,
}

impl fmt::Display for EconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EconError::InsufficientFunds { need, have } => write!(
                f,
                "insufficient funds: need {}, have {}",
                crate::fmt_dollars(*need),
                crate::fmt_dollars(*have)
            ),
            EconError::RequirementNotMet(msg) => write!(f, "requirement not met: {msg}"),
            EconError::AlreadyOwned => write!(f, "already owned"),
            EconError::NotOwned => write!(f, "not owned"),
            EconError::NotSellable => write!(f, "not sellable"),
        }
    }
}

impl std::error::Error for EconError {}

/// What an owned item is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemKind {
    /// A rifle from the ladder.
    Rifle(RifleModel),
    /// An optic from the ladder.
    Optic(OpticModel),
    /// An accessory or consumable.
    Accessory(Accessory),
}

/// A piece of owned equipment with its cost basis (for sell-back).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedItem {
    /// What it is.
    pub kind: ItemKind,
    /// Total paid, including retrofits/upgrades applied to it.
    pub paid_cents: Cents,
}

impl OwnedItem {
    /// Sell-back value: [`SELLBACK_RATE_PCT`]% of cost basis; consumables
    /// are worthless.
    pub fn sell_value_cents(&self) -> Cents {
        match self.kind {
            ItemKind::Accessory(a) if a.is_consumable() => 0,
            _ => self.paid_cents * SELLBACK_RATE_PCT / 100,
        }
    }
}

/// The player's whole business (FR-B1..B6). Serialize via [`crate::save`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Business {
    /// Cash on hand in cents. May go negative (that's how you go bankrupt).
    pub cash_cents: Cents,
    /// Upcoming night number, starting at 1.
    pub night: u32,
    equipment: Vec<OwnedItem>,
    licenses: BTreeSet<License>,
    reputation: BTreeMap<String, i32>,
    contracts: Vec<Contract>,
}

impl Default for Business {
    fn default() -> Self {
        Self::new()
    }
}

impl Business {
    /// Fresh campaign: $1,200 starting investment (SDD §7.1) and the
    /// included License A.
    pub fn new() -> Self {
        let mut licenses = BTreeSet::new();
        licenses.insert(License::A);
        Self {
            cash_cents: STARTING_CASH_CENTS,
            night: 1,
            equipment: Vec::new(),
            licenses,
            reputation: BTreeMap::new(),
            contracts: Vec::new(),
        }
    }

    // ---------------------------------------------------------------- state

    /// Owned equipment.
    pub fn equipment(&self) -> &[OwnedItem] {
        &self.equipment
    }

    /// Owned licenses.
    pub fn licenses(&self) -> &BTreeSet<License> {
        &self.licenses
    }

    /// All contracts the player has taken (any status).
    pub fn contracts(&self) -> &[Contract] {
        &self.contracts
    }

    /// Mutable access to the contract list, for callers that manage the
    /// board directly (the campaign layer, save migration, tests).
    pub fn contracts_mut(&mut self) -> &mut Vec<Contract> {
        &mut self.contracts
    }

    /// Reputation with a client (0 if never met). Range −100..=100.
    pub fn rep(&self, client: &str) -> i32 {
        self.reputation.get(client).copied().unwrap_or(0)
    }

    /// Adjust reputation with a client, clamped to −100..=100.
    pub fn adjust_rep(&mut self, client: &str, delta: i32) {
        let e = self.reputation.entry(client.to_string()).or_insert(0);
        *e = (*e + delta).clamp(-100, 100);
    }

    /// Highest tier among owned rifles (0 if none).
    pub fn best_rifle_tier(&self) -> u8 {
        self.equipment
            .iter()
            .filter_map(|i| match i.kind {
                ItemKind::Rifle(r) => Some(r.tier()),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// Whether an identical item is owned.
    pub fn owns(&self, kind: ItemKind) -> bool {
        self.equipment.iter().any(|i| i.kind == kind)
    }

    /// Whether a license is held.
    pub fn has_license(&self, license: License) -> bool {
        self.licenses.contains(&license)
    }

    /// Whether the player may legally take this species (FR-B3).
    /// Friendlies are never legal; zombies need no license but this
    /// returns `false` because there is no *hunt* for them (no bounty).
    pub fn can_hunt(&self, species: Species) -> bool {
        species
            .license_required()
            .is_some_and(|l| self.licenses.contains(&l))
    }

    /// Classify a kill against licenses (SDD §7.4): licensed bounty
    /// species pay; unlicensed bounty species are poaching; friendlies
    /// are fined; zombies are free but worthless.
    pub fn classify_kill(&self, species: Species) -> KillRecord {
        if species.is_friendly() {
            KillRecord::FriendlyHit
        } else if let Some(license) = species.license_required() {
            if self.licenses.contains(&license) {
                KillRecord::Bounty(species.bounty_cents().unwrap_or(0))
            } else {
                KillRecord::Poached
            }
        } else {
            KillRecord::NoBounty
        }
    }

    // ------------------------------------------------------------ purchases

    fn pay(&mut self, price: Cents) -> Result<(), EconError> {
        if self.cash_cents < price {
            return Err(EconError::InsufficientFunds {
                need: price,
                have: self.cash_cents,
            });
        }
        self.cash_cents -= price;
        Ok(())
    }

    /// Buy a rifle at sticker price.
    pub fn buy_rifle(&mut self, model: RifleModel) -> Result<(), EconError> {
        if self.owns(ItemKind::Rifle(model)) {
            return Err(EconError::AlreadyOwned);
        }
        self.pay(model.price_cents())?;
        self.equipment.push(OwnedItem {
            kind: ItemKind::Rifle(model),
            paid_cents: model.price_cents(),
        });
        Ok(())
    }

    /// Apply the $300 regulator retrofit to an owned Tier 2 rifle,
    /// converting it into the Tier 3 regulated PCP (SDD §7.2). Works on
    /// both the unregulated Tier 2 ($500 + $300 = $800, the smart path)
    /// and the regulated Tier 2 variant ($700 + $300 = $1,000, the trap).
    pub fn retrofit_regulator(&mut self) -> Result<(), EconError> {
        let idx = self
            .equipment
            .iter()
            .position(|i| matches!(i.kind, ItemKind::Rifle(r) if r.retrofit_eligible()))
            .ok_or(EconError::NotOwned)?;
        self.pay(REGULATOR_RETROFIT_CENTS)?;
        self.equipment[idx].kind = ItemKind::Rifle(RifleModel::RegulatedPcp);
        self.equipment[idx].paid_cents += REGULATOR_RETROFIT_CENTS;
        Ok(())
    }

    /// Buy an optic outright. Fails for models only sold as upgrades.
    pub fn buy_optic(&mut self, model: OpticModel) -> Result<(), EconError> {
        if self.owns(ItemKind::Optic(model)) {
            return Err(EconError::AlreadyOwned);
        }
        let price = model.price_outright_cents().ok_or_else(|| {
            EconError::RequirementNotMet(format!("{} is only sold as an upgrade", model.name()))
        })?;
        self.pay(price)?;
        self.equipment.push(OwnedItem {
            kind: ItemKind::Optic(model),
            paid_cents: price,
        });
        Ok(())
    }

    /// Upgrade an owned optic along the ladder (Mk I → Mk II $1,100,
    /// Mk II → Mk III $2,100). The old unit is traded in: the owned item
    /// becomes the new model and its cost basis grows by the upgrade price.
    pub fn upgrade_optic(&mut self, target: OpticModel) -> Result<(), EconError> {
        let (from, price) = target.upgrade_from().ok_or_else(|| {
            EconError::RequirementNotMet(format!("{} has no upgrade path", target.name()))
        })?;
        let idx = self
            .equipment
            .iter()
            .position(|i| i.kind == ItemKind::Optic(from))
            .ok_or(EconError::NotOwned)?;
        self.pay(price)?;
        self.equipment[idx].kind = ItemKind::Optic(target);
        self.equipment[idx].paid_cents += price;
        Ok(())
    }

    /// Buy an accessory (duplicates allowed for consumable tins only).
    pub fn buy_accessory(&mut self, acc: Accessory) -> Result<(), EconError> {
        if !acc.is_consumable() && self.owns(ItemKind::Accessory(acc)) {
            return Err(EconError::AlreadyOwned);
        }
        if let Some(tier) = acc.requires_rifle_tier() {
            if self.best_rifle_tier() < tier {
                return Err(EconError::RequirementNotMet(format!(
                    "{} requires a Tier {tier}+ rifle",
                    acc.name()
                )));
            }
        }
        self.pay(acc.price_cents())?;
        self.equipment.push(OwnedItem {
            kind: ItemKind::Accessory(acc),
            paid_cents: acc.price_cents(),
        });
        Ok(())
    }

    /// Buy a license, enforcing the cash, rifle-tier, and reputation gates
    /// of SDD §7.4 / FR-B3.
    pub fn buy_license(&mut self, license: License) -> Result<(), EconError> {
        if self.licenses.contains(&license) {
            return Err(EconError::AlreadyOwned);
        }
        let tier = license.min_rifle_tier();
        if self.best_rifle_tier() < tier {
            return Err(EconError::RequirementNotMet(format!(
                "License {license:?} requires a Tier {tier}+ rifle"
            )));
        }
        match license.rep_requirement() {
            RepRequirement::None => {}
            RepRequirement::AnyFarmAbove(t) => {
                let ok = self
                    .reputation
                    .iter()
                    .any(|(client, &rep)| client != TOWN_CLIENT && rep > t);
                if !ok {
                    return Err(EconError::RequirementNotMet(format!(
                        "License {license:?} requires reputation above {t} with a farm"
                    )));
                }
            }
            RepRequirement::TownAbove(t) => {
                if self.rep(TOWN_CLIENT) <= t {
                    return Err(EconError::RequirementNotMet(format!(
                        "License {license:?} requires town reputation above {t}"
                    )));
                }
            }
        }
        self.pay(license.price_cents())?;
        self.licenses.insert(license);
        Ok(())
    }

    // ------------------------------------------------------------ contracts

    /// Accept a contract off the board. Requires the reputation threshold
    /// and a license covering the species.
    pub fn accept_contract(&mut self, mut contract: Contract) -> Result<(), EconError> {
        if self.rep(&contract.client) < contract.rep_required {
            return Err(EconError::RequirementNotMet(format!(
                "contract {} requires {} reputation with {}",
                contract.id, contract.rep_required, contract.client
            )));
        }
        if !self.can_hunt(contract.species) {
            return Err(EconError::RequirementNotMet(format!(
                "no license for {}",
                contract.species.plural()
            )));
        }
        contract.status = ContractStatus::Accepted;
        self.contracts.push(contract);
        Ok(())
    }

    /// Cancel accepted contracts whose client reputation has slipped below
    /// the requirement. Returns cancelled contract ids.
    pub fn enforce_reputation(&mut self) -> Vec<u32> {
        let mut cancelled = Vec::new();
        for c in &mut self.contracts {
            if c.status == ContractStatus::Accepted && self.reputation
                .get(&c.client)
                .copied()
                .unwrap_or(0)
                < c.rep_required
            {
                c.status = ContractStatus::Cancelled;
                cancelled.push(c.id);
            }
        }
        cancelled
    }

    /// Burn one deadline night on every accepted contract; expire those
    /// that hit zero unfinished (rep penalty with the client).
    fn tick_deadlines(&mut self) {
        let mut rep_hits: Vec<String> = Vec::new();
        for c in &mut self.contracts {
            if c.status != ContractStatus::Accepted {
                continue;
            }
            c.deadline_nights = c.deadline_nights.saturating_sub(1);
            if c.deadline_nights == 0 && c.progress < c.quota {
                c.status = ContractStatus::Failed;
                rep_hits.push(c.client.clone());
            }
        }
        for client in rep_hits {
            self.adjust_rep(&client, -CONTRACT_FAIL_REP_PENALTY);
        }
    }

    // ----------------------------------------------------------- settlement

    /// Return to camp and settle the night (FR-E1, FR-B4): pay queued
    /// bounties (with contract weather bonuses), charge penalties and
    /// operating costs, apply reputation deltas, advance contract progress
    /// and deadlines, and cancel contracts that fell below their
    /// reputation floor.
    pub fn settle_night(&mut self, ledger: &NightLedger) -> PnLStatement {
        // Bounties, with weather-bonus multipliers from matching accepted
        // contracts (FR-WX4).
        let mut bounties: Cents = 0;
        for &sp in ledger.confirmed_kills() {
            let base = sp.bounty_cents().unwrap_or(0);
            let mult = self
                .contracts
                .iter()
                .find_map(|c| match c.bounty_bonus {
                    Some(b)
                        if c.status == ContractStatus::Accepted
                            && c.client == ledger.client
                            && c.species == sp
                            && b.forecast == ledger.forecast =>
                    {
                        Some(b.multiplier)
                    }
                    _ => None,
                })
                .unwrap_or(1.0);
            bounties += (base as f64 * mult as f64).round() as Cents;
        }

        // Penalties: friendly-hit fines and poaching reputation damage.
        let mut penalties: Vec<(String, Cents)> = Vec::new();
        for &sp in ledger.friendly_hits() {
            penalties.push((format!("Penalty ({})", sp.name()), FRIENDLY_HIT_FINE_CENTS));
            self.adjust_rep(&ledger.client, -FRIENDLY_HIT_REP_PENALTY);
        }
        for &sp in ledger.poached() {
            penalties.push((format!("Poaching ({})", sp.name()), 0));
            self.adjust_rep(&ledger.client, -POACHING_REP_PENALTY);
        }
        let penalty_total: Cents = penalties.iter().map(|(_, c)| c).sum();

        let costs = ledger.operating_costs_cents();
        let net = bounties - penalty_total - costs;
        self.cash_cents += net;

        // Contract progress: each confirmed kill advances the first
        // matching accepted contract for tonight's client.
        let mut completed_clients: Vec<String> = Vec::new();
        for &sp in ledger.confirmed_kills() {
            if let Some(c) = self.contracts.iter_mut().find(|c| {
                c.status == ContractStatus::Accepted
                    && c.client == ledger.client
                    && c.species == sp
                    && c.progress < c.quota
            }) {
                c.progress += 1;
                if c.progress >= c.quota {
                    c.status = ContractStatus::Completed;
                    completed_clients.push(c.client.clone());
                }
            }
        }
        for client in completed_clients {
            self.adjust_rep(&client, CONTRACT_COMPLETE_REP_BONUS);
        }

        self.tick_deadlines();
        self.enforce_reputation();
        self.night = self.night.max(ledger.night) + 1;

        PnLStatement {
            night: ledger.night,
            title: ledger.client.clone(),
            bounty_counts: ledger.bounty_counts(),
            bounties_cents: bounties,
            penalties,
            operating_costs_cents: costs,
            net_cents: net,
            balance_after_cents: self.cash_cents,
        }
    }

    /// Skip tonight (FR-WX3): pay the camp fee only, burn one deadline
    /// night on every accepted contract.
    pub fn skip_night(&mut self) -> PnLStatement {
        let night = self.night;
        self.cash_cents -= CAMP_FEE_CENTS;
        self.tick_deadlines();
        self.enforce_reputation();
        self.night += 1;
        PnLStatement {
            night,
            title: "skipped".to_string(),
            bounty_counts: Vec::new(),
            bounties_cents: 0,
            penalties: Vec::new(),
            operating_costs_cents: CAMP_FEE_CENTS,
            net_cents: -CAMP_FEE_CENTS,
            balance_after_cents: self.cash_cents,
        }
    }

    // ----------------------------------------------- sell-back / bankruptcy

    /// Total sell-back value of everything owned.
    pub fn sellable_total_cents(&self) -> Cents {
        self.equipment.iter().map(|i| i.sell_value_cents()).sum()
    }

    /// Sell an owned item at depreciated value (FR-B6). Consumables
    /// cannot be sold.
    pub fn sell_equipment(&mut self, index: usize) -> Result<Cents, EconError> {
        let item = *self.equipment.get(index).ok_or(EconError::NotOwned)?;
        let value = item.sell_value_cents();
        if value == 0 {
            return Err(EconError::NotSellable);
        }
        self.equipment.remove(index);
        self.cash_cents += value;
        Ok(value)
    }

    /// Campaign fail state (FR-B6): cash below zero AND nothing left to
    /// sell.
    pub fn is_bankrupt(&self) -> bool {
        self.cash_cents < 0 && self.sellable_total_cents() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_position_matches_sdd_7_1() {
        let mut b = Business::new();
        assert_eq!(b.cash_cents, 120_000);
        assert!(b.has_license(License::A));
        // The forced first purchases.
        b.buy_rifle(RifleModel::MultiPump).unwrap();
        b.buy_accessory(Accessory::BasicScope).unwrap();
        b.buy_accessory(Accessory::RedFilter).unwrap();
        b.buy_accessory(Accessory::PelletTin).unwrap();
        // $1,200 − 200 − 60 − 25 − 18 = $897 working capital (~$900).
        assert_eq!(b.cash_cents, 89_700);
    }

    #[test]
    fn moderator_needs_tier3_mount() {
        let mut b = Business::new();
        b.cash_cents = 500_000;
        b.buy_rifle(RifleModel::UnregulatedPcp).unwrap();
        assert!(matches!(
            b.buy_accessory(Accessory::Moderator),
            Err(EconError::RequirementNotMet(_))
        ));
        b.retrofit_regulator().unwrap();
        b.buy_accessory(Accessory::Moderator).unwrap();
    }

    #[test]
    fn license_purchase_gates() {
        let mut b = Business::new();
        b.cash_cents = 1_000_000;
        b.buy_license(License::B).unwrap();
        // C needs Tier 2+ AND farm rep > 50.
        assert!(b.buy_license(License::C).is_err());
        b.buy_rifle(RifleModel::UnregulatedPcp).unwrap();
        assert!(b.buy_license(License::C).is_err());
        b.adjust_rep("Grain Co-op", 60);
        b.buy_license(License::C).unwrap();
        // D needs Tier 4 AND town rep > 80.
        b.buy_rifle(RifleModel::Premium25).unwrap();
        assert!(b.buy_license(License::D).is_err());
        b.adjust_rep(TOWN_CLIENT, 85);
        b.buy_license(License::D).unwrap();
    }

    #[test]
    fn town_rep_does_not_satisfy_farm_requirement() {
        let mut b = Business::new();
        b.cash_cents = 200_000;
        b.buy_rifle(RifleModel::UnregulatedPcp).unwrap();
        b.adjust_rep(TOWN_CLIENT, 90);
        assert!(matches!(
            b.buy_license(License::C),
            Err(EconError::RequirementNotMet(_))
        ));
    }

    #[test]
    fn sell_back_is_60_percent_and_consumables_are_worthless() {
        let mut b = Business::new();
        b.buy_rifle(RifleModel::MultiPump).unwrap(); // $180
        b.buy_accessory(Accessory::PelletTin).unwrap(); // $18, consumable
        assert_eq!(b.sellable_total_cents(), 20_000 * 60 / 100);
        assert!(matches!(b.sell_equipment(1), Err(EconError::NotSellable)));
        let got = b.sell_equipment(0).unwrap();
        assert_eq!(got, 12_000);
    }

    #[test]
    fn upgraded_optic_sells_on_full_cost_basis() {
        let mut b = Business::new();
        b.cash_cents = 500_000;
        b.buy_optic(OpticModel::ThermalMk1).unwrap(); // $950
        b.upgrade_optic(OpticModel::ThermalMk2).unwrap(); // +$1,100
        assert_eq!(b.equipment()[0].kind, ItemKind::Optic(OpticModel::ThermalMk2));
        assert_eq!(b.equipment()[0].paid_cents, 205_000);
        assert_eq!(b.sellable_total_cents(), 205_000 * 60 / 100);
        // Mk III has no outright SKU.
        assert!(matches!(
            b.buy_optic(OpticModel::ThermalMk3),
            Err(EconError::RequirementNotMet(_))
        ));
        b.upgrade_optic(OpticModel::ThermalMk3).unwrap();
        assert_eq!(b.equipment()[0].paid_cents, 415_000);
    }

    #[test]
    fn zombies_are_legal_but_worthless() {
        let b = Business::new();
        assert!(!b.can_hunt(Species::Zombie));
        assert_eq!(b.classify_kill(Species::Zombie), KillRecord::NoBounty);
    }
}
