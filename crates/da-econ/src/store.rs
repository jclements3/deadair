//! Store catalogs: rifle ladder (SDD §7.2), optics ladder (§7.3),
//! efficiency upgrades (§7.6). Pure data — purchase logic lives in
//! [`crate::business`].

use crate::Cents;
use serde::{Deserialize, Serialize};

/// Price of the Tier 2 → Tier 3 regulator retrofit (SDD §7.2).
pub const REGULATOR_RETROFIT_CENTS: Cents = 30_000;

/// Rifle models on the store shelf, including the regulated Tier 2
/// variant that exists purely as a buying trap (SDD §7.2).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum RifleModel {
    /// Tier 1 — Multi-pump .22. $180. Rats/possums only.
    MultiPump,
    /// Tier 2 — Unregulated PCP .22. $450. Retrofit-eligible.
    UnregulatedPcp,
    /// Tier 2 — factory-regulated PCP variant. $600. Shoots like the
    /// unregulated Tier 2 but its regulator is NOT the Tier 3 unit, so
    /// retrofitting it later still costs the full $300 — the trap.
    RegulatedTier2Variant,
    /// Tier 3 — Regulated PCP .22. $850 outright, or Tier 2 + $300 retrofit.
    RegulatedPcp,
    /// Tier 4 — Premium PCP .25. $1,900. Unlocks License D targets.
    Premium25,
}

impl RifleModel {
    /// Shelf order.
    pub const ALL: [RifleModel; 5] = [
        RifleModel::MultiPump,
        RifleModel::UnregulatedPcp,
        RifleModel::RegulatedTier2Variant,
        RifleModel::RegulatedPcp,
        RifleModel::Premium25,
    ];

    /// Sticker price.
    pub fn price_cents(self) -> Cents {
        match self {
            RifleModel::MultiPump => 18_000,
            RifleModel::UnregulatedPcp => 45_000,
            RifleModel::RegulatedTier2Variant => 60_000,
            RifleModel::RegulatedPcp => 85_000,
            RifleModel::Premium25 => 190_000,
        }
    }

    /// Gameplay tier (license gating, moderator mount, maintenance rate).
    pub fn tier(self) -> u8 {
        match self {
            RifleModel::MultiPump => 1,
            RifleModel::UnregulatedPcp | RifleModel::RegulatedTier2Variant => 2,
            RifleModel::RegulatedPcp => 3,
            RifleModel::Premium25 => 4,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            RifleModel::MultiPump => "Multi-pump .22",
            RifleModel::UnregulatedPcp => "Unregulated PCP .22",
            RifleModel::RegulatedTier2Variant => "Regulated PCP .22 (Tier 2 variant)",
            RifleModel::RegulatedPcp => "Regulated PCP .22",
            RifleModel::Premium25 => "Premium PCP .25",
        }
    }

    /// Whether the $300 regulator retrofit can be applied, turning this
    /// rifle into the Tier 3 [`RifleModel::RegulatedPcp`].
    pub fn retrofit_eligible(self) -> bool {
        matches!(
            self,
            RifleModel::UnregulatedPcp | RifleModel::RegulatedTier2Variant
        )
    }

    /// Store tooltip warning, if any. The regulated Tier 2 variant carries
    /// the retrofit-trap warning from SDD §7.2.
    pub fn warning(self) -> Option<&'static str> {
        match self {
            RifleModel::RegulatedTier2Variant => Some(
                "Trap: this factory regulator is not the Tier 3 unit. $600 now + $300 \
                 re-regulation later = $900 — more than the $850 Tier 3 outright. If you \
                 plan to retrofit, buy the unregulated Tier 2 ($450 + $300 = $750).",
            ),
            _ => None,
        }
    }

    /// Maintenance accrual per shot. Highest on Tier 1 — the pump linkage
    /// wears with every stroke (SDD §7.5).
    pub fn maintenance_per_shot_cents(self) -> Cents {
        match self.tier() {
            1 => 25,
            2 => 12,
            3 => 8,
            _ => 10,
        }
    }
}

/// Optic spec block surfaced in store tooltips (SDD §7.3): players shop
/// the way real thermal buyers do — resolution, NETD-style mK sensitivity,
/// refresh rate, detection vs. identification range, battery multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OpticSpec {
    /// Sensor resolution (thermal/digital NV). `None` for the headlamp.
    pub resolution: Option<(u32, u32)>,
    /// NETD-style sensitivity in mK; lower is better. Thermal only.
    pub sensitivity_mk: Option<f32>,
    /// Refresh rate in Hz (0 for the headlamp).
    pub refresh_hz: u32,
    /// Detection range in yards — "something warm is there".
    pub detect_yd: f32,
    /// Identification range in yards — "it is a raccoon, not a cat".
    pub id_yd: f32,
    /// Battery drain multiplier applied to the $2/hr wear rate.
    pub battery_mult: f32,
}

/// Optics ladder (SDD §7.3).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum OpticModel {
    /// Headlamp with red filter. $25. The free "optic".
    Headlamp,
    /// Digital NV Gen-basic. $220.
    NvBasic,
    /// Digital NV Pro. $480.
    NvPro,
    /// Thermal 256 "Mk I". $550.
    ThermalMk1,
    /// Thermal 384 "Mk II". $600 upgrade from Mk I / $1,100 outright.
    ThermalMk2,
    /// Thermal 640 "Mk III". $1,600 upgrade from Mk II (no outright SKU).
    ThermalMk3,
}

impl OpticModel {
    /// Shelf order.
    pub const ALL: [OpticModel; 6] = [
        OpticModel::Headlamp,
        OpticModel::NvBasic,
        OpticModel::NvPro,
        OpticModel::ThermalMk1,
        OpticModel::ThermalMk2,
        OpticModel::ThermalMk3,
    ];

    /// Outright purchase price. `None` where the model is only sold as an
    /// upgrade (Mk III).
    pub fn price_outright_cents(self) -> Option<Cents> {
        match self {
            OpticModel::Headlamp => Some(2_500),
            OpticModel::NvBasic => Some(22_000),
            OpticModel::NvPro => Some(48_000),
            OpticModel::ThermalMk1 => Some(55_000),
            OpticModel::ThermalMk2 => Some(110_000),
            OpticModel::ThermalMk3 => None,
        }
    }

    /// Upgrade path: `(required_owned_model, upgrade_price)`.
    pub fn upgrade_from(self) -> Option<(OpticModel, Cents)> {
        match self {
            OpticModel::ThermalMk2 => Some((OpticModel::ThermalMk1, 60_000)),
            OpticModel::ThermalMk3 => Some((OpticModel::ThermalMk2, 160_000)),
            _ => None,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            OpticModel::Headlamp => "Headlamp (red filter)",
            OpticModel::NvBasic => "Digital NV Gen-basic",
            OpticModel::NvPro => "Digital NV Pro",
            OpticModel::ThermalMk1 => "Thermal 256 \"Mk I\"",
            OpticModel::ThermalMk2 => "Thermal 384 \"Mk II\"",
            OpticModel::ThermalMk3 => "Thermal 640 \"Mk III\"",
        }
    }

    /// Spec block for the store tooltip (SDD §7.3 table).
    pub fn spec(self) -> OpticSpec {
        match self {
            OpticModel::Headlamp => OpticSpec {
                resolution: None,
                sensitivity_mk: None,
                refresh_hz: 0,
                detect_yd: 25.0,
                id_yd: 25.0,
                battery_mult: 0.25,
            },
            OpticModel::NvBasic => OpticSpec {
                resolution: Some((1280, 720)),
                sensitivity_mk: None,
                refresh_hz: 50,
                detect_yd: 60.0,
                id_yd: 40.0,
                battery_mult: 1.0,
            },
            OpticModel::NvPro => OpticSpec {
                resolution: Some((1920, 1080)),
                sensitivity_mk: None,
                refresh_hz: 50,
                detect_yd: 120.0,
                id_yd: 80.0,
                battery_mult: 1.0,
            },
            OpticModel::ThermalMk1 => OpticSpec {
                resolution: Some((256, 192)),
                sensitivity_mk: Some(40.0),
                refresh_hz: 25,
                detect_yd: 150.0,
                id_yd: 50.0,
                battery_mult: 2.5,
            },
            OpticModel::ThermalMk2 => OpticSpec {
                resolution: Some((384, 288)),
                sensitivity_mk: Some(25.0),
                refresh_hz: 50,
                detect_yd: 300.0,
                id_yd: 100.0,
                battery_mult: 2.5,
            },
            OpticModel::ThermalMk3 => OpticSpec {
                resolution: Some((640, 480)),
                sensitivity_mk: Some(18.0),
                refresh_hz: 50,
                detect_yd: 400.0,
                id_yd: 200.0,
                battery_mult: 3.0,
            },
        }
    }

    /// Rendered store tooltip: name plus the spec language real buyers use.
    pub fn tooltip(self) -> String {
        let s = self.spec();
        let mut t = String::from(self.name());
        if let Some((w, h)) = s.resolution {
            t.push_str(&format!(" — {w}\u{d7}{h}"));
        }
        if let Some(mk) = s.sensitivity_mk {
            t.push_str(&format!(", {mk:.0} mK NETD"));
        }
        if s.refresh_hz > 0 {
            t.push_str(&format!(", {} Hz", s.refresh_hz));
        }
        t.push_str(&format!(
            ". Detect {:.0} yd / ID {:.0} yd. Battery {}x.",
            s.detect_yd, s.id_yd, s.battery_mult
        ));
        t
    }
}

/// Efficiency upgrades and consumables (SDD §7.1, §7.6).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum Accessory {
    /// Moderator. $200. Requires a Tier 3+ rifle (moderator mount).
    Moderator,
    /// Higher-capacity battery pack. $150.
    BatteryPack,
    /// Larger air tank. $250.
    LargerTank,
    /// Bicycle (travel speed). $300.
    Bicycle,
    /// Matched pellets, tin of 500. $30. Accuracy bonus per rifle tier.
    MatchedPelletTin,
    /// Scope magnification upgrade. $120.
    ScopeMagnification,
    /// Basic pellet tin (500). $18.
    PelletTin,
    /// Basic 3-9x scope. $60 (starting kit).
    BasicScope,
    /// Headlamp red filter. $25 (starting kit).
    RedFilter,
}

impl Accessory {
    /// Shelf order.
    pub const ALL: [Accessory; 9] = [
        Accessory::Moderator,
        Accessory::BatteryPack,
        Accessory::LargerTank,
        Accessory::Bicycle,
        Accessory::MatchedPelletTin,
        Accessory::ScopeMagnification,
        Accessory::PelletTin,
        Accessory::BasicScope,
        Accessory::RedFilter,
    ];

    /// Sticker price.
    pub fn price_cents(self) -> Cents {
        match self {
            Accessory::Moderator => 20_000,
            Accessory::BatteryPack => 15_000,
            Accessory::LargerTank => 25_000,
            Accessory::Bicycle => 30_000,
            Accessory::MatchedPelletTin => 3_000,
            Accessory::ScopeMagnification => 12_000,
            Accessory::PelletTin => 1_800,
            Accessory::BasicScope => 6_000,
            Accessory::RedFilter => 2_500,
        }
    }

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            Accessory::Moderator => "Moderator",
            Accessory::BatteryPack => "Battery pack",
            Accessory::LargerTank => "Larger air tank",
            Accessory::Bicycle => "Bicycle",
            Accessory::MatchedPelletTin => "Matched pellets (tin of 500)",
            Accessory::ScopeMagnification => "Scope magnification upgrade",
            Accessory::PelletTin => "Pellet tin (500)",
            Accessory::BasicScope => "Basic 3-9x scope",
            Accessory::RedFilter => "Headlamp red filter",
        }
    }

    /// Minimum owned rifle tier required to buy (moderator mount is Tier 3+).
    pub fn requires_rifle_tier(self) -> Option<u8> {
        match self {
            Accessory::Moderator => Some(3),
            _ => None,
        }
    }

    /// Consumables (pellet tins) are used up and have no sell-back value.
    pub fn is_consumable(self) -> bool {
        matches!(self, Accessory::PelletTin | Accessory::MatchedPelletTin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rifle_ladder_prices_match_sdd_7_2() {
        assert_eq!(RifleModel::MultiPump.price_cents(), 18_000);
        assert_eq!(RifleModel::UnregulatedPcp.price_cents(), 45_000);
        assert_eq!(RifleModel::RegulatedPcp.price_cents(), 85_000);
        assert_eq!(RifleModel::Premium25.price_cents(), 190_000);
        assert_eq!(REGULATOR_RETROFIT_CENTS, 30_000);
    }

    #[test]
    fn regulated_variant_is_the_only_warned_rifle() {
        for r in RifleModel::ALL {
            assert_eq!(
                r.warning().is_some(),
                r == RifleModel::RegulatedTier2Variant
            );
        }
    }

    #[test]
    fn tier1_has_highest_maintenance() {
        let t1 = RifleModel::MultiPump.maintenance_per_shot_cents();
        for r in RifleModel::ALL {
            assert!(r.maintenance_per_shot_cents() <= t1);
        }
    }

    #[test]
    fn optic_specs_match_sdd_7_3() {
        let mk1 = OpticModel::ThermalMk1.spec();
        assert_eq!(mk1.resolution, Some((256, 192)));
        assert_eq!(mk1.sensitivity_mk, Some(40.0));
        assert_eq!(mk1.refresh_hz, 25);
        assert_eq!(mk1.battery_mult, 2.5);

        let mk2 = OpticModel::ThermalMk2.spec();
        assert_eq!(mk2.resolution, Some((384, 288)));
        assert_eq!(
            OpticModel::ThermalMk2.upgrade_from(),
            Some((OpticModel::ThermalMk1, 60_000))
        );
        assert_eq!(OpticModel::ThermalMk2.price_outright_cents(), Some(110_000));

        let mk3 = OpticModel::ThermalMk3.spec();
        assert!(mk3.sensitivity_mk.unwrap() < 20.0);
        assert_eq!(mk3.battery_mult, 3.0);
        assert_eq!(OpticModel::ThermalMk3.price_outright_cents(), None);
        assert_eq!(
            OpticModel::ThermalMk3.upgrade_from(),
            Some((OpticModel::ThermalMk2, 160_000))
        );
    }

    #[test]
    fn tooltips_surface_spec_language() {
        let t = OpticModel::ThermalMk2.tooltip();
        assert!(t.contains("384"));
        assert!(t.contains("25 mK"));
        assert!(t.contains("50 Hz"));
        assert!(t.contains("300 yd"));
    }

    #[test]
    fn accessory_prices_match_sdd() {
        assert_eq!(Accessory::Moderator.price_cents(), 20_000);
        assert_eq!(Accessory::BatteryPack.price_cents(), 15_000);
        assert_eq!(Accessory::LargerTank.price_cents(), 25_000);
        assert_eq!(Accessory::Bicycle.price_cents(), 30_000);
        assert_eq!(Accessory::MatchedPelletTin.price_cents(), 3_000);
        assert_eq!(Accessory::ScopeMagnification.price_cents(), 12_000);
        assert_eq!(Accessory::PelletTin.price_cents(), 1_800);
        assert_eq!(Accessory::BasicScope.price_cents(), 6_000);
        assert_eq!(Accessory::RedFilter.price_cents(), 2_500);
        assert_eq!(Accessory::Moderator.requires_rifle_tier(), Some(3));
    }
}
