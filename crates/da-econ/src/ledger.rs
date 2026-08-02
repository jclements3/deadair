//! Nightly ledger and P&L statement (SDD §7.5, SRS FR-B4, FR-E1).
//!
//! During the night the [`NightLedger`] accumulates confirmed kills (paid
//! on return to camp), penalties, and cost drivers (shots, optic hours).
//! [`crate::Business::settle_night`] turns it into a [`PnLStatement`] and
//! applies the cash and reputation consequences.

use crate::{
    business::Business,
    species::Species,
    store::{Accessory, OpticModel, RifleModel},
    Cents, BATTERY_WEAR_CENTS_PER_HR, CAMP_FEE_CENTS, FRIENDLY_HIT_FINE_CENTS,
    PELLET_TIN_CAPACITY,
};
use da_core::Forecast;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Outcome of a recorded kill, decided the instant the shot connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KillRecord {
    /// Licensed bounty species: queued for payment at camp (FR-E1).
    Bounty(Cents),
    /// Bounty species without the license: poaching. $0 and a reputation
    /// penalty at settlement (SDD §7.4).
    Poached,
    /// Friendly animal: fine plus reputation penalty at settlement.
    FriendlyHit,
    /// Legal but worthless (zombies).
    NoBounty,
}

/// Accumulates one night's economic events.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NightLedger {
    /// Night number (for the statement header).
    pub night: u32,
    /// Client whose land was hunted; penalties hit this reputation.
    pub client: String,
    /// The night's forecast (drives contract weather bonuses, FR-WX4).
    pub forecast: Forecast,
    /// Rifle carried (drives maintenance accrual).
    pub rifle: RifleModel,
    /// Optic carried, if any (drives battery wear).
    pub optic: Option<OpticModel>,
    confirmed_kills: Vec<Species>,
    poached: Vec<Species>,
    friendly_hits: Vec<Species>,
    shots_fired: u32,
    optic_hours: f32,
    matched_pellets: bool,
}

impl NightLedger {
    /// Open a ledger for a night on `client`'s land.
    pub fn new(
        night: u32,
        client: &str,
        forecast: Forecast,
        rifle: RifleModel,
        optic: Option<OpticModel>,
    ) -> Self {
        Self {
            night,
            client: client.to_string(),
            forecast,
            rifle,
            optic,
            confirmed_kills: Vec::new(),
            poached: Vec::new(),
            friendly_hits: Vec::new(),
            shots_fired: 0,
            optic_hours: 0.0,
            matched_pellets: false,
        }
    }

    /// Shoot matched pellets tonight ($30/tin instead of $18/tin).
    pub fn use_matched_pellets(&mut self) {
        self.matched_pellets = true;
    }

    /// Record one shot fired (pellet cost + maintenance accrual).
    pub fn record_shot(&mut self) {
        self.shots_fired += 1;
    }

    /// Record several shots at once.
    pub fn record_shots(&mut self, n: u32) {
        self.shots_fired += n;
    }

    /// Accumulate hours of optic use (battery wear at $2/hr × the optic's
    /// battery multiplier).
    pub fn record_optic_hours(&mut self, hours: f32) {
        self.optic_hours += hours;
    }

    /// Record a confirmed kill. Classification against the player's
    /// licenses happens now; money and reputation settle at camp.
    pub fn record_kill(&mut self, species: Species, business: &Business) -> KillRecord {
        let record = business.classify_kill(species);
        match record {
            KillRecord::Bounty(_) => self.confirmed_kills.push(species),
            KillRecord::Poached => self.poached.push(species),
            KillRecord::FriendlyHit => self.friendly_hits.push(species),
            KillRecord::NoBounty => {}
        }
        record
    }

    /// Confirmed, licensed kills queued for bounty payment.
    pub fn confirmed_kills(&self) -> &[Species] {
        &self.confirmed_kills
    }

    /// Unlicensed kills (poaching).
    pub fn poached(&self) -> &[Species] {
        &self.poached
    }

    /// Friendly animals hit.
    pub fn friendly_hits(&self) -> &[Species] {
        &self.friendly_hits
    }

    /// Shots fired tonight.
    pub fn shots_fired(&self) -> u32 {
        self.shots_fired
    }

    /// Hours of optic use tonight.
    pub fn optic_hours(&self) -> f32 {
        self.optic_hours
    }

    /// Total operating costs (SDD §7.5): camp fee, pellets at cost,
    /// battery wear $2/hr × optic multiplier, maintenance accrual per shot.
    pub fn operating_costs_cents(&self) -> Cents {
        let tin_price = if self.matched_pellets {
            Accessory::MatchedPelletTin.price_cents()
        } else {
            Accessory::PelletTin.price_cents()
        };
        let pellets = tin_price * self.shots_fired as Cents / PELLET_TIN_CAPACITY as Cents;
        let battery = self
            .optic
            .map(|o| {
                (self.optic_hours as f64
                    * BATTERY_WEAR_CENTS_PER_HR as f64
                    * o.spec().battery_mult as f64)
                    .round() as Cents
            })
            .unwrap_or(0);
        let maintenance = self.rifle.maintenance_per_shot_cents() * self.shots_fired as Cents;
        CAMP_FEE_CENTS + pellets + battery + maintenance
    }

    /// Friendly-hit fines owed (money side only; reputation settles later).
    pub fn penalty_total_cents(&self) -> Cents {
        FRIENDLY_HIT_FINE_CENTS * self.friendly_hits.len() as Cents
    }

    /// Kill counts per species in declaration (bounty-ladder) order.
    pub fn bounty_counts(&self) -> Vec<(Species, u32)> {
        let mut counts: BTreeMap<Species, u32> = BTreeMap::new();
        for &sp in &self.confirmed_kills {
            *counts.entry(sp).or_insert(0) += 1;
        }
        counts.into_iter().collect()
    }
}

/// End-of-night profit & loss statement (SDD §7.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PnLStatement {
    /// Night number.
    pub night: u32,
    /// Statement title: client name, or "SKIPPED" for a rest night.
    pub title: String,
    /// Confirmed kill counts per species.
    pub bounty_counts: Vec<(Species, u32)>,
    /// Total bounty income (includes contract weather bonuses).
    pub bounties_cents: Cents,
    /// Penalty lines: (label, magnitude in cents). Poaching lines carry 0.
    pub penalties: Vec<(String, Cents)>,
    /// Total operating costs (positive magnitude).
    pub operating_costs_cents: Cents,
    /// Net for the night: bounties − penalties − costs.
    pub net_cents: Cents,
    /// Cash balance after settlement.
    pub balance_after_cents: Cents,
}

impl fmt::Display for PnLStatement {
    /// Renders the SDD §7.5 end-of-night screen:
    ///
    /// ```text
    /// NIGHT 7 — GRAIN CO-OP
    /// Bounties (11 rats, 2 possums)   +$138
    /// Penalty (cat)                   -$150
    /// Operating costs                 -$31
    /// NET                             -$43   Balance: $612
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "NIGHT {} — {}", self.night, self.title.to_uppercase())?;
        if !self.bounty_counts.is_empty() || self.bounties_cents != 0 {
            let list = if self.bounty_counts.is_empty() {
                "none".to_string()
            } else {
                self.bounty_counts
                    .iter()
                    .map(|(sp, n)| {
                        format!("{} {}", n, if *n == 1 { sp.name() } else { sp.plural() })
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            writeln!(
                f,
                "{:<32}{}",
                format!("Bounties ({list})"),
                crate::fmt_signed(self.bounties_cents)
            )?;
        }
        for (label, cents) in &self.penalties {
            writeln!(f, "{:<32}{}", label, crate::fmt_signed(-cents))?;
        }
        writeln!(
            f,
            "{:<32}{}",
            "Operating costs",
            crate::fmt_signed(-self.operating_costs_cents)
        )?;
        write!(
            f,
            "{:<32}{}   Balance: {}",
            "NET",
            crate::fmt_signed(self.net_cents),
            crate::fmt_dollars(self.balance_after_cents)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operating_costs_decompose_per_sdd_rates() {
        // 28 shots on the Tier 1 pump, 4 h of NV-basic use, basic pellets:
        // camp $15 + pellets $1.00 (28 × $18/500, floored) + battery $8
        // + maintenance $7 (28 × 25¢) = $31.
        let mut l = NightLedger::new(
            7,
            "Grain Co-op",
            Forecast::Overcast,
            RifleModel::MultiPump,
            Some(OpticModel::NvBasic),
        );
        l.record_shots(28);
        l.record_optic_hours(4.0);
        assert_eq!(l.operating_costs_cents(), 3_100);
    }

    #[test]
    fn skipping_the_optic_means_no_battery_wear() {
        let mut l = NightLedger::new(
            1,
            "Grain Co-op",
            Forecast::Clear,
            RifleModel::MultiPump,
            None,
        );
        l.record_optic_hours(6.0);
        assert_eq!(l.operating_costs_cents(), CAMP_FEE_CENTS);
    }

    #[test]
    fn thermal_battery_multiplier_applies() {
        let mut a = NightLedger::new(
            1,
            "x",
            Forecast::Overcast,
            RifleModel::RegulatedPcp,
            Some(OpticModel::NvBasic),
        );
        let mut b = a.clone();
        b.optic = Some(OpticModel::ThermalMk1);
        a.record_optic_hours(2.0);
        b.record_optic_hours(2.0);
        // Mk I drains 2.5× the NV rate.
        assert_eq!(
            b.operating_costs_cents() - CAMP_FEE_CENTS,
            (a.operating_costs_cents() - CAMP_FEE_CENTS) * 5 / 2
        );
    }
}
