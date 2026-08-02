//! Integration tests for the economy / P&L system.

use deadair::economy::{Catalogue, LineItem, ProfitLossLedger};

#[test]
fn profitable_hunt_is_identified() {
    let mut ledger = ProfitLossLedger::new();
    // 5 kills × $250 = $1,250 revenue
    ledger.add(LineItem::revenue("Bounty ×5", 1_250.0));
    // Costs
    ledger.add(LineItem::expense("Permit", 75.0));
    ledger.add(LineItem::expense("Ammo ×12", 14.40));
    ledger.add(LineItem::expense("Scope depreciation", 6.0));
    ledger.add(LineItem::expense("Rifle depreciation", 0.9));

    assert!(ledger.is_profitable());
    let expected_net = 1_250.0 - 75.0 - 14.40 - 6.0 - 0.9;
    assert!((ledger.net() - expected_net).abs() < 0.001);
}

#[test]
fn no_kills_means_a_loss() {
    let mut ledger = ProfitLossLedger::new();
    ledger.add(LineItem::expense("Permit", 75.0));
    ledger.add(LineItem::expense("Scope depreciation", 6.0));
    assert!(!ledger.is_profitable());
    assert!(ledger.net() < 0.0);
}

#[test]
fn revenue_and_expenses_are_calculated_correctly() {
    let mut ledger = ProfitLossLedger::new();
    ledger.add(LineItem::revenue("Bounty A", 500.0));
    ledger.add(LineItem::revenue("Bounty B", 250.0));
    ledger.add(LineItem::expense("Cost X", 100.0));
    ledger.add(LineItem::expense("Cost Y",  50.0));

    assert!((ledger.revenue() - 750.0).abs() < 0.001);
    assert!((ledger.expenses() - 150.0).abs() < 0.001);
    assert!((ledger.net() - 600.0).abs() < 0.001);
}

#[test]
fn equipment_depreciation_is_correct() {
    let scope = Catalogue::thermal_scope_budget();
    // $1200 / 200 hunts = $6.00 / hunt
    assert!((scope.cost_per_hunt() - 6.0).abs() < 0.001, "{}", scope.cost_per_hunt());

    let mil = Catalogue::thermal_scope_mil();
    // $8500 / 500 hunts = $17.00 / hunt
    assert!((mil.cost_per_hunt() - 17.0).abs() < 0.001, "{}", mil.cost_per_hunt());

    let rifle = Catalogue::rifle_bolt_action();
    // $900 / 1000 hunts = $0.90 / hunt
    assert!((rifle.cost_per_hunt() - 0.9).abs() < 0.001, "{}", rifle.cost_per_hunt());
}

#[test]
fn report_contains_expected_labels() {
    let mut ledger = ProfitLossLedger::new();
    ledger.add(LineItem::revenue("Test bounty", 999.0));
    ledger.add(LineItem::expense("Test cost", 1.0));
    let report = ledger.report();
    assert!(report.contains("REVENUE"), "Report should contain REVENUE section");
    assert!(report.contains("EXPENSES"), "Report should contain EXPENSES section");
    assert!(report.contains("NET PROFIT"), "Report should contain NET PROFIT line");
}

#[test]
fn zero_entry_ledger_has_zero_net() {
    let ledger = ProfitLossLedger::new();
    assert_eq!(ledger.net(), 0.0);
    assert_eq!(ledger.revenue(), 0.0);
    assert_eq!(ledger.expenses(), 0.0);
}
