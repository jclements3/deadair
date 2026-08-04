//! Species and bounty table (SDD §7.4).

use crate::{license::License, Cents};
use serde::{Deserialize, Serialize};

/// Everything that can end up in front of the reticle.
///
/// Ordering matters cosmetically: bounty species are declared in ascending
/// bounty order so P&L lines read "11 rats, 2 possums" like the SDD example.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub enum Species {
    // Bounty species (SDD §7.4).
    Rat,
    Rabbit,
    Possum,
    Raccoon,
    Groundhog,
    Beaver,
    JuvenileFeralHog,
    // Friendly species — hitting one is a fine plus reputation damage.
    Dog,
    Cat,
    Cow,
    Sheep,
    // No bounty, no license, no fine. The thermal can't see it.
    Zombie,
}

impl Species {
    /// Every species, bounty species first.
    pub const ALL: [Species; 12] = [
        Species::Rat,
        Species::Rabbit,
        Species::Possum,
        Species::Raccoon,
        Species::Groundhog,
        Species::Beaver,
        Species::JuvenileFeralHog,
        Species::Dog,
        Species::Cat,
        Species::Cow,
        Species::Sheep,
        Species::Zombie,
    ];

    /// Bounty per confirmed kill (SDD §7.4). `None` for friendlies and
    /// zombies — nobody pays for either.
    pub fn bounty_cents(self) -> Option<Cents> {
        match self {
            Species::Rat => Some(800),
            Species::Rabbit => Some(1_500),
            Species::Possum => Some(2_500),
            Species::Raccoon => Some(6_000),
            Species::Groundhog => Some(9_000),
            Species::Beaver => Some(14_000),
            Species::JuvenileFeralHog => Some(20_000),
            _ => None,
        }
    }

    /// License required to legally take this species (FR-B3).
    /// `None` for species outside the bounty program.
    pub fn license_required(self) -> Option<License> {
        match self {
            Species::Rat | Species::Rabbit => Some(License::A),
            Species::Possum => Some(License::B),
            Species::Raccoon => Some(License::C),
            Species::Groundhog | Species::Beaver | Species::JuvenileFeralHog => Some(License::D),
            _ => None,
        }
    }

    /// Farm animals and pets: shooting one is a fine plus reputation loss.
    pub fn is_friendly(self) -> bool {
        matches!(
            self,
            Species::Dog | Species::Cat | Species::Cow | Species::Sheep
        )
    }

    /// Lowercase singular display name.
    pub fn name(self) -> &'static str {
        match self {
            Species::Rat => "rat",
            Species::Rabbit => "rabbit",
            Species::Possum => "possum",
            Species::Raccoon => "raccoon",
            Species::Groundhog => "groundhog",
            Species::Beaver => "beaver",
            Species::JuvenileFeralHog => "juvenile feral hog",
            Species::Dog => "dog",
            Species::Cat => "cat",
            Species::Cow => "cow",
            Species::Sheep => "sheep",
            Species::Zombie => "zombie",
        }
    }

    /// Lowercase plural display name.
    pub fn plural(self) -> &'static str {
        match self {
            Species::Rat => "rats",
            Species::Rabbit => "rabbits",
            Species::Possum => "possums",
            Species::Raccoon => "raccoons",
            Species::Groundhog => "groundhogs",
            Species::Beaver => "beavers",
            Species::JuvenileFeralHog => "juvenile feral hogs",
            Species::Dog => "dogs",
            Species::Cat => "cats",
            Species::Cow => "cows",
            Species::Sheep => "sheep",
            Species::Zombie => "zombies",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounty_table_matches_sdd_7_4() {
        assert_eq!(Species::Rat.bounty_cents(), Some(800));
        assert_eq!(Species::Rabbit.bounty_cents(), Some(1_500));
        assert_eq!(Species::Possum.bounty_cents(), Some(2_500));
        assert_eq!(Species::Raccoon.bounty_cents(), Some(6_000));
        assert_eq!(Species::Groundhog.bounty_cents(), Some(9_000));
        assert_eq!(Species::Beaver.bounty_cents(), Some(14_000));
        assert_eq!(Species::JuvenileFeralHog.bounty_cents(), Some(20_000));
        assert_eq!(Species::Zombie.bounty_cents(), None);
        assert_eq!(Species::Cat.bounty_cents(), None);
    }

    #[test]
    fn friendlies_and_zombies_are_unlicensed() {
        for sp in Species::ALL {
            if sp.is_friendly() || sp == Species::Zombie {
                assert_eq!(sp.license_required(), None);
            } else {
                assert!(sp.license_required().is_some());
            }
        }
    }
}
