//! `--demo`: the app plays itself like a user for a conference reel.
//!
//! One deterministic script drives both consumers: the live GUI attract
//! mode and the headless film render (`--demo-film`). Every segment runs
//! the REAL systems — the calibration range, zone expansion, the night
//! sim, the economy — nothing is canned footage. The only cinematic
//! license is time: "nights later" montage cards compress the earning
//! curve so the upgrade arc fits a reel (the per-night numbers shown are
//! the ledger's own).
//!
//! Determinism holds film-grade: same build, same frames.

use crate::aim;
use crate::camp::ZoneCatalog;
use crate::camp3d::CampWorld;
use crate::fauna;
use crate::hunt::{Mounted, NightHunt};
use crate::range::RangeState;
use da_core::Forecast;
use da_econ::{Accessory, Business, OpticModel, RifleModel};
use da_render::draw::{Camera, DrawList};
use da_render::{OpticMode, OpticSettings, ThermalPalette};
use da_sim::Species;
use glam::Vec3;

/// Everything one demo frame needs rendered + captioned.
pub struct DemoFrame {
    pub list: DrawList,
    pub cam: Camera,
    pub settings: OpticSettings,
    /// Caption lines, top to bottom (burned over the lower third).
    pub captions: Vec<String>,
    /// Magnification for the corner readout (1.0 = unscoped).
    pub mag: f32,
    /// Stellar-style HUD boxes when a thermal is up (counter, info).
    pub thermal_hud: Option<(u32, String)>,
}

enum SegKind {
    /// Title / montage card: black frame, captions only.
    Card,
    /// The calibration range under the Stellar-class boot optic.
    Range(Box<RangeState>),
    /// Aerial crane over a zone: contract context.
    Fly(Box<NightHunt>),
    /// A scripted first-person hunt.
    Hunt(Box<HuntScript>),
    /// Camp shelf glide with scripted purchases.
    Camp {
        world: Box<CampWorld>,
        /// (at_t, cash_grant_cents, buys, caption) — grants model the
        /// montage nights; buys go through the real store.
        script: Vec<CampBeat>,
        fired: Vec<bool>,
    },
}

struct CampBeat {
    at: f32,
    grant_cents: i64,
    rifle: Option<RifleModel>,
    optic: Option<OpticModel>,
    tins: u32,
    caption: String,
}

struct Segment {
    dur: f32,
    kind: SegKind,
    /// Static caption track: (from_t, to_t, line).
    captions: Vec<(f32, f32, String)>,
}

// ---------------------------------------------------------------------------
// Scripted hunter
// ---------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
enum Phase {
    Seek,
    Walk,
    ScopeIn,
    Settle,
    Recover,
}

struct HuntScript {
    hunt: NightHunt,
    mounted: Mounted,
    yaw: f32,
    pitch: f32,
    mag: f32,
    phase: Phase,
    phase_t: f32,
    /// Prefer this species when choosing marks (None = any pest).
    prefer: Option<Species>,
    engage_m: f32,
    kill_feed: Vec<(String, f32)>,
    shots: u32,
}

impl HuntScript {
    fn new(
        zone: &str,
        forecast: Forecast,
        business: &Business,
        seed: u64,
        mounted: Mounted,
        prefer: Option<Species>,
        engage_m: f32,
    ) -> Result<Self, String> {
        let hunt = NightHunt::new(zone, forecast, business, seed, mounted)?;
        Ok(HuntScript {
            hunt,
            mounted,
            yaw: 0.0,
            pitch: 0.0,
            mag: 1.0,
            phase: Phase::Seek,
            phase_t: 0.0,
            prefer,
            engage_m,
            kill_feed: Vec::new(),
            shots: 0,
        })
    }

    fn scoped(&self) -> bool {
        matches!(self.phase, Phase::ScopeIn | Phase::Settle) && self.mag > 1.2
    }

    fn view_axis(&self) -> Vec3 {
        Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.sin() * self.pitch.cos(),
        )
    }

    /// Pick the mark: nearest preferred species, else nearest pest.
    fn mark(&self) -> Option<da_sim::Target> {
        let eye = self.hunt.sim.player.pos;
        let targets = self.hunt.rig_targets();
        let pick = |want: Option<Species>| {
            targets
                .iter()
                .filter(|t| match want {
                    Some(s) => t.species == s,
                    None => t.species.is_pest(),
                })
                .min_by(|a, b| {
                    a.pos
                        .distance(eye)
                        .partial_cmp(&b.pos.distance(eye))
                        .expect("finite")
                })
                .cloned()
        };
        pick(self.prefer).or_else(|| pick(None))
    }

    fn tick(&mut self, dt: f32, business: &Business) {
        self.phase_t += dt;
        for k in &mut self.kill_feed {
            k.1 += dt;
        }
        self.kill_feed.retain(|k| k.1 < 4.0);

        let Some(mark) = self.mark() else {
            self.hunt.tick(dt, Vec3::ZERO, false);
            return;
        };
        let eye = self.hunt.sim.player.pos;
        let head = mark.head.center;
        let dist = head.distance(eye);
        let to = (head - eye).normalize_or_zero();
        let want_yaw = to.z.atan2(to.x);
        let want_pitch = to.y.asin();

        // Human-speed view easing: fast while walking, glacial when settled.
        let rate = match self.phase {
            Phase::Settle => 6.0,
            Phase::Recover => 1.2,
            _ => 3.5,
        };
        let f = 1.0 - (-rate * dt).exp();
        let mut dy = want_yaw - self.yaw;
        while dy > std::f32::consts::PI {
            dy -= std::f32::consts::TAU;
        }
        while dy < -std::f32::consts::PI {
            dy += std::f32::consts::TAU;
        }
        self.yaw += dy * f;
        self.pitch += (want_pitch - self.pitch) * f;

        let mut move_dir = Vec3::ZERO;
        match self.phase {
            Phase::Seek => {
                self.phase = if dist > self.engage_m { Phase::Walk } else { Phase::ScopeIn };
                self.phase_t = 0.0;
            }
            Phase::Walk => {
                let flat = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
                move_dir = flat * 3.2;
                if dist <= self.engage_m {
                    self.phase = Phase::ScopeIn;
                    self.phase_t = 0.0;
                }
            }
            Phase::ScopeIn => {
                let want_mag = (dist / 9.0).clamp(2.0, 14.5);
                self.mag += (want_mag - self.mag) * (1.0 - (-5.0 * dt).exp());
                if self.phase_t > 1.0 {
                    self.phase = Phase::Settle;
                    self.phase_t = 0.0;
                }
            }
            Phase::Settle => {
                // Keep the multi-pump charged like a player would.
                if !self.hunt.sim.rifle.plant.can_fire() {
                    self.hunt.sim.pump(dt * 4.0);
                } else if self.phase_t > 1.3 {
                    let err = (want_yaw - self.yaw).abs() + (want_pitch - self.pitch).abs();
                    if err < 0.004 {
                        let axis = self.view_axis();
                        if self.hunt.sim.check_backstop(axis) {
                            move_dir = Vec3::new(1.5, 0.0, 0.0); // sidestep
                        } else {
                            let kills = self.hunt.ledger.confirmed_kills().len();
                            let blurb = self.hunt.fire_axis(axis, business);
                            self.shots += 1;
                            let landed =
                                self.hunt.ledger.confirmed_kills().len() > kills;
                            if let Some(b) = blurb {
                                let line = if landed || b.contains("destroyed") {
                                    format!("{b}  ·  {dist:.0} m")
                                } else {
                                    b
                                };
                                self.kill_feed.push((line, 0.0));
                            }
                            self.phase = Phase::Recover;
                            self.phase_t = 0.0;
                        }
                    }
                }
            }
            Phase::Recover => {
                self.mag += (1.0 - self.mag) * (1.0 - (-4.0 * dt).exp());
                if self.phase_t > 2.2 {
                    self.phase = Phase::Seek;
                    self.phase_t = 0.0;
                }
            }
        }
        self.hunt.tick(dt, move_dir, self.scoped());
    }

    fn frame(&self, t_seg: f32) -> (DrawList, Camera, OpticSettings, f32, Vec<String>) {
        let eye = self.hunt.sim.player.pos;
        // Sway rides the view when scoped — damped, like a held breath.
        let (sy, sp) = if self.scoped() {
            let amp = if self.phase == Phase::Settle { 0.35 } else { 1.0 };
            let o = aim::sway_offset(t_seg, 7, 0.002 * amp);
            (o.x, o.y)
        } else {
            (0.0, 0.0)
        };
        let yaw = self.yaw + sy;
        let pitch = (self.pitch + sp).clamp(-1.4, 1.4);
        let axis = Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        );
        let mag = if self.scoped() { self.mag } else { 1.0 };
        let cam = Camera {
            eye,
            look: eye + axis * 50.0,
            up: Vec3::Y,
            fov_y_deg: aim::fov_for_mag(mag),
            aspect: 1.0,
        };
        let settings = optic_for(self.mounted, self.scoped());
        let feed: Vec<String> = self.kill_feed.iter().map(|k| k.0.clone()).collect();
        (self.hunt.draw_list(), cam, settings, mag, feed)
    }
}

/// Render settings for a mounted optic — scoped uses the device pipeline,
/// unscoped walks on the eye (or goggles when the business owns them).
fn optic_for(mounted: Mounted, scoped: bool) -> OpticSettings {
    let mut s = OpticSettings::default();
    if scoped {
        match mounted {
            Mounted::Headlamp => {
                s.mode = OpticMode::Eye;
                s.eye_exposure = 1.6;
            }
            Mounted::NvBasic | Mounted::NvPro => {
                s.mode = OpticMode::Nv;
                s.scope_mask = true;
                s.sensor_res = Some(720);
            }
            Mounted::Thermal(mk) => {
                s.mode = OpticMode::Thermal;
                s.palette = ThermalPalette::WhiteHot;
                s.scope_mask = true;
                s.sensor_res = Some(match mk {
                    1 => 192,
                    2 => 288,
                    _ => 480,
                });
            }
        }
    } else {
        s.mode = OpticMode::Eye;
        s.eye_exposure = 1.35;
    }
    s
}

// ---------------------------------------------------------------------------
// The director
// ---------------------------------------------------------------------------

pub struct DemoDirector {
    pub business: Business,
    segments: Vec<Segment>,
    idx: usize,
    t: f32,
    frame_seed: u32,
    range_shot_t: f32,
}

/// Side-panel narration for each reel segment: (title, bullet lines).
/// Indexed by segment position — keep in step with the `segments.push`
/// order in `DemoDirector::new` (checked by a debug_assert there).
const PROMO: &[(&str, &[&str])] = &[
    (
        "The pitch",
        &[
            "Every frame is the live engine — nothing pre-rendered",
            "One binary: the game, the editor, and this reel",
        ],
    ),
    (
        "Sensor-true optics",
        &[
            "288-class thermal core, device AGC, real refresh",
            "Checkerboards and picket fence: shimmer is measured, not hoped away",
            "--calibrate rates any machine in rabbits",
        ],
    ),
    (
        "Parametric worlds",
        &[
            "Zones are text: same source + seed = the identical farm, every run",
            "Props authored in a CSG modeling language (.vim)",
            "Every pest carries a bounty — contracts drive the night",
        ],
    ),
    (
        "Digital NV",
        &[
            "Eyeshine: retro-reflecting eyes give animals away first",
            "Multi-pump .22 — pump management is part of the shot",
        ],
    ),
    (
        "A real P&L",
        &[
            "Bounties in, pellets out — buy wrong, go bankrupt",
            "The store is real: purchases change the next hunt",
        ],
    ),
    (
        "White-hot 288",
        &[
            "Matched to real scope footage, blob bloom to AGC lerp",
            "Covey behavior: drop one, the rest freeze",
        ],
    ),
    (
        "The gear ladder",
        &[
            "Hogs need 30 FPE — power, glass, and licenses gate the work",
        ],
    ),
    (
        "The finale",
        &[
            "480-class glass on Main Street hogs",
            "Some contracts list a bounty the thermal cannot see...",
        ],
    ),
    (
        "What thermal cannot see",
        &[
            "Ambient-temperature contacts: no heat signature, silent",
            "NV confirms the walker — no eyeshine, dead retinas don't reflect",
            "Head shots only",
        ],
    ),
    (
        "DarkAir",
        &[
            "Deterministic worlds · honest optics · a real P&L",
            "github.com/jclements3/darkair",
        ],
    ),
];

impl DemoDirector {
    /// Total scripted runtime, seconds.
    pub fn total_dur(&self) -> f32 {
        self.segments.iter().map(|s| s.dur).sum()
    }

    /// The whole narration script with reel timings: one
    /// (start_s, end_s, title, bullets) entry per segment. The film pass
    /// burns these into the 16:9 panel.
    pub fn promo_script(&self) -> Vec<(f32, f32, &'static str, &'static [&'static str])> {
        let mut out = Vec::new();
        let mut t0 = 0.0f32;
        for (seg, (title, bullets)) in self.segments.iter().zip(PROMO) {
            out.push((t0, t0 + seg.dur, *title, *bullets));
            t0 += seg.dur;
        }
        out
    }

    /// Promo narration for the current segment: (title, bullets, segment
    /// index, segment count, whole-reel progress 0..1). `None` once done.
    pub fn promo(&self) -> Option<(&'static str, &'static [&'static str], usize, usize, f32)> {
        self.segments.get(self.idx)?;
        let (title, bullets) = *PROMO.get(self.idx)?;
        let done: f32 = self.segments[..self.idx].iter().map(|s| s.dur).sum();
        let prog = ((done + self.t) / self.total_dur().max(0.001)).clamp(0.0, 1.0);
        Some((title, bullets, self.idx, self.segments.len(), prog))
    }

    pub fn finished(&self) -> bool {
        self.idx >= self.segments.len()
    }

    /// Skip to the next segment (GUI: Space).
    pub fn skip(&mut self) {
        self.idx += 1;
        self.t = 0.0;
    }

    /// Register the current segment's zone meshes with the renderer.
    /// Idempotent and cheap once registered — call every frame before
    /// rendering the segment's draw list.
    pub fn register_meshes(&self, device: &wgpu::Device, renderer: &mut da_render::Renderer) {
        let Some(seg) = self.segments.get(self.idx) else {
            return;
        };
        match &seg.kind {
            SegKind::Fly(h) => renderer.register_meshes(device, h.mesh_registry()),
            SegKind::Hunt(hs) => renderer.register_meshes(device, hs.hunt.mesh_registry()),
            SegKind::Camp { world, .. } => {
                renderer.register_meshes(device, world.mesh_registry())
            }
            SegKind::Card | SegKind::Range(_) => {}
        }
    }

    pub fn new(zones_dir: &str, camp_path: &str) -> Result<Self, String> {
        let mut business = Business::new();
        // Night one loadout, bought at the real counter: multi-pump, basic
        // digital NV, pellets. ($1,200 - 200 - 350 - tins.)
        business.buy_rifle(RifleModel::MultiPump).map_err(|e| format!("{e:?}"))?;
        business.buy_optic(OpticModel::NvBasic).map_err(|e| format!("{e:?}"))?;
        for _ in 0..2 {
            business
                .buy_accessory(Accessory::PelletTin)
                .map_err(|e| format!("{e:?}"))?;
        }

        let catalog = ZoneCatalog::load(zones_dir)?;
        let zone = |name: &str| format!("{zones_dir}/{name}.zone.ron");

        let mut segments = Vec::new();
        let cap = |v: &[(f32, f32, &str)]| -> Vec<(f32, f32, String)> {
            v.iter().map(|(a, b, s)| (*a, *b, s.to_string())).collect()
        };

        // 0 — Title card.
        segments.push(Segment {
            dur: 5.0,
            kind: SegKind::Card,
            captions: cap(&[
                (0.3, 5.0, "D A R K A I R"),
                (1.2, 5.0, "night contracting — an air-rifle pest-control sim"),
                (2.2, 5.0, "everything that follows is the live engine"),
            ]),
        });

        // 1 — Calibration range.
        let mut range = RangeState::new();
        range.rabbit_count = 24;
        range.auto_zoom = true;
        segments.push(Segment {
            dur: 20.0,
            kind: SegKind::Range(Box::new(range)),
            captions: cap(&[
                (0.5, 6.0, "CALIBRATION RANGE — every install boots here"),
                (6.5, 13.0, "sensor-true thermal: 288-class core, device AGC, real refresh"),
                (13.5, 20.0, "--calibrate rates your machine in rabbits (this laptop: 131)"),
            ]),
        });

        // 2 — Contract flyover: the home farm at night.
        let fly = NightHunt::new(&zone("home_farm"), Forecast::Clear, &business, 4242, Mounted::Headlamp)?;
        segments.push(Segment {
            dur: 22.0,
            kind: SegKind::Fly(Box::new(fly)),
            captions: cap(&[
                (0.5, 7.0, "HOME FARM — the client can hear rats under the grain floor"),
                (7.5, 14.0, "CONTRACT: Rat $9 · Rabbit $7 · Raccoon $28 · Hog $60"),
                (14.5, 21.5, "same farm, hunter's thermal — warm bodies have nowhere to hide"),
            ]),
        });

        // 3 — Night 1: rats on digital NV (the eyeshine channel).
        let h1 = HuntScript::new(
            &zone("grain_coop"),
            Forecast::Clear,
            &business,
            77,
            Mounted::NvBasic,
            Some(Species::Rat),
            26.0,
        )?;
        segments.push(Segment {
            dur: 38.0,
            kind: SegKind::Hunt(Box::new(h1)),
            captions: cap(&[
                (0.5, 6.0, "NIGHT 1 — GRAIN CO-OP · multi-pump .22 + digital NV"),
                (6.5, 12.0, "watch for eyeshine: the eye gives the rat away first"),
            ]),
        });

        // 4 — Camp: settle + montage to the thermal purchase.
        let world = CampWorld::new(camp_path, &business, &catalog)?;
        segments.push(Segment {
            dur: 16.0,
            kind: SegKind::Camp {
                world: Box::new(world),
                script: vec![
                    CampBeat {
                        at: 2.0,
                        grant_cents: 0,
                        rifle: None,
                        optic: None,
                        tins: 1,
                        caption: "the night settles: bounties in, pellets out".into(),
                    },
                    CampBeat {
                        at: 7.0,
                        grant_cents: 210_000,
                        rifle: None,
                        optic: Some(OpticModel::ThermalMk2),
                        tins: 1,
                        caption: "SIX NIGHTS LATER — the ledger affords a 288 thermal ($1,950)".into(),
                    },
                ],
                fired: vec![false; 2],
            },
            captions: cap(&[(0.5, 5.0, "CAMP — rack, shelf, contracts. Buy wrong, go bankrupt.")]),
        });

        // 5 — Rabbits on the new thermal (the Stellar look, in anger).
        let h2 = HuntScript::new(
            &zone("home_farm"),
            Forecast::Clear,
            &business, // note: purchases land before this segment RUNS, see advance()
            1312,
            Mounted::Thermal(2),
            Some(Species::Rabbit),
            34.0,
        )?;
        segments.push(Segment {
            dur: 36.0,
            kind: SegKind::Hunt(Box::new(h2)),
            captions: cap(&[
                (0.5, 6.0, "HOME FARM — white-hot 288, the optic from the field footage"),
                (6.5, 12.0, "drop a rabbit and the covey freezes — seconds to work the sits"),
            ]),
        });

        // 6 — Camp montage 2: License D glass and steel.
        let world2 = CampWorld::new(camp_path, &business, &catalog)?;
        segments.push(Segment {
            dur: 14.0,
            kind: SegKind::Camp {
                world: Box::new(world2),
                script: vec![CampBeat {
                    at: 3.0,
                    grant_cents: 430_000,
                    rifle: Some(RifleModel::Premium25),
                    optic: Some(OpticModel::ThermalMk3),
                    tins: 2,
                    caption: "WEEK THREE — License D: .25 PCP ($2,000) + 480 thermal".into(),
                }],
                fired: vec![false; 1],
            },
            captions: cap(&[(0.5, 5.0, "hogs need 30 FPE — the ladder is the game")]),
        });

        // 7 — Finale: hogs on main street, and something the thermal can't see.
        let h3 = HuntScript::new(
            &zone("main_street"),
            Forecast::Clear,
            &business,
            2077,
            Mounted::Thermal(3),
            Some(Species::JuvenileFeralHog),
            40.0,
        )?;
        segments.push(Segment {
            dur: 34.0,
            kind: SegKind::Hunt(Box::new(h3)),
            captions: cap(&[
                (0.5, 6.0, "MAIN STREET — feral hogs in the park, 480-class glass"),
                (18.0, 24.0, "some contracts list a bounty the thermal cannot see..."),
            ]),
        });
        let h4 = HuntScript::new(
            &zone("main_street"),
            Forecast::Clear,
            &business,
            666,
            Mounted::NvBasic,
            Some(Species::Zombie),
            22.0,
        )?;
        segments.push(Segment {
            dur: 26.0,
            kind: SegKind::Hunt(Box::new(h4)),
            captions: cap(&[
                (0.5, 6.0, "ambient-temperature contacts: invisible to thermal, silent"),
                (6.5, 12.0, "NV shows a walker. No eyeshine — dead retinas don't reflect"),
                (12.5, 18.0, "head shots only"),
            ]),
        });

        // 8 — Outro card.
        segments.push(Segment {
            dur: 6.0,
            kind: SegKind::Card,
            captions: cap(&[
                (0.3, 6.0, "D A R K A I R"),
                (1.2, 6.0, "deterministic worlds · honest optics · a real P&L"),
                (2.2, 6.0, "github.com/jclements3/darkair"),
            ]),
        });

        debug_assert_eq!(
            segments.len(),
            PROMO.len(),
            "PROMO narration table must match the segment list"
        );
        Ok(DemoDirector {
            business,
            segments,
            idx: 0,
            t: 0.0,
            frame_seed: 0,
            range_shot_t: 0.0,
        })
    }

    /// Advance the script and produce the frame. Returns None when done.
    pub fn advance(&mut self, dt: f32) -> Option<DemoFrame> {
        // Re-arm hunts that depend on camp purchases: HuntScript::new above
        // captured the business as of construction; rebuild lazily on entry
        // so tier/pellets reflect every purchase made in earlier segments.
        if self.idx >= self.segments.len() {
            return None;
        }
        self.t += dt;
        self.frame_seed = self.frame_seed.wrapping_add(1);
        if self.t >= self.segments[self.idx].dur {
            self.idx += 1;
            self.t = 0.0;
            if self.idx >= self.segments.len() {
                return None;
            }
        }
        let t = self.t;
        let business = &mut self.business;
        let seg = &mut self.segments[self.idx];

        let mut captions: Vec<String> = seg
            .captions
            .iter()
            .filter(|(a, b, _)| t >= *a && t < *b)
            .map(|(_, _, s)| s.clone())
            .collect();

        let (list, cam, settings, mag, hud) = match &mut seg.kind {
            SegKind::Card => {
                let list = DrawList {
                    items: vec![],
                    ambient_f: 50.0,
                    sky_temp_f: 10.0,
                    moonlight: 0.0,
                    heat_decals: vec![],
                    eyeshine: vec![],
                };
                let cam = Camera {
                    eye: Vec3::ZERO,
                    look: Vec3::NEG_Z,
                    up: Vec3::Y,
                    fov_y_deg: 60.0,
                    aspect: 1.0,
                };
                let mut s = OpticSettings::default();
                s.mode = OpticMode::Eye;
                s.eye_exposure = 0.0;
                (list, cam, s, 1.0, None)
            }
            SegKind::Range(r) => {
                r.tick(dt);
                self.range_shot_t += dt;
                if self.range_shot_t > 3.5 {
                    self.range_shot_t = 0.0;
                    r.shots += 1;
                    r.flash_frames = 3;
                }
                let mag = r.sweep_mag();
                let eye = Vec3::new(0.0, 1.6, 8.0);
                let cam = Camera {
                    eye,
                    look: eye + Vec3::new(0.0, -0.04, -1.0) * 40.0,
                    up: Vec3::Y,
                    fov_y_deg: aim::fov_for_mag(mag),
                    aspect: 1.0,
                };
                let mut s = OpticSettings::default();
                s.mode = OpticMode::Thermal;
                s.palette = ThermalPalette::WhiteHot;
                s.scope_mask = true;
                s.sensor_res = Some(288);
                let mins = (r.session_t() / 60.0) as u32;
                let secs = (r.session_t() % 60.0) as u32;
                let hud = Some((r.shots, format!("A1-40m\n{mins:02}:{secs:02}")));
                (r.draw_list(), cam, s, mag, hud)
            }
            SegKind::Fly(h) => {
                h.tick(dt, Vec3::ZERO, false);
                // Crane: high establishing orbit easing down toward the gate.
                let f = (t / seg.dur).clamp(0.0, 1.0);
                let ease = f * f * (3.0 - 2.0 * f);
                let center = Vec3::new(120.0, 0.0, 90.0);
                let ang = 0.9 + f * 0.8;
                let radius = 120.0 - 70.0 * ease;
                let height = 60.0 - 55.0 * ease;
                let eye = center + Vec3::new(ang.cos() * radius, height.max(2.2), ang.sin() * radius);
                let cam = Camera {
                    eye,
                    look: center + Vec3::Y * (2.0 + 8.0 * (1.0 - ease)),
                    up: Vec3::Y,
                    fov_y_deg: 55.0,
                    aspect: 1.0,
                };
                // Eye first (moonlit recon), thermal tease for the last stretch.
                let mut s = OpticSettings::default();
                if f < 0.62 {
                    s.mode = OpticMode::Eye;
                    s.eye_exposure = 1.8;
                } else {
                    s.mode = OpticMode::Thermal;
                    s.palette = ThermalPalette::WhiteHot;
                    s.sensor_res = Some(480);
                }
                (h.draw_list(), cam, s, 1.0, None)
            }
            SegKind::Hunt(hs) => {
                hs.tick(dt, business);
                let (list, cam, settings, mag, feed) = hs.frame(t);
                captions.extend(feed);
                let hud = if matches!(hs.mounted, Mounted::Thermal(_)) && hs.scoped() {
                    Some((
                        hs.hunt.ledger.confirmed_kills().len() as u32,
                        format!("A1-40m\nmag {:.1}x", mag),
                    ))
                } else {
                    None
                };
                (list, cam, settings, mag, hud)
            }
            SegKind::Camp { world, script, fired } => {
                for (i, beat) in script.iter().enumerate() {
                    if !fired[i] && t >= beat.at {
                        fired[i] = true;
                        business.cash_cents += beat.grant_cents;
                        if let Some(r) = beat.rifle {
                            let _ = business.buy_rifle(r);
                        }
                        if let Some(o) = beat.optic {
                            // Outright when the store sells it that way,
                            // else the trade-in upgrade ladder (Mk III).
                            if business.buy_optic(o).is_err() {
                                let _ = business.upgrade_optic(o);
                            }
                        }
                        for _ in 0..beat.tins {
                            let _ = business.buy_accessory(Accessory::PelletTin);
                        }
                    }
                }
                for beat in script.iter() {
                    if t >= beat.at && t < beat.at + 5.0 {
                        captions.push(beat.caption.clone());
                    }
                }
                captions.push(format!(
                    "BANK  ${:.2}",
                    business.cash_cents as f64 / 100.0
                ));
                // Slow dolly past the cabin-wall gun rack (RACK_ORIGIN in
                // camp3d: x 22→26.4 at z 31.5), gaze panning rifle to rifle.
                let f = (t / seg.dur).clamp(0.0, 1.0);
                let eye = Vec3::new(21.0 + f * 5.0, 1.7, 28.2);
                let cam = Camera {
                    eye,
                    look: Vec3::new(22.0 + f * 4.8, 1.25, 31.5),
                    up: Vec3::Y,
                    fov_y_deg: 50.0,
                    aspect: 1.0,
                };
                let mut s = OpticSettings::default();
                s.mode = OpticMode::Eye;
                s.eye_exposure = 1.9;
                (world.draw_list(business), cam, s, 1.0, None)
            }
        };

        let mut settings = settings;
        settings.frame = self.frame_seed;
        Some(DemoFrame {
            list,
            cam,
            settings,
            captions,
            mag,
            thermal_hud: hud,
        })
    }

    /// Rebuild hunts whose segment hasn't started yet against the CURRENT
    /// business (post-purchase tiers/pellets). Called by both consumers
    /// after camp beats fire; cheap no-op when nothing changed.
    pub fn refresh_pending_hunts(&mut self, zones_dir: &str) {
        let zone = |name: &str| format!("{zones_dir}/{name}.zone.ron");
        let plan: Vec<(usize, &str, u64, Mounted, Option<Species>, f32)> = vec![
            (5, "home_farm", 1312, Mounted::Thermal(2), Some(Species::Rabbit), 34.0),
            (7, "main_street", 2077, Mounted::Thermal(3), Some(Species::JuvenileFeralHog), 40.0),
            (8, "main_street", 666, Mounted::NvBasic, Some(Species::Zombie), 22.0),
        ];
        for (i, z, seed, mounted, prefer, engage) in plan {
            if i > self.idx {
                if let Ok(fresh) = HuntScript::new(
                    &zone(z),
                    Forecast::Clear,
                    &self.business,
                    seed,
                    mounted,
                    prefer,
                    engage,
                ) {
                    if let Some(seg) = self.segments.get_mut(i) {
                        if matches!(seg.kind, SegKind::Hunt(_)) {
                            seg.kind = SegKind::Hunt(Box::new(fresh));
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> (String, String) {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        (format!("{root}/assets/zones"), format!("{root}/assets/camp.zone.ron"))
    }

    /// The whole reel plays headless, hits its runtime, lands kills, and
    /// walks the upgrade arc — twice, identically (film-grade determinism).
    #[test]
    fn demo_script_plays_deterministically() {
        let (zones, camp) = assets();
        let run = || {
            let mut d = DemoDirector::new(&zones, &camp).expect("script boots");
            let total = d.total_dur();
            let dt = 1.0 / 30.0;
            let mut frames = 0u32;
            let mut caption_chars = 0usize;
            while let Some(f) = d.advance(dt) {
                d.refresh_pending_hunts(&zones);
                frames += 1;
                caption_chars += f.captions.iter().map(|c| c.len()).sum::<usize>();
                assert!(f.cam.fov_y_deg > 0.5 && f.cam.fov_y_deg <= 90.0);
                assert!(frames < (total * 30.0) as u32 + 60, "script terminates");
            }
            let owns_t4 = d
                .business
                .owns(da_econ::ItemKind::Rifle(RifleModel::Premium25));
            let owns_mk3 = d
                .business
                .owns(da_econ::ItemKind::Optic(OpticModel::ThermalMk3));
            (frames, caption_chars, owns_t4, owns_mk3, d.business.cash_cents)
        };
        let a = run();
        let b = run();
        assert_eq!(a, b, "same build, same reel");
        let (frames, caption_chars, owns_t4, owns_mk3, _) = a;
        assert!(frames > 200 * 30 / 10, "multi-minute reel: {frames} frames");
        assert!(caption_chars > 1_000, "captions actually display");
        assert!(owns_t4 && owns_mk3, "the upgrade arc completes");
    }
}
