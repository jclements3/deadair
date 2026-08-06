//! License / commission ladder (SDD §7.4, SRS FR-B3).

use crate::Cents;
use serde::{Deserialize, Serialize};

/// Reputation gate attached to a license purchase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepRequirement {
    /// No reputation requirement.
    None,
    /// Reputation strictly above the threshold with **any farm client**
    /// (any client except the town).
    AnyFarmAbove(i32),
    /// Reputation strictly above the threshold with the town.
    TownAbove(i32),
}

/// The four licenses. A is included with the starting investment; the
/// others are bought with cash and gated by reputation and rifle tier.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum License {
    /// Starter license: rats and rabbits. Included.
    A,
    /// + possums. $150.
    B,
    /// + raccoons. $400, rep > 50 with any farm, Tier 2+ rifle.
    C,
    /// Municipal commission: + groundhog, beaver, juvenile feral hog.
    /// $900, town rep > 80, Tier 4 rifle.
    D,
}

impl License {
    /// All licenses in ladder order.
    pub const ALL: [License; 4] = [License::A, License::B, License::C, License::D];

    /// Purchase price (License A is included).
    pub fn price_cents(self) -> Cents {
        match self {
            License::A => 0,
            License::B => 15_000,
            License::C => 40_000,
            License::D => 90_000,
        }
    }

    /// Minimum owned rifle tier required to purchase (0 = no requirement).
    pub fn min_rifle_tier(self) -> u8 {
        match self {
            License::A | License::B => 0,
            License::C => 2,
            License::D => 4,
        }
    }

    /// Reputation gate for purchase.
    pub fn rep_requirement(self) -> RepRequirement {
        match self {
            License::A | License::B => RepRequirement::None,
            License::C => RepRequirement::AnyFarmAbove(50),
            License::D => RepRequirement::TownAbove(80),
        }
    }

    /// Store tooltip line.
    pub fn tooltip(self) -> &'static str {
        match self {
            License::A => {
                "License A — rats ($8/head) and rabbits ($15). Included with your starting \
                 investment."
            }
            License::B => "License B — adds possums ($25). $150.",
            License::C => {
                "License C — adds raccoons ($60). $400. Requires >50 reputation with a farm \
                 and a Tier 2+ rifle."
            }
            License::D => {
                "License D — municipal commission: groundhog ($90), beaver ($140), juvenile \
                 feral hog ($200). $900. Requires >80 town reputation and the Tier 4 rifle."
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prices_match_sdd_7_4() {
        assert_eq!(License::A.price_cents(), 0);
        assert_eq!(License::B.price_cents(), 15_000);
        assert_eq!(License::C.price_cents(), 40_000);
        assert_eq!(License::D.price_cents(), 90_000);
    }

    #[test]
    fn gating_requirements() {
        assert_eq!(License::C.min_rifle_tier(), 2);
        assert_eq!(License::D.min_rifle_tier(), 4);
        assert_eq!(License::C.rep_requirement(), RepRequirement::AnyFarmAbove(50));
        assert_eq!(License::D.rep_requirement(), RepRequirement::TownAbove(80));
    }
}
