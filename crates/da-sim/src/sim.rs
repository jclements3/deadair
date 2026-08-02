//! The headless night simulation: entities, hazards, weapon, noise
//! propagation, and the per-tick event stream the other layers consume.

use da_core::weather::WeatherMods;
use da_core::{EntityId, IdGen, Rng};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::ai::{AiCtx, Animal, Light};
use crate::events::{DamageCause, HeatKind, SimEvent};
use crate::hazard::{Hazard, Health};
use crate::hit::{check_backstop, resolve_shot, ShotOutcome, Species, Target};
use crate::noise::{discharge_noise_radius_m, NoiseEvent, NoiseKind};
use crate::weapon::{PowerPlant, RifleConfig};

/// Horizontal range at which a zombie lands contact damage.
pub const ZOMBIE_MELEE_RANGE_M: f32 = 1.4;
/// Health lost per zombie contact hit.
pub const ZOMBIE_CONTACT_DAMAGE: f32 = 8.0;
/// Seconds between contact hits from the same zombie.
pub const ZOMBIE_ATTACK_COOLDOWN_S: f32 = 1.0;
/// Wound/friendly-hit alert pulse radius (nearby pests flee).
pub const ALERT_PULSE_RADIUS_M: f32 = 25.0;
/// A raccoon must be this close to a dying group-mate to witness it.
pub const WITNESS_RADIUS_M: f32 = 40.0;
/// Reference walking speed used to normalize hazard trip rolls.
pub const WALK_SPEED_MPS: f32 = 1.5;

/// The player: eye-height position plus health pool.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Player {
    /// Eye position, world meters (shots originate here).
    pub pos: Vec3,
    /// Health pool (FR-H1).
    pub health: Health,
}

/// A scripted input, for replays and determinism testing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Advance the simulation.
    Tick {
        /// Seconds.
        dt: f32,
    },
    /// Move the player by `delta` over `dt` seconds (speed = |delta|/dt).
    Move {
        /// Displacement, meters.
        delta: Vec3,
        /// Seconds spent moving.
        dt: f32,
    },
    /// Fire the rifle along `dir`.
    Fire {
        /// Aim direction (ballistically compensated by the caller).
        dir: Vec3,
    },
    /// Work the multi-pump lever for `dt` seconds (interruptible).
    Pump {
        /// Seconds of pumping.
        dt: f32,
    },
    /// Inject an external noise (dropped gear, scripted scare...).
    EmitNoise {
        /// Source position.
        pos: Vec3,
        /// Reaction radius, meters.
        radius_m: f32,
    },
}

/// The headless gameplay simulation. Deterministic: the same seed and
/// command script always produce the same event log.
#[derive(Debug)]
pub struct Sim {
    /// Absolute sim time, seconds since dusk.
    pub time: f32,
    /// Night weather modifiers.
    pub weather: WeatherMods,
    /// The player.
    pub player: Player,
    /// Carried rifle.
    pub rifle: RifleConfig,
    /// All AI entities.
    pub animals: Vec<Animal>,
    /// Placed trip hazards.
    pub hazards: Vec<Hazard>,
    /// Player light (headlamp); animals inside its radius count as lit.
    pub light: Option<Light>,
    ids: IdGen,
    master_rng: Rng,
    shot_rng: Rng,
    player_rng: Rng,
    pending_noise: Vec<NoiseEvent>,
    events: Vec<SimEvent>,
    hazard_inside: Vec<bool>,
}

impl Sim {
    /// Build an empty world.
    pub fn new(seed: u64, rifle: RifleConfig, weather: WeatherMods) -> Self {
        let mut master = Rng::new(seed);
        let shot_rng = master.fork(0xB411);
        let player_rng = master.fork(0x9A57);
        Self {
            time: 0.0,
            weather,
            player: Player { pos: Vec3::new(0.0, 1.6, 0.0), health: Health::new(100.0) },
            rifle,
            animals: Vec::new(),
            hazards: Vec::new(),
            light: None,
            ids: IdGen::new(),
            master_rng: master,
            shot_rng,
            player_rng,
            pending_noise: Vec::new(),
            events: Vec::new(),
            hazard_inside: Vec::new(),
        }
    }

    /// Spawn an animal (spawn node = `pos`), with optional raccoon group
    /// tag and waypoints (patrol route / tree nodes / rat nodes).
    pub fn spawn_animal(
        &mut self,
        species: Species,
        pos: Vec3,
        group: Option<u32>,
        waypoints: Vec<Vec3>,
    ) -> EntityId {
        let id = self.ids.entity();
        let rng = self.master_rng.fork(id.0);
        self.animals.push(Animal::new(id, species, pos, group, waypoints, rng));
        id
    }

    /// Spawn with no group/waypoints.
    pub fn spawn(&mut self, species: Species, pos: Vec3) -> EntityId {
        self.spawn_animal(species, pos, None, Vec::new())
    }

    /// Add a hazard volume.
    pub fn add_hazard(&mut self, hazard: Hazard) {
        self.hazards.push(hazard);
        self.hazard_inside.push(false);
    }

    /// Look up an animal.
    pub fn animal(&self, id: EntityId) -> Option<&Animal> {
        self.animals.iter().find(|a| a.id == id)
    }

    /// Take everything emitted since the last drain.
    pub fn drain_events(&mut self) -> Vec<SimEvent> {
        std::mem::take(&mut self.events)
    }

    /// Advance the world and return the events of this tick.
    pub fn tick(&mut self, dt: f32) -> Vec<SimEvent> {
        self.step(dt);
        self.drain_events()
    }

    /// Apply one scripted command (events accumulate; drain when ready).
    pub fn apply(&mut self, cmd: &Command) {
        match *cmd {
            Command::Tick { dt } => self.step(dt),
            Command::Move { delta, dt } => self.move_player(delta, dt),
            Command::Fire { dir } => {
                self.fire(dir);
            }
            Command::Pump { dt } => self.pump(dt),
            Command::EmitNoise { pos, radius_m } => {
                self.emit_noise(NoiseEvent { pos, radius_m, kind: NoiseKind::Other });
            }
        }
    }

    /// Run a whole command script and return the complete event log.
    pub fn run_script(&mut self, script: &[Command]) -> Vec<SimEvent> {
        for cmd in script {
            self.apply(cmd);
        }
        self.drain_events()
    }

    /// Advance all systems by `dt` (events accumulate internally).
    pub fn step(&mut self, dt: f32) {
        self.time += dt;
        let noises = std::mem::take(&mut self.pending_noise);
        let light = self.light;
        let time = self.time;
        let player_pos = self.player.pos;

        for a in self.animals.iter_mut() {
            let is_lit = light.map_or(false, |l| (a.pos - l.pos).length() < l.radius_m);
            let ctx = AiCtx {
                dt,
                time,
                noises: &noises,
                is_lit,
                player_pos,
                weather: &self.weather,
            };
            a.tick(&ctx, &mut self.events);
        }

        // Zombie contact damage (FR-A7).
        let mut total = 0.0;
        for a in self.animals.iter_mut() {
            if a.alive
                && a.species == Species::Zombie
                && !a.is_staggered(time)
                && time >= a.next_attack_time
            {
                let d = a.pos - player_pos;
                if Vec3::new(d.x, 0.0, d.z).length() <= ZOMBIE_MELEE_RANGE_M {
                    a.next_attack_time = time + ZOMBIE_ATTACK_COOLDOWN_S;
                    total += ZOMBIE_CONTACT_DAMAGE;
                }
            }
        }
        if total > 0.0 {
            self.player.health.damage(total);
            self.events.push(SimEvent::PlayerDamaged {
                amount: total,
                cause: DamageCause::ZombieContact,
            });
        }
    }

    /// Register a noise: recorded as an event now, propagated to AI on the
    /// next tick.
    pub fn emit_noise(&mut self, n: NoiseEvent) {
        self.events.push(SimEvent::NoiseMade { pos: n.pos, radius_m: n.radius_m, kind: n.kind });
        self.pending_noise.push(n);
    }

    /// Work the pump lever for `dt` seconds. Each completed stroke emits a
    /// small movement-noise event; stopping mid-stroke keeps completed
    /// strokes (interruptible timed action).
    pub fn pump(&mut self, dt: f32) {
        let strokes = self.rifle.pump(dt);
        if strokes == 0 {
            return;
        }
        if let PowerPlant::MultiPump { pump_noise_radius_m, .. } = self.rifle.plant {
            let pos = self.player.pos;
            for _ in 0..strokes {
                self.emit_noise(NoiseEvent {
                    pos,
                    radius_m: pump_noise_radius_m,
                    kind: NoiseKind::PumpStroke,
                });
            }
        }
    }

    /// Colliders for every living animal.
    pub fn targets(&self) -> Vec<Target> {
        self.animals.iter().filter(|a| a.alive).map(|a| a.target()).collect()
    }

    /// Pre-fire reticle warning (FR-A8): friendly in the line of fire
    /// within lethal range, including behind the intended target.
    pub fn check_backstop(&self, dir: Vec3) -> bool {
        check_backstop(self.player.pos, dir, &self.targets(), self.rifle.lethal_range_m())
    }

    /// Fire the rifle along `dir` (ballistically compensated by the aiming
    /// layer). Consumes pumps/air, emits noise + barrel heat, resolves the
    /// hit, and applies the outcome to the world.
    pub fn fire(&mut self, dir: Vec3) -> Option<ShotOutcome> {
        if self.rifle.muzzle_energy_fpe().is_none() {
            self.events.push(SimEvent::DryFire);
            return None;
        }
        let origin = self.player.pos;
        let targets = self.targets();
        let outcome = resolve_shot(origin, dir, &targets, &self.rifle, &mut self.shot_rng);
        let energy = self.rifle.fire().expect("checked above");

        let radius = discharge_noise_radius_m(energy, self.rifle.moderator);
        self.emit_noise(NoiseEvent { pos: origin, radius_m: radius, kind: NoiseKind::Discharge });
        self.events.push(SimEvent::HeatResidue { kind: HeatKind::Barrel, pos: origin });

        self.apply_outcome(&outcome);
        Some(outcome)
    }

    fn apply_outcome(&mut self, outcome: &ShotOutcome) {
        let time = self.time;
        match *outcome {
            ShotOutcome::Kill { id, species, pos } => {
                if let Some(a) = self.animals.iter_mut().find(|a| a.id == id) {
                    a.kill(time);
                }
                self.events.push(SimEvent::KillConfirmed {
                    id,
                    species,
                    bounty_eligible: species.is_pest(),
                    pos,
                });
                if species == Species::Raccoon {
                    self.spook_witnessing_group(id, pos);
                }
            }
            ShotOutcome::Wound { id, species, pos } => {
                self.events.push(SimEvent::Wounded { id, species, pos });
                let player = self.player.pos;
                if let Some(a) = self.animals.iter_mut().find(|a| a.id == id) {
                    a.start_flee(player, time, &mut self.events);
                }
                self.alert_pulse(pos, Some(id));
            }
            ShotOutcome::FriendlyHit { id, species } => {
                self.events.push(SimEvent::FriendlyHit { id, species });
                if let Some(pos) = self.animals.iter().find(|a| a.id == id).map(|a| a.pos) {
                    self.alert_pulse(pos, Some(id));
                }
            }
            ShotOutcome::ZombieDestroyed { id } => {
                if let Some(a) = self.animals.iter_mut().find(|a| a.id == id) {
                    a.alive = false;
                }
                self.events.push(SimEvent::ZombieDestroyed { id });
            }
            ShotOutcome::ZombieStagger { id } => {
                if let Some(a) = self.animals.iter_mut().find(|a| a.id == id) {
                    a.stagger(time);
                }
                self.events.push(SimEvent::ZombieStaggered { id });
            }
            ShotOutcome::Miss { impact } => {
                if let Some(p) = impact {
                    self.events
                        .push(SimEvent::HeatResidue { kind: HeatKind::PelletImpact, pos: p });
                }
                self.events.push(SimEvent::Missed { impact });
            }
        }
    }

    /// Wound/friendly-hit alert pulse: pests near the impact bolt (FR-A3).
    fn alert_pulse(&mut self, center: Vec3, exclude: Option<EntityId>) {
        let time = self.time;
        for a in self.animals.iter_mut() {
            if a.alive
                && a.species.is_pest()
                && Some(a.id) != exclude
                && (a.pos - center).length() < ALERT_PULSE_RADIUS_M
            {
                a.start_flee(center, time, &mut self.events);
            }
        }
    }

    /// FR-A4: if any living group-mate was close enough to witness the
    /// kill, the whole group's flee threshold rises for the rest of the
    /// night.
    fn spook_witnessing_group(&mut self, victim: EntityId, at: Vec3) {
        let Some(group) =
            self.animals.iter().find(|a| a.id == victim).and_then(|a| a.group)
        else {
            return;
        };
        let witnessed = self.animals.iter().any(|a| {
            a.alive
                && a.id != victim
                && a.species == Species::Raccoon
                && a.group == Some(group)
                && (a.pos - at).length() < WITNESS_RADIUS_M
        });
        if witnessed {
            for a in self.animals.iter_mut() {
                if a.species == Species::Raccoon && a.group == Some(group) {
                    a.spook_group_memory();
                }
            }
        }
    }

    /// Move the player, rolling trip damage when entering a hazard —
    /// scaled by weather severity and movement speed (SRS §3.5).
    pub fn move_player(&mut self, delta: Vec3, dt: f32) {
        let speed = if dt > 0.0 { delta.length() / dt } else { 0.0 };
        self.player.pos += delta;
        let pos = self.player.pos;
        let severity = self.weather.hazard_severity;
        for (i, h) in self.hazards.iter().enumerate() {
            let inside = h.contains(pos);
            if inside && !self.hazard_inside[i] {
                let speed_factor = (speed / WALK_SPEED_MPS).clamp(0.25, 3.0);
                let p = (h.kind.trip_chance() * severity * speed_factor).min(1.0);
                if self.player_rng.chance(p) {
                    let dmg = h.kind.base_damage()
                        * severity
                        * (0.75 + 0.25 * (speed / WALK_SPEED_MPS).min(3.0));
                    self.player.health.damage(dmg);
                    self.events.push(SimEvent::PlayerDamaged {
                        amount: dmg,
                        cause: DamageCause::Hazard(h.kind),
                    });
                }
            }
            self.hazard_inside[i] = inside;
        }
    }
}
