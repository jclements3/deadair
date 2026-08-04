//! The calibration range — the default view for now.
//!
//! Purpose-built for measuring speed and responsiveness before the night
//! hunts depend on them: known-distance checkerboard boards and a picket
//! fence are aliasing torture tests for zoom shimmer, hopping rabbit rigs
//! exercise the exact motion future jumping rabbits will perform, and the
//! shell overlays frame-time metrics. Deterministic: same time in, same
//! frame out — so shimmer you see is rendering, not the scene.

use crate::fauna::{self, FaunaPose};
use da_render::draw::{DrawItem, DrawList, Shape as RShape};
use da_sim::Species;
use glam::{Mat4, Vec3};
use std::collections::VecDeque;

/// One hopping rabbit crossing the range on a ping-pong lane.
#[derive(Debug, Clone)]
struct Lane {
    z: f32,
    x: f32,
    dir: f32,
    phase: f32,
}

/// Rolling frame-time statistics.
#[derive(Debug, Default)]
pub struct FrameStats {
    dts: VecDeque<f32>,
}

impl FrameStats {
    pub fn push(&mut self, dt: f32) {
        self.dts.push_back(dt);
        while self.dts.len() > 240 {
            self.dts.pop_front();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.dts.is_empty()
    }

    /// (avg_ms, p95_ms, max_ms) over the window.
    pub fn summary_ms(&self) -> (f32, f32, f32) {
        if self.dts.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        let mut v: Vec<f32> = self.dts.iter().copied().collect();
        let avg = v.iter().sum::<f32>() / v.len() as f32;
        v.sort_by(|a, b| a.total_cmp(b));
        let p95 = v[((v.len() as f32 * 0.95) as usize).min(v.len() - 1)];
        let max = *v.last().unwrap_or(&0.0);
        (avg * 1000.0, p95 * 1000.0, max * 1000.0)
    }

    /// The last N frame times in seconds, oldest first (sparkline data).
    pub fn recent(&self) -> impl Iterator<Item = f32> + '_ {
        self.dts.iter().copied()
    }
}

/// The whole calibration state.
pub struct RangeState {
    t: f32,
    lanes: Vec<Lane>,
    /// Rabbit ground speed, m/s (their bolt is ~9.6 — test above it).
    pub rabbit_speed: f32,
    /// How many rabbits run (stress dial).
    pub rabbit_count: usize,
    /// Oscillate magnification automatically to expose zoom shimmer.
    pub auto_zoom: bool,
    /// Freeze the scene (isolate rendering from motion).
    pub paused: bool,
    /// Reticle sway on/off — off by default: latency reads cleaner.
    pub sway_enabled: bool,
    pub stats: FrameStats,
    /// Frames remaining on the click-flash marker (photon-latency check:
    /// film screen + mouse with a phone; count frames between click and
    /// this white square appearing).
    pub flash_frames: u8,
}

impl Default for RangeState {
    fn default() -> Self {
        Self::new()
    }
}

impl RangeState {
    pub fn new() -> Self {
        let mut lanes = Vec::new();
        for i in 0..MAX_RABBITS {
            let k = i as f32;
            lanes.push(Lane {
                z: -12.0 - (k * 3.7) % 45.0,
                x: -14.0 + (k * 5.3) % 28.0,
                dir: if i % 2 == 0 { 1.0 } else { -1.0 },
                phase: k * 0.37,
            });
        }
        Self {
            t: 0.0,
            lanes,
            rabbit_speed: 4.0,
            rabbit_count: 6,
            auto_zoom: false,
            paused: false,
            sway_enabled: false,
            stats: FrameStats::default(),
            flash_frames: 0,
        }
    }

    /// Advance rabbits along their lanes.
    pub fn tick(&mut self, dt: f32) {
        self.stats.push(dt);
        if self.paused {
            return;
        }
        self.t += dt;
        let speed = self.rabbit_speed;
        for lane in &mut self.lanes {
            let step = speed * dt * lane.dir;
            lane.x += step;
            lane.phase = fauna::advance_phase(Species::Rabbit, lane.phase, step.abs());
            if lane.x > 15.0 {
                lane.dir = -1.0;
            }
            if lane.x < -15.0 {
                lane.dir = 1.0;
            }
        }
    }

    /// Magnification when the auto zoom sweep is on: a slow full-range
    /// sine, 2× to 14.5× — watch the checkerboards while it breathes.
    pub fn sweep_mag(&self) -> f32 {
        2.0 + (14.5 - 2.0) * (0.5 + 0.5 * (self.t * 0.4).sin())
    }

    /// Build the range scene.
    pub fn draw_list(&self) -> DrawList {
        let ambient = 55.0;
        let mut items = vec![DrawItem {
            shape: RShape::GroundPatch { half: 200.0 },
            world: Mat4::from_translation(Vec3::new(0.0, -0.02, -60.0)),
            albedo: [0.24, 0.3, 0.2],
            emissive: 0.0,
            temp_f: ambient - 2.0,
            glass: false,
        }];

        // Distance boards: checkerboard faces at known ranges. The checker
        // cells are deliberately near pixel-frequency at high zoom — if the
        // renderer shimmers, these boards show it first.
        for &range in &[10.0f32, 25.0, 50.0, 75.0, 100.0] {
            let z = -range;
            // Post.
            items.push(DrawItem {
                shape: RShape::Cylinder { radius: 0.05, height: 1.0 },
                world: Mat4::from_translation(Vec3::new(0.0, 0.0, z)),
                albedo: [0.3, 0.25, 0.2],
                emissive: 0.0,
                temp_f: ambient,
                glass: false,
            });
            // Checkerboard: 8×8 alternating cells, 0.8 m board.
            let cell = 0.1;
            for cy in 0..8 {
                for cx in 0..8 {
                    let dark = (cx + cy) % 2 == 0;
                    items.push(DrawItem {
                        shape: RShape::Box {
                            half: Vec3::new(cell * 0.5, cell * 0.5, 0.01),
                        },
                        world: Mat4::from_translation(Vec3::new(
                            (cx as f32 - 3.5) * cell,
                            1.0 + (cy as f32 - 3.5) * cell,
                            z,
                        )),
                        albedo: if dark { [0.05; 3] } else { [0.9; 3] },
                        emissive: 0.0,
                        // Alternating warm/cold cells: the thermal pipeline
                        // gets the same torture test as the light pipelines.
                        temp_f: if dark { ambient + 25.0 } else { ambient - 5.0 },
                        glass: false,
                    });
                }
            }
        }

        // Picket fence at 30 m: thin members near pixel frequency.
        for i in 0..60 {
            items.push(DrawItem {
                shape: RShape::Box {
                    half: Vec3::new(0.04, 0.5, 0.015),
                },
                world: Mat4::from_translation(Vec3::new(-12.0 + i as f32 * 0.4, 0.5, -30.0)),
                albedo: [0.75, 0.72, 0.65],
                emissive: 0.0,
                temp_f: ambient + 3.0,
                glass: false,
            });
        }

        // The rabbits: full articulated rigs, hop gait, warm bodies.
        for lane in self.lanes.iter().take(self.rabbit_count) {
            let pose = FaunaPose {
                pos: Vec3::new(lane.x, 0.0, lane.z),
                heading: if lane.dir > 0.0 { 0.0 } else { std::f32::consts::PI },
                speed_norm: 1.0,
                gait_phase: lane.phase,
                frozen: false,
            };
            for part in fauna::build(Species::Rabbit, &pose) {
                items.push(DrawItem {
                    shape: part.shape,
                    world: part.world,
                    albedo: part.albedo,
                    emissive: 0.0,
                    temp_f: 101.0,
                    glass: false,
                });
            }
        }

        DrawList {
            items,
            ambient_f: ambient,
            sky_temp_f: ambient - 40.0,
            moonlight: 0.6,
            heat_decals: vec![],
            eyeshine: vec![],
        }
    }
}

/// Lane pool size (the stress dial's ceiling).
pub const MAX_RABBITS: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_is_deterministic() {
        let mut a = RangeState::new();
        let mut b = RangeState::new();
        for _ in 0..100 {
            a.tick(1.0 / 60.0);
            b.tick(1.0 / 60.0);
        }
        let da = a.draw_list();
        let db = b.draw_list();
        assert_eq!(da.items.len(), db.items.len());
        for (x, y) in da.items.iter().zip(&db.items) {
            assert_eq!(x.world, y.world, "same time in, same frame out");
        }
    }

    #[test]
    fn rabbits_ping_pong_and_animate() {
        let mut r = RangeState::new();
        r.rabbit_speed = 10.0;
        let x0 = r.lanes[0].x;
        let p0 = r.lanes[0].phase;
        for _ in 0..60 {
            r.tick(1.0 / 60.0);
        }
        assert_ne!(r.lanes[0].x, x0, "rabbit moved");
        assert_ne!(r.lanes[0].phase, p0, "gait advanced");
        // Run long enough to hit both walls: still in bounds.
        for _ in 0..3000 {
            r.tick(1.0 / 60.0);
        }
        for lane in &r.lanes {
            assert!(lane.x >= -15.5 && lane.x <= 15.5, "lane stays on range");
        }
    }

    #[test]
    fn pause_freezes_the_scene_but_not_the_stats() {
        let mut r = RangeState::new();
        r.tick(0.016);
        r.paused = true;
        let before = r.draw_list().items.len();
        let x = r.lanes[0].x;
        for _ in 0..30 {
            r.tick(0.016);
        }
        assert_eq!(r.lanes[0].x, x);
        assert_eq!(r.draw_list().items.len(), before);
        assert!(!r.stats.is_empty());
    }

    #[test]
    fn stress_dial_scales_the_draw_list() {
        let mut r = RangeState::new();
        r.rabbit_count = 2;
        let small = r.draw_list().items.len();
        r.rabbit_count = 50;
        let big = r.draw_list().items.len();
        assert!(big > small + 40 * 8, "each rabbit is a full rig");
    }

    #[test]
    fn frame_stats_summarize() {
        let mut s = FrameStats::default();
        for _ in 0..99 {
            s.push(0.010);
        }
        s.push(0.050); // one spike
        let (avg, p95, max) = s.summary_ms();
        assert!((avg - 10.4).abs() < 1.0, "avg {avg}");
        assert!(max > 49.0);
        // With a single outlier the p95 sits at the common frame time —
        // BELOW the spike-dragged mean. That's the point of reporting it.
        assert!(p95 <= max);
        assert!((p95 - 10.0).abs() < 0.5, "p95 {p95}");
    }

    #[test]
    fn sweep_covers_the_zoom_ladder() {
        let mut r = RangeState::new();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for _ in 0..2000 {
            r.tick(0.016);
            let m = r.sweep_mag();
            lo = lo.min(m);
            hi = hi.max(m);
        }
        assert!(lo < 2.5 && hi > 13.5, "sweep spans 2x..14.5x: {lo}..{hi}");
    }
}
