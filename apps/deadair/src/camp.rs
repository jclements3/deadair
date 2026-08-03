//! Camp: the between-nights layer. Zone catalog and hub-path travel
//! (SDD §3), nightly contract generation from zone sources, and the
//! purchase gates the store UI drives. Pure logic — the UI in `main.rs`
//! only renders it.

use da_core::{Forecast, Rng};
use da_econ::{Accessory, Business, Contract, ContractBoard, ItemKind, Species};

/// One playable zone, discovered from `assets/zones/`.
#[derive(Debug, Clone)]
pub struct ZoneEntry {
    /// File name, e.g. `home_farm.zone.ron`.
    pub file: String,
    /// Display name from the source, e.g. "Home Farm".
    pub name: String,
    /// Client the contracts bill to (the zone name; town zones bill Town).
    pub client: String,
    /// Walking minutes from camp (camp sits at Home Farm).
    pub walk_min: u32,
    /// Species the zone's own source suggests, with quotas.
    pub hints: Vec<(Species, u32)>,
    /// Baseline population per species, straight from the zone's spawn
    /// tables. Contract quotas are capped against this so the board can
    /// never post a job the zone cannot physically supply.
    pub population: Vec<(Species, u32)>,
}

impl ZoneEntry {
    /// Night-clock fraction consumed travelling here, given the bicycle
    /// upgrade (FR-E3: the bike halves travel time).
    pub fn travel_fraction(&self, night_hours: f32, bicycle: bool) -> f32 {
        let mins = if bicycle {
            self.walk_min as f32 * 0.5
        } else {
            self.walk_min as f32
        };
        (mins / 60.0 / night_hours).clamp(0.0, 0.9)
    }
}

/// All zones, loaded from their parametric sources so the catalog can never
/// drift from the content.
#[derive(Debug, Clone)]
pub struct ZoneCatalog {
    pub zones: Vec<ZoneEntry>,
}

/// Camp is co-located with the Home Farm.
pub const CAMP_ZONE: &str = "Home Farm";

/// How many of `species` this zone can yield over `nights`.
///
/// Rats breed back fast enough to restock nightly (SDD §6: "fast re-spawn —
/// population pressure justifies contracts"); possums and raccoons trickle
/// back at roughly half a population per night; the License-D species
/// (beaver, groundhog, hog) do not meaningfully replenish inside a contract
/// window, so their supply is simply what is standing there tonight.
pub fn supply_over(base_count: u32, species: Species, nights: u32) -> u32 {
    let nights = nights.max(1);
    let base = base_count as f32;
    let total = match species {
        Species::Rat => base * nights as f32,
        Species::Possum | Species::Raccoon => base * (1.0 + 0.5 * (nights - 1) as f32),
        _ => base,
    };
    total.floor() as u32
}

impl ZoneEntry {
    /// Baseline population of `species` in this zone (0 if it doesn't live here).
    pub fn population_of(&self, species: Species) -> u32 {
        self.population
            .iter()
            .find(|(s, _)| *s == species)
            .map(|(_, n)| *n)
            .unwrap_or(0)
    }
}

impl ZoneCatalog {
    /// Load every `*.zone.ron` in `dir`, deriving travel times from the hub
    /// paths declared in the sources (shortest path from the camp zone).
    pub fn load(dir: &str) -> Result<Self, String> {
        let sources = da_param::load_all_zones(dir).map_err(|e| e.to_string())?;
        if sources.is_empty() {
            return Err(format!("no zone sources in {dir}"));
        }

        // Dijkstra over declared connections, from the camp zone.
        let mut dist: Vec<(String, u32)> = Vec::new();
        let mut frontier = vec![(CAMP_ZONE.to_string(), 0u32)];
        while let Some((name, d)) = frontier.pop() {
            if let Some(existing) = dist.iter_mut().find(|(n, _)| *n == name) {
                if existing.1 <= d {
                    continue;
                }
                existing.1 = d;
            } else {
                dist.push((name.clone(), d));
            }
            if let Some(src) = sources.iter().find(|s| s.name == name) {
                for c in &src.connections {
                    frontier.push((c.to.clone(), d + c.walk_min));
                }
            }
        }

        let mut zones: Vec<ZoneEntry> = sources
            .iter()
            .map(|s| {
                let walk_min = dist
                    .iter()
                    .find(|(n, _)| *n == s.name)
                    .map(|(_, d)| *d)
                    .unwrap_or(45); // unreachable by declared paths: long hike
                ZoneEntry {
                    file: file_name_for(&s.name),
                    name: s.name.clone(),
                    client: client_for(&s.name),
                    walk_min,
                    hints: s
                        .contracts_hint
                        .iter()
                        .filter_map(|h| {
                            econ_species(&format!("{:?}", h.species)).map(|sp| (sp, h.quota))
                        })
                        .collect(),
                    population: s
                        .spawn_tables
                        .iter()
                        .filter_map(|t| {
                            econ_species(&format!("{:?}", t.species)).map(|sp| (sp, t.base_count))
                        })
                        .collect(),
                }
            })
            .collect();
        zones.sort_by_key(|z| z.walk_min);
        Ok(Self { zones })
    }

    pub fn find(&self, name: &str) -> Option<&ZoneEntry> {
        self.zones.iter().find(|z| z.name == name)
    }
}

fn file_name_for(display: &str) -> String {
    format!(
        "{}.zone.ron",
        display.to_lowercase().replace(['-', ' '], "_").replace("co_op", "coop")
    )
}

fn client_for(zone: &str) -> String {
    if zone.starts_with("Town") || zone.starts_with("Main") {
        da_econ::TOWN_CLIENT.to_string()
    } else {
        zone.to_string()
    }
}

fn econ_species(name: &str) -> Option<Species> {
    Some(match name {
        "Rat" => Species::Rat,
        "Possum" => Species::Possum,
        "Raccoon" => Species::Raccoon,
        "Beaver" => Species::Beaver,
        "Groundhog" => Species::Groundhog,
        "JuvenileFeralHog" => Species::JuvenileFeralHog,
        _ => return None,
    })
}

/// Build tonight's contract board from the zone catalog. Deterministic in
/// `seed`, so the same night always offers the same jobs.
pub fn generate_board(catalog: &ZoneCatalog, seed: u64, forecast: Forecast) -> ContractBoard {
    let mut rng = Rng::new(seed);
    let mut contracts = Vec::new();
    let mut id = 1u32;
    for zone in &catalog.zones {
        for (species, quota) in &zone.hints {
            let deadline = 2 + rng.below(3) as u32;
            // Quota jitters ±25% so repeat visits aren't identical — then is
            // capped at what the zone can actually supply before the deadline.
            // A contract you cannot physically fill is not difficulty, it's a
            // bug that eats the player's nights and reputation.
            let wanted = ((*quota as f32) * rng.range(0.75, 1.25)).round().max(1.0) as u32;
            let supply = supply_over(zone.population_of(*species), *species, deadline);
            if supply == 0 {
                continue;
            }
            let q = wanted.min(supply);
            let rep_required = match species {
                Species::Rat => 0,
                Species::Possum => 10,
                Species::Raccoon => 50,
                _ => 80,
            };
            // Pre-storm rat surges pay a bonus at the grain co-op (FR-WX4).
            let bonus = if forecast == Forecast::PreStorm && *species == Species::Rat {
                Some(da_econ::WeatherBonus {
                    forecast: Forecast::PreStorm,
                    multiplier: 1.5,
                })
            } else {
                None
            };
            contracts.push(Contract::new(
                id,
                &zone.client,
                &zone.name,
                *species,
                q,
                deadline,
                rep_required,
                bonus,
            ));
            id += 1;
        }
    }
    ContractBoard { contracts }
}

/// Campaign outcome (FR-E4 / FR-B6). The campaign is won by clearing a
/// completed contract in every zone while keeping every client's reputation
/// above zero, and lost by bankruptcy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CampaignState {
    /// Still trading.
    Running,
    /// Cash below zero with nothing left to sell.
    Bankrupt,
    /// Every zone cleared with reputation intact.
    Won,
}

/// Evaluate the campaign against the zone catalog.
///
/// "Cleared" means the player has a completed contract on record for that
/// zone; reputation must be above zero with every client they've worked for,
/// since a client who has soured on you can't be worked again.
pub fn campaign_state(business: &Business, catalog: &ZoneCatalog) -> CampaignState {
    if business.is_bankrupt() {
        return CampaignState::Bankrupt;
    }
    if catalog.zones.is_empty() {
        return CampaignState::Running;
    }
    let all_cleared = catalog.zones.iter().all(|z| {
        business
            .contracts()
            .iter()
            .any(|c| c.zone == z.name && c.status == da_econ::ContractStatus::Completed)
    });
    let rep_intact = catalog
        .zones
        .iter()
        .all(|z| business.rep(&z.client) > 0);
    if all_cleared && rep_intact {
        CampaignState::Won
    } else {
        CampaignState::Running
    }
}

/// Does the player own a bicycle (halves travel time)?
pub fn has_bicycle(business: &Business) -> bool {
    business.owns(ItemKind::Accessory(Accessory::Bicycle))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/zones").to_string()
    }

    #[test]
    fn catalog_loads_all_six_zones_with_travel_times() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        assert_eq!(cat.zones.len(), 6, "six zones per SDD §3");
        let home = cat.find("Home Farm").expect("home farm");
        assert_eq!(home.walk_min, 0, "camp is at the home farm");
        // Main Street is the far end of the hub path: Home→Orchard→Town
        // Edge→Main St, or via the co-op. Either way it's the longest hike.
        let main = cat.find("Main Street").expect("main street");
        assert!(main.walk_min >= 35, "main street travel: {}", main.walk_min);
        assert!(cat.zones.windows(2).all(|w| w[0].walk_min <= w[1].walk_min));
    }

    #[test]
    fn zone_files_resolve_to_real_sources() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        for z in &cat.zones {
            let path = format!("{}/{}", dir(), z.file);
            assert!(
                std::path::Path::new(&path).exists(),
                "derived file name must exist: {path}"
            );
        }
    }

    #[test]
    fn bicycle_halves_travel() {
        let z = ZoneEntry {
            file: "x".into(),
            name: "X".into(),
            client: "X".into(),
            walk_min: 30,
            hints: vec![],
            population: vec![],
        };
        let walk = z.travel_fraction(10.0, false);
        let ride = z.travel_fraction(10.0, true);
        assert!((walk - 0.05).abs() < 1e-6, "30 min of a 10 h night");
        assert!((ride - walk * 0.5).abs() < 1e-6);
    }

    #[test]
    fn town_zones_bill_the_town_client() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        assert_eq!(cat.find("Main Street").expect("z").client, da_econ::TOWN_CLIENT);
        assert_eq!(cat.find("Town Edge").expect("z").client, da_econ::TOWN_CLIENT);
        assert_eq!(cat.find("Orchard").expect("z").client, "Orchard");
    }

    #[test]
    fn board_is_deterministic_and_gated() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        let a = generate_board(&cat, 7, Forecast::Clear);
        let b = generate_board(&cat, 7, Forecast::Clear);
        assert_eq!(a.contracts.len(), b.contracts.len());
        for (x, y) in a.contracts.iter().zip(&b.contracts) {
            assert_eq!((x.quota, x.species, &x.zone), (y.quota, y.species, &y.zone));
        }
        // A fresh business (License A, rats only) sees only rat jobs.
        let biz = Business::new();
        let visible = a.visible(&biz);
        assert!(!visible.is_empty(), "starter must have work");
        assert!(
            visible.iter().all(|c| c.species == Species::Rat),
            "License A gates everything but rats"
        );
    }

    #[test]
    fn no_contract_can_exceed_what_the_zone_can_supply() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        for seed in 0..40u64 {
            for forecast in Forecast::ALL {
                for c in generate_board(&cat, seed, forecast).contracts {
                    let zone = cat.find(&c.zone).expect("zone");
                    let supply = supply_over(
                        zone.population_of(c.species),
                        c.species,
                        c.deadline_nights,
                    );
                    assert!(
                        c.quota <= supply && c.quota > 0,
                        "unfillable contract: {} wants {} {:?} but the zone supplies {} \
                         over {} nights",
                        c.zone,
                        c.quota,
                        c.species,
                        supply,
                        c.deadline_nights
                    );
                }
            }
        }
    }

    #[test]
    fn supply_model_matches_respawn_behaviour() {
        // Rats restock every night; raccoons trickle; beavers don't come back.
        assert_eq!(supply_over(8, Species::Rat, 3), 24);
        assert_eq!(supply_over(4, Species::Raccoon, 3), 8);
        assert_eq!(supply_over(2, Species::Beaver, 3), 2);
        // A zone with none of a species supplies none, whatever the deadline.
        assert_eq!(supply_over(0, Species::Rat, 5), 0);
    }

    #[test]
    fn zones_without_a_species_post_no_contract_for_it() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        for c in generate_board(&cat, 11, Forecast::Clear).contracts {
            let zone = cat.find(&c.zone).expect("zone");
            assert!(
                zone.population_of(c.species) > 0,
                "{} posted a {:?} job but has none",
                c.zone,
                c.species
            );
        }
    }

    #[test]
    fn campaign_starts_running_and_bankruptcy_is_terminal() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        let mut biz = Business::new();
        assert_eq!(campaign_state(&biz, &cat), CampaignState::Running);
        // Spend everything, own nothing sellable.
        while biz.sell_equipment(0).is_ok() {}
        biz.cash_cents = -1;
        assert_eq!(campaign_state(&biz, &cat), CampaignState::Bankrupt);
    }

    #[test]
    fn clearing_every_zone_with_reputation_intact_wins() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        let mut biz = Business::new();
        // Record a completed contract in every zone, reputation positive.
        for (i, z) in cat.zones.iter().enumerate() {
            biz.adjust_rep(&z.client, 20);
            let mut c = Contract::new(
                i as u32 + 1,
                &z.client,
                &z.name,
                Species::Rat,
                1,
                3,
                0,
                None,
            );
            c.status = da_econ::ContractStatus::Completed;
            biz.contracts_mut().push(c);
        }
        assert_eq!(campaign_state(&biz, &cat), CampaignState::Won);
    }

    #[test]
    fn one_uncleared_zone_keeps_the_campaign_running() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        let mut biz = Business::new();
        for (i, z) in cat.zones.iter().enumerate().skip(1) {
            biz.adjust_rep(&z.client, 20);
            let mut c = Contract::new(
                i as u32 + 1,
                &z.client,
                &z.name,
                Species::Rat,
                1,
                3,
                0,
                None,
            );
            c.status = da_econ::ContractStatus::Completed;
            biz.contracts_mut().push(c);
        }
        assert_eq!(campaign_state(&biz, &cat), CampaignState::Running);
    }

    #[test]
    fn pre_storm_adds_rat_surge_bonuses() {
        let cat = ZoneCatalog::load(&dir()).expect("catalog");
        let surge = generate_board(&cat, 3, Forecast::PreStorm);
        assert!(
            surge
                .contracts
                .iter()
                .any(|c| c.species == Species::Rat && c.bounty_bonus.is_some()),
            "pre-storm rat contracts carry a weather bonus (FR-WX4)"
        );
        let calm = generate_board(&cat, 3, Forecast::Clear);
        assert!(calm.contracts.iter().all(|c| c.bounty_bonus.is_none()));
    }
}
