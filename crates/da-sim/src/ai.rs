//! Per-species AI state machines (SDD §6, FR-A1..A7).
//!
//! Every animal owns a forked [`Rng`] stream, so movement replays
//! identically from the same seed regardless of what other entities do.

use da_core::{EntityId, Rng};
use da_core::weather::WeatherMods;
use glam::Vec3;

use crate::events::{HeatKind, SimEvent};
use crate::hit::{Species, Target};
use crate::noise::NoiseEvent;

/// Seconds a zombie keeps homing on a heard noise (SDD §5.3).
pub const NOISE_SEEK_DURATION_S: f32 = 60.0;
/// Seconds a pest keeps running once spooked.
pub const FLEE_DURATION_S: f32 = 6.0;
/// Rat respawn delay (population pressure justifies contracts).
pub const RAT_RESPAWN_S: f32 = 25.0;
/// Flee-radius multiplier for a raccoon group that witnessed a kill (FR-A4).
pub const GROUP_SPOOK_MULT: f32 = 1.75;
/// Effective noise-radius multiplier while a possum is frozen in a light —
/// freezing lowers its flee response.
pub const FROZEN_FLEE_MULT: f32 = 0.3;
/// Zombie body-shot stagger duration (movement pause).
pub const STAGGER_DURATION_S: f32 = 2.5;
/// Resting this long leaves a warm bedding spot (SDD §2.3).
pub const BEDDING_REST_S: f32 = 30.0;

/// A player-carried light source (headlamp cone abstracted to a radius).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    /// Light position.
    pub pos: Vec3,
    /// Radius within which an animal counts as lit.
    pub radius_m: f32,
}

/// Per-entity world context for one AI tick. Built by the sim; `is_lit`
/// is already resolved for the entity being ticked.
#[derive(Debug)]
pub struct AiCtx<'a> {
    /// Tick length, seconds.
    pub dt: f32,
    /// Absolute sim time, seconds.
    pub time: f32,
    /// Noises emitted since the previous tick.
    pub noises: &'a [NoiseEvent],
    /// Whether THIS entity stands in the player's light.
    pub is_lit: bool,
    /// Player position (flee-from point for light reactions).
    pub player_pos: Vec3,
    /// Night weather modifiers (activity scaling hooks).
    pub weather: &'a WeatherMods,
}

/// Locomotion state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum AiState {
    /// Dwelling until the given time.
    Idle { until: f32 },
    /// Walking to a destination.
    MoveTo { dest: Vec3 },
    /// Running away from a point until the given time.
    Flee { from: Vec3, until: f32 },
    /// Possum in a light beam.
    Frozen,
}

/// One simulated animal (pest, friendly, or zombie).
#[derive(Debug, Clone)]
pub struct Animal {
    /// Stable id.
    pub id: EntityId,
    /// Species — selects the behavior program and hit-resolution class.
    pub species: Species,
    /// Current ground position (m).
    pub pos: Vec3,
    /// Spawn node (feeding-loop anchor, respawn point).
    pub spawn: Vec3,
    /// Raccoon group tag for shared memory (FR-A4).
    pub group: Option<u32>,
    /// Patrol / bias nodes: raccoon+dog patrol them, possums treat them as
    /// trees, cats treat them as rat nodes to haunt.
    pub waypoints: Vec<Vec3>,
    /// Alive (dead rats may respawn).
    pub alive: bool,
    wp_idx: usize,
    state: AiState,
    rng: Rng,
    flee_boost: f32,
    stagger_until: f32,
    noise_seek: Option<(Vec3, f32)>,
    respawn_at: Option<f32>,
    /// Next time this zombie may land contact damage (sim-managed).
    pub(crate) next_attack_time: f32,
    still_time: f32,
    bedding_emitted: bool,
}

impl Animal {
    /// Spawn an animal with its own deterministic RNG stream.
    pub fn new(
        id: EntityId,
        species: Species,
        pos: Vec3,
        group: Option<u32>,
        waypoints: Vec<Vec3>,
        rng: Rng,
    ) -> Self {
        Self {
            id,
            species,
            pos,
            spawn: pos,
            group,
            waypoints,
            alive: true,
            wp_idx: 0,
            state: AiState::Idle { until: 0.0 },
            rng,
            flee_boost: 1.0,
            stagger_until: 0.0,
            noise_seek: None,
            respawn_at: None,
            next_attack_time: 0.0,
            still_time: 0.0,
            bedding_emitted: false,
        }
    }

    /// Colliders for hit resolution at the current position.
    pub fn target(&self) -> Target {
        Target::for_species(self.id, self.species, self.pos)
    }

    /// Base walking speed, m/s.
    pub fn speed(&self) -> f32 {
        match self.species {
            Species::Rat => 3.0,
            Species::Possum => 0.9, // slow — real possum energy
            Species::Raccoon => 2.2,
            Species::Dog => 1.8,
            Species::Cat => 1.5,
            Species::Cow | Species::Sheep => 0.0,
            Species::Zombie => 0.7,
        }
    }

    /// Current flee-radius multiplier (1.0 baseline; >1 after a raccoon
    /// group witnessed a death — FR-A4).
    pub fn flee_radius_multiplier(&self) -> f32 {
        self.flee_boost
    }

    /// Raise the group-memory flee threshold for the rest of the night.
    pub fn spook_group_memory(&mut self) {
        self.flee_boost = self.flee_boost.max(GROUP_SPOOK_MULT);
    }

    /// Is the animal currently running from something?
    pub fn is_fleeing(&self) -> bool {
        matches!(self.state, AiState::Flee { .. })
    }

    /// Possum-in-headlights?
    pub fn is_frozen(&self) -> bool {
        matches!(self.state, AiState::Frozen)
    }

    /// Movement-paused from a body shot (zombies)?
    pub fn is_staggered(&self, time: f32) -> bool {
        time < self.stagger_until
    }

    /// Apply a body-shot stagger (zombie): brief movement pause.
    pub fn stagger(&mut self, time: f32) {
        self.stagger_until = time + STAGGER_DURATION_S;
    }

    /// Kill the animal; rats schedule a fast respawn (SDD §6).
    pub fn kill(&mut self, time: f32) {
        self.alive = false;
        if self.species == Species::Rat {
            self.respawn_at = Some(time + RAT_RESPAWN_S);
        }
    }

    /// Force the animal into flight away from `from` (alert pulses, wounds).
    pub fn start_flee(&mut self, from: Vec3, time: f32, events: &mut Vec<SimEvent>) {
        if !self.alive || self.speed() <= 0.0 {
            return;
        }
        let newly = !self.is_fleeing();
        self.state = AiState::Flee { from, until: time + FLEE_DURATION_S };
        if newly && self.species.is_pest() {
            events.push(SimEvent::PestFled { id: self.id, species: self.species });
        }
    }

    /// Advance the state machine by one tick.
    pub fn tick(&mut self, ctx: &AiCtx, events: &mut Vec<SimEvent>) {
        if !self.alive {
            if let Some(at) = self.respawn_at {
                if ctx.time >= at {
                    self.alive = true;
                    self.respawn_at = None;
                    self.pos = self.spawn;
                    self.state = AiState::Idle { until: ctx.time };
                    self.still_time = 0.0;
                    self.bedding_emitted = false;
                }
            }
            return;
        }
        if self.is_staggered(ctx.time) {
            return; // body shot: movement pause
        }
        match self.species {
            Species::Rat | Species::Possum | Species::Raccoon => self.tick_pest(ctx, events),
            Species::Dog | Species::Cat => self.tick_friendly(ctx, events),
            Species::Cow | Species::Sheep => self.rest(ctx.dt, events),
            Species::Zombie => self.tick_zombie(ctx, events),
        }
    }

    // --- species programs -------------------------------------------------

    fn tick_pest(&mut self, ctx: &AiCtx, events: &mut Vec<SimEvent>) {
        // Possum freeze-when-lit: overrides wandering, lowers flee response.
        if self.species == Species::Possum {
            if ctx.is_lit && !self.is_fleeing() {
                self.state = AiState::Frozen;
            } else if self.is_frozen() && !ctx.is_lit {
                self.state = AiState::Idle { until: ctx.time + 0.5 };
            }
        }

        // Noise response, scaled by group memory and freeze.
        let mult =
            self.flee_boost * if self.is_frozen() { FROZEN_FLEE_MULT } else { 1.0 };
        let heard = ctx.noises.iter().find(|n| n.reaches(self.pos, mult)).map(|n| n.pos);
        if let Some(src) = heard {
            self.start_flee(src, ctx.time, events);
        }

        // Rats bolt from light (possums freeze instead, raccoons rely on
        // noise/memory).
        if self.species == Species::Rat && ctx.is_lit {
            self.start_flee(ctx.player_pos, ctx.time, events);
        }

        self.advance(ctx, events);
    }

    fn tick_friendly(&mut self, ctx: &AiCtx, events: &mut Vec<SimEvent>) {
        // Dogs patrol; cats haunt rat nodes. Neither flees — their entire
        // gameplay weight is in hit resolution and penalties.
        self.advance(ctx, events);
    }

    fn tick_zombie(&mut self, ctx: &AiCtx, events: &mut Vec<SimEvent>) {
        // Noise-seek override: latch onto the most recent audible noise.
        for n in ctx.noises {
            if n.reaches(self.pos, 1.0) {
                self.noise_seek = Some((n.pos, ctx.time + NOISE_SEEK_DURATION_S));
            }
        }
        if let Some((src, until)) = self.noise_seek {
            if ctx.time >= until {
                self.noise_seek = None;
                self.state = AiState::Idle { until: ctx.time + 2.0 };
            } else {
                self.step_toward(src, self.speed(), ctx.dt);
                return;
            }
        }
        self.advance(ctx, events);
    }

    // --- shared locomotion -------------------------------------------------

    fn advance(&mut self, ctx: &AiCtx, events: &mut Vec<SimEvent>) {
        match self.state {
            AiState::Idle { until } => {
                self.rest(ctx.dt, events);
                if ctx.time >= until && self.speed() > 0.0 {
                    let dest = self.pick_destination();
                    self.state = AiState::MoveTo { dest };
                }
            }
            AiState::MoveTo { dest } => {
                if self.step_toward(dest, self.speed(), ctx.dt) {
                    let dwell = self.dwell_seconds();
                    self.state = AiState::Idle { until: ctx.time + dwell };
                }
            }
            AiState::Flee { from, until } => {
                if ctx.time >= until {
                    self.state = AiState::Idle { until: ctx.time + 1.0 };
                } else {
                    let away = self.pos - from;
                    let dir = if away.length_squared() < 1e-6 {
                        self.random_direction()
                    } else {
                        Vec3::new(away.x, 0.0, away.z).normalize_or_zero()
                    };
                    self.pos += dir * self.speed() * 1.6 * ctx.dt;
                    self.still_time = 0.0;
                    self.bedding_emitted = false;
                }
            }
            AiState::Frozen => {
                // Motionless in the beam.
                self.rest(ctx.dt, events);
            }
        }
    }

    /// Step toward `dest` at `speed`; true when arrived.
    fn step_toward(&mut self, dest: Vec3, speed: f32, dt: f32) -> bool {
        let to = Vec3::new(dest.x - self.pos.x, 0.0, dest.z - self.pos.z);
        let dist = to.length();
        let step = speed * dt;
        self.still_time = 0.0;
        self.bedding_emitted = false;
        if dist <= step.max(0.05) {
            self.pos = Vec3::new(dest.x, self.pos.y, dest.z);
            true
        } else {
            self.pos += to / dist * step;
            false
        }
    }

    /// Accumulate rest; long rests leave warm bedding spots (SDD §2.3).
    fn rest(&mut self, dt: f32, events: &mut Vec<SimEvent>) {
        self.still_time += dt;
        if !self.bedding_emitted && self.still_time >= BEDDING_REST_S {
            self.bedding_emitted = true;
            events.push(SimEvent::HeatResidue { kind: HeatKind::Bedding, pos: self.pos });
        }
    }

    fn random_direction(&mut self) -> Vec3 {
        let a = self.rng.range(0.0, std::f32::consts::TAU);
        Vec3::new(a.cos(), 0.0, a.sin())
    }

    fn pick_destination(&mut self) -> Vec3 {
        match self.species {
            // Short feeding loop around the spawn node.
            Species::Rat => {
                let d = self.random_direction();
                self.spawn + d * self.rng.range(1.0, 6.0)
            }
            // Tree-node-biased slow wandering.
            Species::Possum => {
                if !self.waypoints.is_empty() && self.rng.chance(0.7) {
                    let i = self.rng.below(self.waypoints.len() as u64) as usize;
                    let jitter = self.random_direction() * self.rng.range(0.0, 1.5);
                    self.waypoints[i] + jitter
                } else {
                    let d = self.random_direction();
                    self.spawn + d * self.rng.range(1.0, 4.0)
                }
            }
            // Waypoint patrol.
            Species::Raccoon | Species::Dog => {
                if self.waypoints.is_empty() {
                    let d = self.random_direction();
                    self.spawn + d * self.rng.range(2.0, 8.0)
                } else {
                    let dest = self.waypoints[self.wp_idx % self.waypoints.len()];
                    self.wp_idx = (self.wp_idx + 1) % self.waypoints.len();
                    dest
                }
            }
            // Cats wander near rat nodes.
            Species::Cat => {
                if self.waypoints.is_empty() {
                    let d = self.random_direction();
                    self.spawn + d * self.rng.range(1.0, 5.0)
                } else {
                    let i = self.rng.below(self.waypoints.len() as u64) as usize;
                    self.waypoints[i] + self.random_direction() * self.rng.range(0.0, 3.0)
                }
            }
            // Slow random wander from wherever it stands.
            Species::Zombie => {
                let d = self.random_direction();
                self.pos + d * self.rng.range(3.0, 12.0)
            }
            Species::Cow | Species::Sheep => self.pos,
        }
    }

    fn dwell_seconds(&mut self) -> f32 {
        let (lo, hi) = match self.species {
            Species::Rat => (0.5, 3.0),
            Species::Possum => (4.0, 12.0),
            Species::Raccoon => (0.5, 2.0),
            Species::Dog => (0.2, 1.0),
            Species::Cat => (1.0, 5.0),
            Species::Zombie => (2.0, 6.0),
            Species::Cow | Species::Sheep => (3600.0, 7200.0),
        };
        self.rng.range(lo, hi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use da_core::weather::Forecast;
    use crate::noise::NoiseKind;

    fn ctx<'a>(
        dt: f32,
        time: f32,
        noises: &'a [NoiseEvent],
        is_lit: bool,
        weather: &'a WeatherMods,
    ) -> AiCtx<'a> {
        AiCtx { dt, time, noises, is_lit, player_pos: Vec3::ZERO, weather }
    }

    #[test]
    fn rat_feeding_loop_stays_near_spawn_and_moves_deterministically() {
        let w = Forecast::Overcast.mods();
        let spawn = Vec3::new(10.0, 0.0, 10.0);
        let mut a = Animal::new(EntityId(1), Species::Rat, spawn, None, vec![], Rng::new(5));
        let mut b = a.clone();
        let mut ev = Vec::new();
        let mut t = 0.0;
        let mut moved = false;
        for _ in 0..600 {
            t += 0.1;
            a.tick(&ctx(0.1, t, &[], false, &w), &mut ev);
            b.tick(&ctx(0.1, t, &[], false, &w), &mut ev);
            assert_eq!(a.pos, b.pos, "same seed, same path");
            if a.pos != spawn {
                moved = true;
            }
            assert!((a.pos - spawn).length() < 12.0, "short loop near spawn");
        }
        assert!(moved);
    }

    #[test]
    fn rat_flees_noise_and_respawns_fast() {
        let w = Forecast::Overcast.mods();
        let mut a =
            Animal::new(EntityId(1), Species::Rat, Vec3::ZERO, None, vec![], Rng::new(7));
        let mut ev = Vec::new();
        let boom =
            [NoiseEvent { pos: Vec3::new(3.0, 0.0, 0.0), radius_m: 40.0, kind: NoiseKind::Discharge }];
        a.tick(&ctx(0.1, 0.1, &boom, false, &w), &mut ev);
        assert!(a.is_fleeing());
        assert!(ev.iter().any(|e| matches!(e, SimEvent::PestFled { id: EntityId(1), .. })));

        a.kill(10.0);
        assert!(!a.alive);
        a.tick(&ctx(0.1, 10.0 + RAT_RESPAWN_S - 1.0, &[], false, &w), &mut ev);
        assert!(!a.alive, "not yet");
        a.tick(&ctx(0.1, 10.0 + RAT_RESPAWN_S + 0.1, &[], false, &w), &mut ev);
        assert!(a.alive, "fast respawn");
        assert_eq!(a.pos, a.spawn);
    }

    #[test]
    fn cow_is_static_and_eventually_leaves_a_bedding_spot() {
        let w = Forecast::Overcast.mods();
        let pos = Vec3::new(4.0, 0.0, 4.0);
        let mut a = Animal::new(EntityId(2), Species::Cow, pos, None, vec![], Rng::new(1));
        let mut ev = Vec::new();
        let mut t = 0.0;
        for _ in 0..400 {
            t += 0.1;
            a.tick(&ctx(0.1, t, &[], false, &w), &mut ev);
        }
        assert_eq!(a.pos, pos, "livestock is static");
        assert!(ev.iter().any(|e| matches!(
            e,
            SimEvent::HeatResidue { kind: HeatKind::Bedding, .. }
        )));
    }

    #[test]
    fn dog_patrols_waypoints() {
        let w = Forecast::Overcast.mods();
        let wps = vec![Vec3::new(10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 10.0)];
        let mut a =
            Animal::new(EntityId(3), Species::Dog, Vec3::ZERO, None, wps.clone(), Rng::new(2));
        let mut ev = Vec::new();
        let mut t = 0.0;
        let mut reached = [false, false];
        for _ in 0..2400 {
            t += 0.1;
            a.tick(&ctx(0.1, t, &[], false, &w), &mut ev);
            for (i, wp) in wps.iter().enumerate() {
                if (a.pos - *wp).length() < 0.5 {
                    reached[i] = true;
                }
            }
        }
        assert!(reached[0] && reached[1], "dog visits its patrol route: {reached:?}");
    }
}
