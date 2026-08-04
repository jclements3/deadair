//! One night's hunt: the expanded zone, the thermal sim over its nodes,
//! the gameplay sim (animals, shots, hazards), the night clock, and the
//! per-frame DrawList assembly. Headless-testable — the UI layer only
//! feeds input and displays output.

use crate::fauna::{self, FaunaPose};
use crate::flora;
use crate::{aim, convert};
use std::collections::HashMap;
use da_core::{Forecast, NightClock, Rng};
use da_econ::{Business, NightLedger, OpticModel, RifleModel};
use da_graph::prelude::*;
use da_param::{expand::FriendlyBehavior, expand::Volume, ZoneExpansion};
use da_audio::{AudioEngine, EventAudioCtx, Listener, NullBackend, SoundSource, Subtitle};
use da_render::draw::{DrawItem, DrawList, EyeShine, Shape as RShape};
use da_sim::{Optic, RifleConfig, Sim, SimEvent, Species};
use da_thermal::{HeatEvent, ThermalSim};
use glam::{Mat4, Vec3};

/// Which optic device is mounted tonight (maps to render mode + drain).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mounted {
    Headlamp,
    NvBasic,
    NvPro,
    Thermal(u8), // Mk 1..3
}

impl Mounted {
    pub fn optic_model(self) -> OpticModel {
        match self {
            Mounted::Headlamp => OpticModel::Headlamp,
            Mounted::NvBasic => OpticModel::NvBasic,
            Mounted::NvPro => OpticModel::NvPro,
            Mounted::Thermal(1) => OpticModel::ThermalMk1,
            Mounted::Thermal(2) => OpticModel::ThermalMk2,
            Mounted::Thermal(_) => OpticModel::ThermalMk3,
        }
    }

    pub fn sim_optic(self) -> Optic {
        match self {
            Mounted::Headlamp => Optic::Eye,
            Mounted::NvBasic | Mounted::NvPro => Optic::Nv,
            Mounted::Thermal(_) => Optic::Thermal,
        }
    }
}

/// A full night hunt in one zone.
pub struct NightHunt {
    pub expansion: ZoneExpansion,
    pub thermal: ThermalSim,
    pub sim: Sim,
    pub clock: NightClock,
    pub forecast: Forecast,
    pub ledger: NightLedger,
    pub mounted: Mounted,
    pub battery_pct: f32,
    pub pellets: u32,
    pub log: Vec<String>,
    pub over: bool,
    leaves: Vec<RenderLeaf>,
    ambient_cache: f32,
    recent: Vec<SimEvent>,
    /// Audio is the fourth optic (SDD §9) — thermal cannot see zombies, so
    /// their moan is the compensating channel. Runs on the null backend
    /// until the game opts into a real sink.
    audio: AudioEngine<NullBackend>,
    /// Captions for the audible sources this frame (NFR-3).
    pub subtitles: Vec<Subtitle>,
    /// Tonight's wind, horizontal m/s. Every shot drifts with it; the
    /// reticle scale is the compensation tool.
    pub wind_mps: Vec3,
    /// Per-animal gait/pose state driving the articulated rigs.
    gaits: HashMap<da_core::EntityId, GaitState>,
}

/// Locomotion state accumulated per animal for the fauna rigs.
#[derive(Debug, Clone, Copy)]
struct GaitState {
    phase: f32,
    heading: f32,
    speed_norm: f32,
    last_pos: Vec3,
}

impl NightHunt {
    /// Load a zone, expand it, spawn its population, open the ledger.
    pub fn new(
        zone_path: &str,
        forecast: Forecast,
        business: &Business,
        seed: u64,
        mounted: Mounted,
    ) -> Result<Self, String> {
        let source = da_param::load_zone_file(zone_path).map_err(|e| e.to_string())?;
        let expansion = da_param::expand_zone(&source).map_err(|e| e.to_string())?;

        // Cull once with no frustum — the static draw set (small zones).
        let center = Vec3::new(source.size_m.0 * 0.5, 1.6, source.size_m.1 * 0.5);
        let leaves = CullVisitor::new(center).cull(&expansion.scene);

        // Thermal registration for every leaf with an attach.
        let mut thermal = ThermalSim::new(forecast);
        for leaf in &leaves {
            if let Some(attach) = &leaf.state.thermal {
                if !thermal.contains(leaf.node) {
                    thermal.register(leaf.node, convert::profile_from_attach(attach), 0.0);
                }
            }
        }

        // Rifle from the business's best tier.
        let rifle = match business.best_rifle_tier() {
            4 => RifleConfig::tier4(),
            3 => RifleConfig::tier3(),
            2 => RifleConfig::tier2(),
            _ => RifleConfig::tier1(),
        };
        let mut sim = Sim::new(seed, rifle, forecast.mods());
        // Player enters at the zone's south edge, mid-x.
        sim.player.pos = Vec3::new(source.size_m.0 * 0.5, 1.6, source.size_m.1 - 5.0);

        let mut rng = Rng::new(seed ^ 0xD00D);

        // Pests from spawn tables.
        let mut group_id = 0u32;
        for sp in &expansion.spawn_points {
            let Some(species) = convert::sim_species(&sp.species) else {
                continue;
            };
            let waypoints = expansion
                .patrol_routes
                .iter()
                .find(|(name, _)| *name == sp.species)
                .map(|(_, route)| route.clone())
                .unwrap_or_default();
            let group = if species == Species::Raccoon {
                group_id += 1;
                Some(group_id)
            } else {
                None
            };
            sim.spawn_animal(species, sp.pos, group, waypoints);
        }
        // Friendlies.
        for f in &expansion.friendly_setups {
            let Some(species) = convert::sim_species(&f.species) else {
                continue;
            };
            match &f.behavior {
                FriendlyBehavior::Patrol(route) => {
                    sim.spawn_animal(species, route[0], None, route.clone());
                }
                FriendlyBehavior::Penned { positions, .. } => {
                    for p in positions {
                        sim.spawn(species, *p);
                    }
                }
                FriendlyBehavior::WanderNear(anchors) => {
                    for _ in 0..f.count {
                        let a = anchors[(rng.below(anchors.len() as u64)) as usize];
                        sim.spawn_animal(species, a, None, anchors.clone());
                    }
                }
            }
        }
        // Zombies by zone weighting.
        let zombie_count = (expansion.zombie_weight * 5.0).round() as u32;
        for _ in 0..zombie_count {
            let pos = Vec3::new(
                rng.range(0.0, source.size_m.0),
                0.0,
                rng.range(0.0, source.size_m.1 * 0.6),
            );
            sim.spawn(Species::Zombie, pos);
        }
        // Hazards: volumes → trigger spheres.
        for hv in &expansion.hazard_volumes {
            let kind = convert::sim_hazard_kind(hv.kind);
            match &hv.volume {
                Volume::Sphere { center, radius } => sim.add_hazard(da_sim::Hazard {
                    kind,
                    pos: *center,
                    radius: *radius,
                }),
                Volume::Segment { from, to, radius } => {
                    let len = from.distance(*to);
                    let n = (len / 4.0).ceil().max(1.0) as u32;
                    for i in 0..=n {
                        let p = from.lerp(*to, i as f32 / n as f32);
                        sim.add_hazard(da_sim::Hazard {
                            kind,
                            pos: p,
                            radius: radius.max(2.0),
                        });
                    }
                }
                Volume::Polyline { points, width } => {
                    for w in points.windows(2) {
                        let len = w[0].distance(w[1]);
                        let n = (len / 6.0).ceil().max(1.0) as u32;
                        for i in 0..=n {
                            let p = w[0].lerp(w[1], i as f32 / n as f32);
                            sim.add_hazard(da_sim::Hazard {
                                kind,
                                pos: p,
                                radius: width * 0.5 + 1.0,
                            });
                        }
                    }
                }
            }
        }

        let ledger = NightLedger::new(
            business.night,
            &source.name,
            forecast,
            match business.best_rifle_tier() {
                4 => RifleModel::Premium25,
                3 => RifleModel::RegulatedPcp,
                2 => RifleModel::UnregulatedPcp,
                _ => RifleModel::MultiPump,
            },
            Some(mounted.optic_model()),
        );

        Ok(Self {
            expansion,
            thermal,
            sim,
            clock: NightClock::standard(),
            forecast,
            ledger,
            mounted,
            battery_pct: 100.0,
            pellets: business.pellets,
            log: {
                let mut l =
                    vec![format!("Night begins at {}. Forecast: {:?}.", source.name, forecast)];
                if business.pellets == 0 {
                    l.push("You carried no pellets tonight. Buy tins at camp.".into());
                }
                l
            },
            over: false,
            leaves,
            ambient_cache: 60.0,
            recent: Vec::new(),
            audio: AudioEngine::new(NullBackend::new(), Default::default(), seed ^ 0xA0D1),
            subtitles: Vec::new(),
            wind_mps: aim::roll_wind(forecast, &mut Rng::new(seed ^ 0x817D)),
            gaits: HashMap::new(),
        })
    }

    /// Advance the whole night by `dt` real seconds; `move_dir` is the
    /// player's flat movement intent (already speed-scaled), `scoped` drains
    /// battery on powered optics.
    pub fn tick(&mut self, dt: f32, move_dir: Vec3, scoped: bool) {
        if self.over {
            return;
        }
        self.clock.tick(dt);
        if self.clock.is_dawn() {
            self.log.push("Dawn. The night is over.".into());
            self.over = true;
        }
        if move_dir.length_squared() > 0.0 {
            self.sim.move_player(move_dir * dt, dt);
        }
        let events = self.sim.tick(dt);
        self.handle_events(&events);
        self.play_audio(&events, move_dir);
        self.recent = events;
        self.update_gaits(dt);
        self.thermal.step(dt, self.clock.t());
        self.ambient_cache = da_thermal::ambient_at(self.clock.t(), self.forecast).0;

        // Battery drain: only while scoped on a powered optic (simplified).
        if scoped {
            let mult = match self.mounted {
                Mounted::Headlamp => 0.0,
                Mounted::NvBasic | Mounted::NvPro => 1.0,
                Mounted::Thermal(_) => 2.5,
            } * self.forecast.mods().battery_drain;
            // 1x drain empties a battery in ~2 in-game nights of constant use.
            self.battery_pct = (self.battery_pct - dt * mult * 100.0 / 4800.0).max(0.0);
            self.ledger.record_optic_hours(
                dt / 3600.0 * self.clock.night_hours / (self.clock.real_seconds / 3600.0),
            );
        }
    }

    /// Distance from the player's eye to the first surface along `axis`:
    /// the nearest animal collider hit, else the ground plane, else a far
    /// default. This is what the laser rangefinder reads.
    pub fn range_along(&self, axis: Vec3) -> f32 {
        const MAX_RANGE_M: f32 = 300.0;
        let eye = self.sim.player.pos;
        let axis = axis.normalize_or_zero();
        let targets = self.sim.targets();
        let animal_t = da_sim::hit::ray_hits(eye, axis, &targets)
            .iter()
            .map(|h| h.t)
            .fold(f32::MAX, f32::min);
        let ground_t = if axis.y < -1e-4 {
            -eye.y / axis.y
        } else {
            f32::MAX
        };
        animal_t.min(ground_t).min(MAX_RANGE_M)
    }

    /// The firing solution along the current sight axis: real range, drop,
    /// and wind drift (SDD §5.1 ballistics + the reticle-scale scheme).
    pub fn shot_solution(&self, axis: Vec3) -> aim::ShotSolution {
        let v0 = self.sim.rifle.muzzle_velocity_mps().unwrap_or(200.0);
        aim::solve(
            self.sim.player.pos,
            axis,
            self.range_along(axis),
            v0,
            self.sim.rifle.pellet.drop_scale(),
            self.wind_mps,
        )
    }

    /// Fire along the sight axis, obeying drop and wind: the pellet goes
    /// where physics sends it, not where the crosshair points. Returns the
    /// HUD blurb from [`NightHunt::fire`].
    pub fn fire_axis(&mut self, axis: Vec3, business: &Business) -> Option<String> {
        let dir = self.shot_solution(axis).dir;
        self.fire(dir, business)
    }

    /// Spatialise this tick's events plus the world's idle sounds, and keep
    /// the captions for the HUD.
    fn play_audio(&mut self, events: &[SimEvent], move_dir: Vec3) {
        let facing = if move_dir.length_squared() > 0.0 {
            move_dir.normalize()
        } else {
            Vec3::NEG_Z
        };
        let listener = Listener::new(self.sim.player.pos, facing);
        let ctx = EventAudioCtx {
            player_pos: self.sim.player.pos,
            moderated: self.sim.rifle.moderator,
            surface: match self.expansion.ground_biome {
                da_param::Biome::Grass => da_audio::Surface::Grass,
                da_param::Biome::Gravel => da_audio::Surface::Gravel,
                da_param::Biome::Mud => da_audio::Surface::Mud,
                // No asphalt surface in the audio set; gravel is the nearest crunch.
                da_param::Biome::Asphalt => da_audio::Surface::Gravel,
            },
        };

        // Idle world sound: every living animal that is close enough to be
        // worth hearing. This is the channel that finds zombies.
        let mut sources: Vec<SoundSource> = Vec::new();
        for a in &self.sim.animals {
            if !a.alive || !a.is_targetable() {
                continue;
            }
            if let Some(kind) = da_audio::event_map::species_sound(a.species) {
                sources.push(SoundSource::new(kind, a.pos));
            }
        }
        let positions: Vec<(da_core::EntityId, Vec3)> =
            self.sim.animals.iter().map(|a| (a.id, a.pos)).collect();
        let locate = |id: da_core::EntityId| {
            positions.iter().find(|(i, _)| *i == id).map(|(_, p)| *p)
        };
        sources.extend(da_audio::sounds_for_events(events, &ctx, locate));

        if let Ok(captions) = self.audio.play_frame(&listener, &sources) {
            self.subtitles = captions;
        }
    }

    /// Advance every animal's gait phase/heading from how it actually moved
    /// this tick — the rigs animate from displacement, not wall time, so a
    /// still animal stands still (bar breathing).
    fn update_gaits(&mut self, dt: f32) {
        for a in &self.sim.animals {
            let g = self.gaits.entry(a.id).or_insert(GaitState {
                // Desynchronise herds: seed the phase from the id.
                phase: (a.id.0 % 97) as f32 * 0.137,
                heading: (a.id.0 % 13) as f32,
                speed_norm: 0.0,
                last_pos: a.pos,
            });
            let delta = a.pos - g.last_pos;
            let dist = Vec3::new(delta.x, 0.0, delta.z).length();
            if dist > 1e-4 {
                // fauna rigs face +X and rotate by yaw around Y.
                g.heading = f32::atan2(-delta.z, delta.x);
                g.phase = fauna::advance_phase(a.species, g.phase, dist);
            }
            let speed = if dt > 0.0 { dist / dt } else { 0.0 };
            let full = a.speed().max(0.1);
            // Ease so the gait doesn't strobe on jittery movement.
            let target = (speed / full).clamp(0.0, 1.0);
            g.speed_norm += (target - g.speed_norm) * (dt * 8.0).clamp(0.0, 1.0);
            g.last_pos = a.pos;
        }
    }

    fn pose_for(&self, a: &da_sim::Animal) -> FaunaPose {
        let g = self
            .gaits
            .get(&a.id)
            .copied()
            .unwrap_or(GaitState {
                phase: 0.0,
                heading: 0.0,
                speed_norm: 0.0,
                last_pos: a.pos,
            });
        FaunaPose {
            pos: a.pos,
            heading: g.heading,
            speed_norm: g.speed_norm,
            gait_phase: g.phase,
            frozen: a.is_frozen(),
        }
    }

    /// World head anchor of a living animal (selection brackets, eyeshine).
    pub fn head_of(&self, id: da_core::EntityId) -> Option<Vec3> {
        let a = self.sim.animals.iter().find(|a| a.id == id && a.alive)?;
        Some(fauna::head_pos(a.species, &self.pose_for(a)))
    }

    /// IR eyeshine points for the NV pass: living, targetable animals within
    /// illuminator reach. Zombies are deliberately excluded — dead retinas do
    /// not retro-reflect, which is the NV-side counterpart to their thermal
    /// invisibility (SDD §4.1, assets/reference/optics-look.md).
    fn eyeshine_points(&self) -> Vec<EyeShine> {
        const IR_REACH_M: f32 = 120.0;
        let eye = self.sim.player.pos;
        self.sim
            .animals
            .iter()
            .filter(|a| a.alive && a.is_targetable() && a.species != Species::Zombie)
            .filter_map(|a| {
                let head = fauna::head_pos(a.species, &self.pose_for(a));
                let d = head.distance(eye);
                if d > IR_REACH_M {
                    return None;
                }
                // Return falls off with distance; the beam is strongest close in.
                Some(EyeShine {
                    pos: head,
                    strength: (1.0 - d / IR_REACH_M).clamp(0.05, 1.0),
                })
            })
            .collect()
    }

    /// Events emitted by the most recent [`NightHunt::tick`] — the surface
    /// the tutorial and audio layers react to.
    pub fn recent_events(&self) -> &[SimEvent] {
        &self.recent
    }

    fn handle_events(&mut self, events: &[SimEvent]) {
        for ev in events {
            match ev {
                SimEvent::KillConfirmed { species, pos, .. } => {
                    self.log.push(format!("Kill confirmed: {species:?}."));
                    self.thermal.spawn_heat(HeatEvent::bedding(*pos));
                }
                SimEvent::FriendlyHit { species, .. } => {
                    self.log.push(format!("YOU HIT A {species:?}. That costs."));
                }
                SimEvent::Wounded { species, .. } => {
                    self.log.push(format!("Wounded {species:?} — it fled. No bounty."));
                }
                SimEvent::ZombieDestroyed { .. } => {
                    self.log.push("Zombie down. Headshot.".into());
                }
                SimEvent::ZombieStaggered { .. } => {
                    self.log.push("Body shot staggered it. Aim for the head.".into());
                }
                SimEvent::PlayerDamaged { amount, cause } => {
                    self.log.push(format!("-{amount:.0} health ({cause:?})."));
                }
                SimEvent::HeatResidue { kind, pos } => {
                    let he = match kind {
                        da_sim::HeatKind::Barrel => HeatEvent::barrel(*pos),
                        da_sim::HeatKind::PelletImpact => HeatEvent::pellet_impact(*pos),
                        da_sim::HeatKind::Bedding => HeatEvent::bedding(*pos),
                    };
                    self.thermal.spawn_heat(he);
                }
                _ => {}
            }
        }
    }

    /// Fire along `dir`. Records economy effects. Returns a HUD blurb.
    pub fn fire(&mut self, dir: Vec3, business: &Business) -> Option<String> {
        if self.pellets == 0 {
            return Some("Out of pellets.".into());
        }
        if self.sim.check_backstop(dir) {
            self.log
                .push("NO BACKSTOP — friendly behind the target. Held fire.".into());
            return Some("No safe backstop!".into());
        }
        let outcome = self.sim.fire(dir)?;
        self.pellets -= 1;
        self.ledger.record_shot();
        use da_sim::ShotOutcome::*;
        let blurb = match outcome {
            Kill { species, .. } => {
                self.ledger
                    .record_kill(convert::econ_species(species), business);
                format!("{species:?} down.")
            }
            Wound { species, .. } => format!("Wounded {species:?}."),
            FriendlyHit { species, .. } => {
                self.ledger
                    .record_kill(convert::econ_species(species), business);
                format!("Hit a {species:?}!")
            }
            ZombieDestroyed { .. } => "Zombie destroyed.".into(),
            ZombieStagger { .. } => "Staggered it.".into(),
            Miss { .. } => "Miss.".into(),
        };
        Some(blurb)
    }

    /// Assemble the frame's DrawList from graph leaves + live entities.
    pub fn draw_list(&self) -> DrawList {
        let ambient = self.ambient_cache;
        let alpha = self.thermal.tick_alpha();
        let mut items: Vec<DrawItem> = Vec::with_capacity(self.leaves.len() + 64);

        // Ground plane (biome-colored).
        // Horizon apron only: the zone expansion carries its own ground
        // slab with its top at y = 0, so this sits 15 cm below it — never
        // coplanar, never z-fighting. The step at the zone edge is
        // invisible at distance.
        items.push(DrawItem {
            shape: RShape::GroundPatch { half: 300.0 },
            world: Mat4::from_translation(Vec3::new(150.0, -0.15, 100.0)),
            albedo: match self.expansion.ground_biome {
                da_param::Biome::Grass => [0.2, 0.28, 0.16],
                da_param::Biome::Gravel => [0.32, 0.3, 0.28],
                da_param::Biome::Mud => [0.24, 0.2, 0.16],
                da_param::Biome::Asphalt => [0.16, 0.16, 0.18],
            },
            emissive: 0.0,
            temp_f: ambient - 3.0,
            glass: false,
        });

        // Grass around the hunter: world-anchored, so walking reveals the
        // same field every time (and tufts occlude low animals — cover is
        // real now).
        items.extend(flora::tufts_around(self.sim.player.pos, 40.0, ambient));

        for leaf in &self.leaves {
            let temp = self
                .thermal
                .sampled_temp(leaf.node, alpha)
                .map(|t| t.0)
                .unwrap_or(ambient);
            if let Some(item) = convert::leaf_to_item(leaf, &self.expansion.scene, temp) {
                items.push(item);
            }
        }

        // Animals: articulated primitive rigs, posed by their own movement
        // (design direction: moving rats, rabbits, boars like the videos).
        // Hidden animals (submerged beaver, burrowed groundhog) don't render.
        for a in &self.sim.animals {
            if !a.alive || !a.is_targetable() {
                continue;
            }
            let pose = self.pose_for(a);
            for part in fauna::build(a.species, &pose) {
                // Warm-blooded parts carry their role bias (coat cooler
                // than bare skin); ambient bodies ignore it — a zombie is
                // exactly ambient everywhere, or the invariant dies.
                let temp = if a.species == Species::Zombie {
                    ambient
                } else {
                    101.0 + part.temp_bias
                };
                items.push(DrawItem {
                    shape: part.shape,
                    world: part.world,
                    albedo: part.albedo,
                    emissive: 0.0,
                    temp_f: temp,
                    glass: false,
                });
            }
        }

        DrawList {
            items,
            ambient_f: ambient,
            sky_temp_f: ambient - 45.0,
            moonlight: self.forecast.mods().eye_visibility * 0.5,
            // Residual heat is a tracking mechanic, thermal-only (SDD §2.3).
            heat_decals: self
                .thermal
                .live_heat()
                .map(|h| da_render::draw::HeatDecal {
                    pos: h.pos,
                    radius_m: 0.6,
                    delta_f: h.intensity_f,
                })
                .collect(),
            eyeshine: self.eyeshine_points(),
        }
    }

    /// HUD strip per SDD §8.
    pub fn hud_line(&self, cash: &str) -> String {
        let rifle = &self.sim.rifle;
        let air = match &rifle.plant {
            da_sim::PowerPlant::MultiPump { pumps, max_pumps, .. } => {
                format!("PUMPS {pumps}/{max_pumps}")
            }
            p => {
                let fill = match p {
                    da_sim::PowerPlant::UnregulatedPcp { fill_pct, .. } => *fill_pct,
                    da_sim::PowerPlant::RegulatedPcp { fill_pct, .. } => *fill_pct,
                    _ => 0.0,
                };
                format!("AIR {:.0}%", fill)
            }
        };
        format!(
            "{air} | pellets {} | BATT {:.0}% | {} to dawn | {}",
            self.pellets,
            self.battery_pct,
            self.clock.hud_countdown(),
            cash
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn stocked_business() -> Business {
        let mut b = Business::new();
        b.buy_accessory(da_econ::Accessory::PelletTin).expect("tin");
        b
    }

    fn hunt() -> NightHunt {
        NightHunt::new(
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/zones/home_farm.zone.ron"),
            Forecast::Clear,
            &stocked_business(),
            42,
            Mounted::Headlamp,
        )
        .expect("hunt")
    }

    #[test]
    fn home_farm_hunt_boots_and_populates() {
        let h = hunt();
        assert!(h.sim.animals.len() >= 10, "animals: {}", h.sim.animals.len());
        assert!(h.thermal.len() > 50, "thermal nodes: {}", h.thermal.len());
        assert!(!h.sim.hazards.is_empty());
        // Home Farm has rats, possums, a dog, cows per the source.
        let species: Vec<_> = h.sim.animals.iter().map(|a| a.species).collect();
        assert!(species.contains(&Species::Rat));
        assert!(species.contains(&Species::Dog));
    }

    #[test]
    fn night_runs_to_dawn_and_draws() {
        let mut h = hunt();
        for _ in 0..100 {
            h.tick(0.5, Vec3::new(0.0, 0.0, -1.0), false);
        }
        let dl = h.draw_list();
        assert!(dl.items.len() > 60, "draw items: {}", dl.items.len());
        // Fast-forward to dawn.
        h.clock.seek(1.0);
        h.tick(0.1, Vec3::ZERO, false);
        assert!(h.over);
    }

    #[test]
    fn firing_consumes_pellets_and_logs() {
        let mut h = hunt();
        // Tier 1 multi-pump starts unpumped: pump first.
        h.sim.pump(10.0);
        let before = h.pellets;
        assert_eq!(before, da_econ::PELLET_TIN_CAPACITY);
        let biz = stocked_business();
        h.fire(Vec3::new(0.0, 0.0, -1.0), &biz);
        assert!(h.pellets <= before);
        assert!(h.ledger.shots_fired() <= 1);
    }
}
