//! Step-through hunt simulation.
//!
//! The hunter sweeps a thermal scope across the scene, detects zombies based on
//! physically-modelled SNR, fires, and hits based on range-dependent accuracy.
//! Full P&L accounting is generated at the end of each hunt.

use rand::Rng;
use crate::{
    economy::{Catalogue, LineItem, ProfitLossLedger},
    entity::EntityKind,
    thermal::ThermalOptics,
    world::World,
};

/// Configuration for a single hunt.
#[derive(Debug, Clone)]
pub struct HuntConfig {
    pub optics: ThermalOptics,
    /// Government bounty per confirmed zombie elimination (USD).
    pub bounty_per_kill: f64,
    /// Cost per round of .308 FMJ (USD).
    pub ammo_cost_per_round: f64,
    /// Night-hunt permit fee (USD).
    pub permit_cost: f64,
    /// Thermal scope depreciation for this hunt (USD).
    pub optics_depreciation: f64,
    /// Rifle depreciation for this hunt (USD).
    pub rifle_depreciation: f64,
    /// Hard cap on simulation ticks (prevents infinite loops on unsolvable maps).
    pub max_ticks: u32,
}

impl Default for HuntConfig {
    fn default() -> Self {
        Self {
            optics: ThermalOptics::budget(),
            bounty_per_kill: 250.0,
            ammo_cost_per_round: 1.20,
            permit_cost: 75.0,
            optics_depreciation: Catalogue::thermal_scope_budget().cost_per_hunt(),
            rifle_depreciation: Catalogue::rifle_bolt_action().cost_per_hunt(),
            max_ticks: 300,
        }
    }
}

/// Final results returned after a hunt is complete.
#[derive(Debug)]
pub struct HuntResult {
    pub kills: u32,
    pub shots_fired: u32,
    pub ticks_elapsed: u32,
    pub ledger: ProfitLossLedger,
}

/// Mutable hunt state used during simulation.
pub struct HuntSimulation {
    pub world: World,
    pub config: HuntConfig,
    pub tick: u32,
    pub shots_fired: u32,
    pub kills: u32,
    pub log: Vec<String>,
}

impl HuntSimulation {
    pub fn new(world: World, config: HuntConfig) -> Self {
        Self { world, config, tick: 0, shots_fired: 0, kills: 0, log: Vec::new() }
    }

    /// Advance one tick.  Returns `false` when the hunt is finished.
    ///
    /// Each tick the hunter rotates their heading by 15 °, scanning for targets
    /// within the thermal optic's FOV.  A detected zombie is engaged immediately.
    pub fn step<R: Rng>(&mut self, rng: &mut R) -> bool {
        if self.tick >= self.config.max_ticks || self.world.zombie_count() == 0 {
            return false;
        }
        self.tick += 1;

        // The hunter sweeps 15 ° per tick (one full rotation every 24 ticks).
        let heading_deg = (self.tick as f32 * 15.0) % 360.0;
        let ambient = self.world.ambient_temp_c;

        // Snapshot alive hunters and zombies (avoids borrow conflicts during mutation).
        let hunters: Vec<_> = self.world.entities.iter()
            .filter(|e| e.kind == EntityKind::Hunter && e.alive)
            .cloned()
            .collect();

        let zombies: Vec<_> = self.world.entities.iter()
            .filter(|e| e.kind == EntityKind::Zombie && e.alive)
            .cloned()
            .collect();

        // Track which zombies were eliminated this tick to avoid double-counting.
        let mut killed_this_tick: std::collections::HashSet<u32> = std::collections::HashSet::new();

        for hunter in &hunters {
            for zombie in &zombies {
                if killed_this_tick.contains(&zombie.id) {
                    continue; // already down this tick
                }

                let p_detect = self.config.optics.detection_probability(
                    hunter,
                    heading_deg,
                    zombie,
                    ambient,
                );

                if rng.gen::<f32>() >= p_detect {
                    continue; // not detected this tick
                }

                // Detection → fire one round.
                self.shots_fired += 1;

                let dx = zombie.position.x - hunter.position.x;
                let dy = zombie.position.y - hunter.position.y;
                let dist = (dx * dx + dy * dy).sqrt();

                // Hit probability falls off exponentially with range:
                // ~70 % at point-blank, ~36 % at 200 m, ~5 % minimum.
                let p_hit = (0.70_f32 * (-dist / 200.0).exp()).clamp(0.05, 0.95);

                if rng.gen::<f32>() < p_hit {
                    self.kills += 1;
                    killed_this_tick.insert(zombie.id);
                    self.log.push(format!(
                        "Tick {:3}: Hunter {} eliminated zombie {} at {:.1} m",
                        self.tick, hunter.id, zombie.id, dist
                    ));
                } else {
                    self.log.push(format!(
                        "Tick {:3}: Shot missed zombie {} (range {:.1} m)",
                        self.tick, zombie.id, dist
                    ));
                }
            }
        }

        // Apply kills.
        for zid in &killed_this_tick {
            if let Some(z) = self.world.entities.iter_mut().find(|e| e.id == *zid) {
                z.alive = false;
            }
        }

        true
    }

    /// Run to completion.
    pub fn run<R: Rng>(&mut self, rng: &mut R) {
        while self.step(rng) {}
    }

    /// Build the P&L ledger from the accumulated hunt stats.
    pub fn build_ledger(&self) -> ProfitLossLedger {
        let mut ledger = ProfitLossLedger::new();

        // Revenue
        if self.kills > 0 {
            ledger.add(LineItem::revenue(
                format!("Zombie eradication bounty ×{}", self.kills),
                self.kills as f64 * self.config.bounty_per_kill,
            ));
        }

        // Variable costs
        if self.shots_fired > 0 {
            ledger.add(LineItem::expense(
                format!(".308 FMJ ×{}", self.shots_fired),
                self.shots_fired as f64 * self.config.ammo_cost_per_round,
            ));
        }

        // Fixed costs
        ledger.add(LineItem::expense("Night-hunt permit", self.config.permit_cost));
        ledger.add(LineItem::expense("Thermal scope (depreciation)", self.config.optics_depreciation));
        ledger.add(LineItem::expense("Rifle (depreciation)", self.config.rifle_depreciation));

        ledger
    }

    /// Consume the simulation and return final results.
    pub fn finish(self) -> HuntResult {
        let ledger = self.build_ledger();
        HuntResult {
            kills: self.kills,
            shots_fired: self.shots_fired,
            ticks_elapsed: self.tick,
            ledger,
        }
    }
}
