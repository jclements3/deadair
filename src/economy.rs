//! Business / P&L system.
//!
//! Running a night-hunting operation has real fixed and variable costs.
//! This module tracks equipment depreciation, consumables, permits, and
//! zombie-eradication bounties, and produces a formatted P&L statement.

use serde::{Deserialize, Serialize};

/// A piece of hunting equipment with purchase cost and expected service life.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Equipment {
    pub name: String,
    /// Purchase price in USD.
    pub purchase_price: f64,
    /// Expected service life in hunts (used for straight-line depreciation).
    pub service_life_hunts: u32,
}

impl Equipment {
    /// Depreciation charged to a single hunt.
    pub fn cost_per_hunt(&self) -> f64 {
        self.purchase_price / self.service_life_hunts as f64
    }
}

/// Pre-built equipment catalogue with realistic 2024 prices.
pub struct Catalogue;

impl Catalogue {
    pub fn thermal_scope_budget() -> Equipment {
        Equipment {
            name: "Budget Thermal Scope (80 mK NETD)".into(),
            purchase_price: 1_200.0,
            service_life_hunts: 200,
        }
    }

    pub fn thermal_scope_mil() -> Equipment {
        Equipment {
            name: "Military-Grade Thermal Scope (25 mK NETD)".into(),
            purchase_price: 8_500.0,
            service_life_hunts: 500,
        }
    }

    pub fn rifle_bolt_action() -> Equipment {
        Equipment {
            name: ".308 Bolt-Action Rifle".into(),
            purchase_price: 900.0,
            service_life_hunts: 1_000,
        }
    }
}

/// A single line item in the P&L ledger.
///
/// Positive `amount` = revenue; negative `amount` = expense.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineItem {
    pub description: String,
    pub amount: f64,
}

impl LineItem {
    pub fn revenue(description: impl Into<String>, amount: f64) -> Self {
        Self { description: description.into(), amount: amount.abs() }
    }

    pub fn expense(description: impl Into<String>, amount: f64) -> Self {
        Self { description: description.into(), amount: -amount.abs() }
    }
}

/// Profit-and-Loss ledger for one or many hunts.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ProfitLossLedger {
    pub entries: Vec<LineItem>,
}

impl ProfitLossLedger {
    pub fn new() -> Self { Self::default() }

    pub fn add(&mut self, item: LineItem) { self.entries.push(item); }

    /// Sum of all revenue entries.
    pub fn revenue(&self) -> f64 {
        self.entries.iter().filter(|e| e.amount > 0.0).map(|e| e.amount).sum()
    }

    /// Sum of all expense entries (returned as a positive number).
    pub fn expenses(&self) -> f64 {
        self.entries.iter().filter(|e| e.amount < 0.0).map(|e| e.amount.abs()).sum()
    }

    /// Net profit (positive) or loss (negative).
    pub fn net(&self) -> f64 { self.revenue() - self.expenses() }

    pub fn is_profitable(&self) -> bool { self.net() > 0.0 }

    /// Render a formatted P&L statement.
    pub fn report(&self) -> String {
        let line = "═".repeat(50);
        let mut out = String::new();
        out.push_str(&format!("╔{}╗\n", line));
        out.push_str(&format!("║{:^50}║\n", "HUNT  P&L  STATEMENT"));
        out.push_str(&format!("╠{}╣\n", line));

        out.push_str(&format!("║{:^50}║\n", "REVENUE"));
        for e in self.entries.iter().filter(|e| e.amount >= 0.0) {
            out.push_str(&format!("║  {:<36}  {:>8.2} ║\n", e.description, e.amount));
        }
        out.push_str(&format!("║  {:<36}  {:>8.2} ║\n", "Total Revenue", self.revenue()));

        out.push_str(&format!("╠{}╣\n", line));
        out.push_str(&format!("║{:^50}║\n", "EXPENSES"));
        for e in self.entries.iter().filter(|e| e.amount < 0.0) {
            out.push_str(&format!("║  {:<36} ({:>8.2})║\n", e.description, e.amount.abs()));
        }
        out.push_str(&format!("║  {:<36} ({:>8.2})║\n", "Total Expenses", self.expenses()));

        out.push_str(&format!("╠{}╣\n", line));
        let net = self.net();
        if net >= 0.0 {
            out.push_str(&format!("║  {:<36}  {:>8.2} ║\n", "NET PROFIT", net));
        } else {
            out.push_str(&format!("║  {:<36} ({:>8.2})║\n", "NET LOSS", net.abs()));
        }
        out.push_str(&format!("╚{}╝\n", line));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profitable_hunt_is_detected() {
        let mut ledger = ProfitLossLedger::new();
        ledger.add(LineItem::revenue("Zombie bounty ×10", 2_500.0));
        ledger.add(LineItem::expense("Permit", 75.0));
        ledger.add(LineItem::expense("Ammo", 30.0));
        assert!(ledger.is_profitable());
        assert!((ledger.net() - 2_395.0).abs() < 0.01);
    }

    #[test]
    fn zero_kills_is_a_loss() {
        let mut ledger = ProfitLossLedger::new();
        ledger.add(LineItem::expense("Permit", 75.0));
        ledger.add(LineItem::expense("Scope depreciation", 6.0));
        assert!(!ledger.is_profitable());
        assert!(ledger.net() < 0.0);
    }

    #[test]
    fn equipment_depreciation_is_sane() {
        let scope = Catalogue::thermal_scope_budget();
        // $1200 / 200 hunts = $6/hunt
        assert!((scope.cost_per_hunt() - 6.0).abs() < 0.001);
    }
}
