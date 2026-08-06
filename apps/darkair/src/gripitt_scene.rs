//! GRIPITT strike/CSG scene core — shared by the windowed app (`bin/gripitt.rs`)
//! and the headless film exporter (`bin/strike_demo.rs`) via `#[path]` include.
//! Pure `da_render` + `glam`; no window, no ffmpeg, no shared-file coupling.

#![allow(dead_code)]


use da_render::{
    Camera, DrawItem, DrawList, Gpu, OpticMode, OpticSettings, Renderer, Shape, ThermalPalette,
};
use glam::{Mat4, Quat, Vec3};

pub const VIEW: u32 = 1024; // square scene render
pub const PANEL_W: u32 = 576; // right-side bullet panel
pub const FRAME_W: u32 = VIEW + PANEL_W; // 1600
pub const AMBIENT: f32 = 60.0;

// The designated target facility (a hardened C2 building), at the city center.
const TGT: Vec3 = Vec3::new(0.0, 0.0, 0.0);
const TGT_W: f32 = 9.0; // x half-extent*2
const TGT_D: f32 = 9.0; // z
const TGT_H: f32 = 8.0; // height

// ---------------------------------------------------------------------------
// small geometry helpers
// ---------------------------------------------------------------------------

fn item(shape: Shape, world: Mat4, albedo: [f32; 3], emissive: f32, temp_f: f32) -> DrawItem {
    DrawItem { shape, world, albedo, emissive, temp_f, glass: false, coat_f: 0.0 }
}

fn ground() -> DrawItem {
    item(
        Shape::GroundPatch { half: 300.0 },
        Mat4::from_translation(Vec3::new(0.0, -0.02, 0.0)),
        [0.20, 0.21, 0.23],
        0.0,
        AMBIENT - 6.0,
    )
}

/// A box building with its base on the ground (y=0).
fn bldg(x: f32, z: f32, w: f32, d: f32, h: f32, albedo: [f32; 3], temp: f32) -> DrawItem {
    item(
        Shape::Box { half: Vec3::new(w * 0.5, h * 0.5, d * 0.5) },
        Mat4::from_translation(Vec3::new(x, h * 0.5, z)),
        albedo,
        0.0,
        temp,
    )
}

/// A fuel tank: vertical cylinder + squashed-sphere domed cap.
fn tank(x: f32, z: f32, r: f32, h: f32, out: &mut Vec<DrawItem>) {
    out.push(item(
        Shape::Cylinder { radius: r, height: h },
        Mat4::from_translation(Vec3::new(x, h * 0.5, z)),
        [0.42, 0.44, 0.46],
        0.0,
        AMBIENT - 1.0,
    ));
    out.push(item(
        Shape::Sphere { radius: r },
        Mat4::from_scale_rotation_translation(
            Vec3::new(1.0, 0.45, 1.0),
            Quat::IDENTITY,
            Vec3::new(x, h, z),
        ),
        [0.46, 0.48, 0.5],
        0.0,
        AMBIENT - 1.0,
    ));
}

/// Orient an X-long body along `dir`, placed at `pos`.
fn oriented(pos: Vec3, dir: Vec3) -> Mat4 {
    let d = dir.normalize_or_zero();
    let q = if d.length_squared() > 1e-6 {
        Quat::from_rotation_arc(Vec3::X, d)
    } else {
        Quat::IDENTITY
    };
    Mat4::from_rotation_translation(q, pos)
}

/// A missile body (hot, elongated) + a bright tip, appended to `out`.
fn missile(pos: Vec3, dir: Vec3, len: f32, out: &mut Vec<DrawItem>) {
    let w = oriented(pos, dir);
    out.push(item(
        Shape::Box { half: Vec3::new(len * 0.5, 0.32, 0.32) },
        w,
        [0.7, 0.7, 0.72],
        0.7,
        AMBIENT + 320.0,
    ));
    let tip = pos + dir.normalize_or_zero() * (len * 0.5);
    out.push(item(
        Shape::Sphere { radius: 0.5 },
        Mat4::from_translation(tip),
        [1.0, 0.95, 0.9],
        1.0,
        AMBIENT + 900.0,
    ));
}

/// Prefer the system ffmpeg (libx264, real `-crf`) over an Anaconda shim that
/// may shadow it on PATH.
fn ffmpeg_bin() -> &'static str {
    if std::path::Path::new("/usr/bin/ffmpeg").exists() {
        "/usr/bin/ffmpeg"
    } else {
        "ffmpeg"
    }
}

fn smoothstep(f: f32) -> f32 {
    let f = f.clamp(0.0, 1.0);
    f * f * (3.0 - 2.0 * f)
}

fn lerp3(a: Vec3, b: Vec3, s: f32) -> Vec3 {
    a + (b - a) * s
}

// ---------------------------------------------------------------------------
// the CSG city / complex (shared by several segments)
// ---------------------------------------------------------------------------

/// Deterministic pseudo-random in [0,1) from an integer (no rng dep).
fn hashf(mut n: u32) -> f32 {
    n = (n << 13) ^ n;
    let m = (n.wrapping_mul(n.wrapping_mul(n).wrapping_mul(15731).wrapping_add(789221)))
        .wrapping_add(1376312589);
    (m & 0x7fffffff) as f32 / 0x7fffffff as f32
}

/// Surrounding facilities (everything EXCEPT the target building, which the
/// strike segment draws itself so it can be replaced by debris).
fn complex_items(out: &mut Vec<DrawItem>) {
    // a loose grid of hardened buildings + hangars, target left clear at center
    let cool = |t: f32| AMBIENT - 3.0 + t;
    let spots: &[(f32, f32, f32, f32, f32)] = &[
        // x,   z,    w,    d,    h
        (-26.0, -14.0, 10.0, 14.0, 6.0),
        (-24.0, 12.0, 8.0, 8.0, 9.0),
        (22.0, -18.0, 12.0, 9.0, 5.0),
        (26.0, 10.0, 9.0, 12.0, 7.0),
        (-40.0, 2.0, 7.0, 7.0, 11.0), // tower
        (40.0, -2.0, 8.0, 8.0, 8.0),
        (2.0, -34.0, 16.0, 8.0, 6.5), // hangar-ish
        (-6.0, 30.0, 14.0, 9.0, 5.5),
    ];
    for (i, &(x, z, w, d, h)) in spots.iter().enumerate() {
        let g = 0.30 + 0.18 * hashf(i as u32 + 3);
        out.push(bldg(x, z, w, d, h, [g, g, g * 1.03], cool(hashf(i as u32) * 3.0)));
        // a warm rooftop unit (HVAC) — a small emissive/warm box
        out.push(bldg(
            x + 1.0,
            z + 1.0,
            2.2,
            2.2,
            h + 1.2,
            [0.35, 0.35, 0.37],
            AMBIENT + 14.0,
        ));
    }
    // fuel farm
    tank(-52.0, -22.0, 3.2, 6.0, out);
    tank(-45.0, -24.0, 3.2, 6.0, out);
    // radar: tapered tower (cylinder) + dish (squashed sphere)
    out.push(item(
        Shape::Cylinder { radius: 0.7, height: 12.0 },
        Mat4::from_translation(Vec3::new(48.0, 6.0, 22.0)),
        [0.4, 0.4, 0.42],
        0.0,
        AMBIENT,
    ));
    out.push(item(
        Shape::Sphere { radius: 3.0 },
        Mat4::from_scale_rotation_translation(
            Vec3::new(1.0, 1.0, 0.35),
            Quat::from_rotation_y(0.6),
            Vec3::new(48.0, 12.5, 22.0),
        ),
        [0.5, 0.5, 0.52],
        0.0,
        AMBIENT + 2.0,
    ));
}

/// The intact target facility (C2 bunker): a box + a low roof cap + a warm
/// hottest-spot (the designated aimpoint window).
fn target_intact(out: &mut Vec<DrawItem>) {
    out.push(bldg(TGT.x, TGT.z, TGT_W, TGT_D, TGT_H, [0.40, 0.40, 0.43], AMBIENT + 2.0));
    // roof parapet cap
    out.push(bldg(TGT.x, TGT.z, TGT_W + 0.6, TGT_D + 0.6, 0.6, [0.32, 0.32, 0.35], AMBIENT + 1.0));
    // aimpoint window: a warm inset on the +Z face
    out.push(item(
        Shape::Box { half: Vec3::new(1.1, 1.1, 0.15) },
        Mat4::from_translation(Vec3::new(TGT.x, TGT_H * 0.62, TGT.z + TGT_D * 0.5 + 0.05)),
        [0.9, 0.85, 0.7],
        0.35,
        AMBIENT + 40.0,
    ));
}

// ---------------------------------------------------------------------------
// segments
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub enum Kind {
    Brand,
    City,
    Operators,
    Seeker,
    Scintillation,
    Strike,
    Aftermath,
    Close,
}

pub struct Seg {
    pub dur: f32,
    pub kind: Kind,
    pub title: &'static str,
    pub bullets: &'static [&'static str],
    pub caps: &'static [(f32, f32, &'static str)],
}

/// Total reel length (seconds).
pub fn total_dur(segs: &[Seg]) -> f32 {
    segs.iter().map(|s| s.dur).sum()
}

/// Map a global time (already wrapped into `[0, total_dur)`) to
/// `(segment index, local time within that segment)`.
pub fn locate(segs: &[Seg], t: f32) -> (usize, f32) {
    let mut acc = 0.0;
    for (i, s) in segs.iter().enumerate() {
        if t < acc + s.dur {
            return (i, t - acc);
        }
        acc += s.dur;
    }
    let last = segs.len().saturating_sub(1);
    (last, segs.get(last).map(|s| s.dur).unwrap_or(0.0))
}

pub fn script() -> Vec<Seg> {
    vec![
        Seg {
            dur: 4.0,
            kind: Kind::Brand,
            title: "GEOMETRIC RiPiTT",
            bullets: &[
                "Constructive Solid Geometry",
                "the modeling engine",
                "watertight - analytic - parametric",
                "the same solids drive the render",
            ],
            caps: &[(0.4, 3.6, "GEOMETRIC RiPiTT  -  CSG IR scene generation")],
        },
        Seg {
            dur: 12.0,
            kind: Kind::City,
            title: "The scene is a CSG tree",
            bullets: &[
                "solid = boolean over primitives",
                "buildings = boxes",
                "tanks = cylinder + domed cap",
                "radar = revolved paraboloid",
                "exact primitives, no mesh soup",
            ],
            caps: &[(0.5, 11.0, "every structure here is exact CSG")],
        },
        Seg {
            dur: 10.0,
            kind: Kind::Operators,
            title: "2D profiles -> 3D by operator",
            bullets: &[
                "sketch: rect, roundrect, bezier",
                "extrude (+twist), revolve/lathe",
                "loft (DTW), sweep (RMF)",
                "one script regenerates it all",
            ],
            caps: &[(0.5, 9.0, "parametric - regenerate the whole complex")],
        },
        Seg {
            dur: 10.0,
            kind: Kind::Seeker,
            title: "Solid -> rays: the seeker view",
            bullets: &[
                "analytic ray-solid intersection",
                "exact silhouette + projected area",
                "cruise + ballistic, both CSG",
                "IR signature computed, not painted",
            ],
            caps: &[(0.5, 9.0, "sensor-true IR: exact projected area per pixel")],
        },
        Seg {
            dur: 12.0,
            kind: Kind::Scintillation,
            title: "Why analytic beats facets",
            bullets: &[
                "target subtends << 1 pixel",
                "naive splat hops cells -> shimmer",
                "analytic coverage: energy conserved",
                "shimmer corrupts seeker trackers",
            ],
            caps: &[
                (0.5, 5.5, "checkerboards near pixel frequency, zoom breathing"),
                (5.8, 11.0, "shimmer is measured, not hoped away"),
            ],
        },
        Seg {
            dur: 12.0,
            kind: Kind::Strike,
            title: "Impact is a boolean",
            bullets: &[
                "BSP kernel: classify, split, clip",
                "impact = subtract a crater volume",
                "debris = solid CSG fragments",
                "verified vs analytic volumes",
            ],
            caps: &[
                (0.4, 5.0, "terminal homing on the designated facility"),
                (5.2, 11.0, "the crater is a real boolean carve"),
            ],
        },
        Seg {
            dur: 8.0,
            kind: Kind::Aftermath,
            title: "Every con is the price of a pro",
            bullets: &[
                "+ parametric, watertight, exact",
                "+ deterministic, tiny data",
                "- detail ceiling (organic is hard)",
                "- boolean robustness at degeneracies",
            ],
            caps: &[(0.5, 7.0, "analytic/parametric is what costs free-form detail")],
        },
        Seg {
            dur: 5.0,
            kind: Kind::Close,
            title: "A watertight analytic kernel",
            bullets: &[
                "verifiable to closed form",
                "small enough to script",
                "exact enough to ray-trace",
                "GEOMETRIC RiPiTT",
            ],
            caps: &[(0.4, 4.6, "VALKYRIE ENTERPRISES")],
        },
    ]
}

/// Build the scene for a segment at local time `tl` (seconds) with `frame`
/// counter (grain animation). Returns (list, camera, optic settings).
pub fn build(kind: Kind, tl: f32, dur: f32, frame: u32) -> (DrawList, Camera, OpticSettings) {
    let f = (tl / dur).clamp(0.0, 1.0);
    let aspect = 1.0;
    let mut items: Vec<DrawItem> = Vec::new();
    let mut heat: Vec<da_render::HeatDecal> = Vec::new();

    let (cam, optic_mode) = match kind {
        Kind::Brand | Kind::Close => {
            // black card: no geometry
            let cam = Camera {
                eye: Vec3::new(0.0, 5.0, 30.0),
                look: Vec3::ZERO,
                up: Vec3::Y,
                fov_y_deg: 40.0,
                aspect,
            };
            (cam, OpticMode::Thermal)
        }
        Kind::City | Kind::Operators => {
            items.push(ground());
            complex_items(&mut items);
            target_intact(&mut items);
            // slow crane orbit around the complex
            let a = 0.6 + smoothstep(f) * 1.5;
            let r = if matches!(kind, Kind::Operators) { 55.0 } else { 82.0 };
            let h = if matches!(kind, Kind::Operators) { 26.0 } else { 40.0 };
            let cam = Camera {
                eye: Vec3::new(r * a.cos(), h, r * a.sin()),
                look: Vec3::new(0.0, 6.0, 0.0),
                up: Vec3::Y,
                fov_y_deg: 34.0,
                aspect,
            };
            (cam, OpticMode::Eye)
        }
        Kind::Seeker => {
            items.push(ground());
            complex_items(&mut items);
            target_intact(&mut items);
            // two threats inbound toward the target
            let sc = smoothstep(f);
            // cruise: low, terrain-following, from -x
            let cstart = Vec3::new(-90.0, 4.0, 14.0);
            let cend = Vec3::new(TGT.x - 8.0, 4.0, TGT.z + 6.0);
            let cpos = lerp3(cstart, cend, 0.15 + 0.8 * sc);
            missile(cpos, cend - cstart, 4.0, &mut items);
            // ballistic: high arc from +x/+y
            let bstart = Vec3::new(70.0, 60.0, 46.0);
            let bend = Vec3::new(TGT.x, TGT_H + 1.0, TGT.z);
            let bs = 0.1 + 0.85 * sc;
            let bpos = lerp3(bstart, bend, bs) + Vec3::Y * 34.0 * bs * (1.0 - bs);
            let bnext = lerp3(bstart, bend, bs + 0.02) + Vec3::Y * 34.0 * (bs + 0.02) * (0.98 - bs);
            missile(bpos, bnext - bpos, 5.0, &mut items);
            // seeker vantage: descending toward the target from the ballistic bearing
            let eye = lerp3(Vec3::new(60.0, 44.0, 40.0), Vec3::new(24.0, 16.0, 18.0), sc);
            let cam = Camera { eye, look: TGT + Vec3::Y * 4.0, up: Vec3::Y, fov_y_deg: 28.0, aspect };
            (cam, OpticMode::Thermal)
        }
        Kind::Scintillation => {
            // checkerboard boards at known ranges (aliasing torture test) +
            // moving hot blobs crossing sub-pixel; camera FOV "breathes" to
            // expose zoom shimmer. All CSG boxes (after range.rs).
            items.push(ground());
            for &range in &[12.0f32, 28.0, 48.0, 72.0, 100.0] {
                let z = -range;
                // post
                items.push(item(
                    Shape::Cylinder { radius: 0.06, height: 1.0 },
                    Mat4::from_translation(Vec3::new(0.0, 0.5, z)),
                    [0.3, 0.28, 0.25],
                    0.0,
                    AMBIENT,
                ));
                let cell = 0.11;
                for cy in 0..8 {
                    for cx in 0..8 {
                        let dark = (cx + cy) % 2 == 0;
                        items.push(item(
                            Shape::Box { half: Vec3::new(cell * 0.5, cell * 0.5, 0.012) },
                            Mat4::from_translation(Vec3::new(
                                (cx as f32 - 3.5) * cell,
                                2.0 + (cy as f32 - 3.5) * cell,
                                z,
                            )),
                            if dark { [0.05; 3] } else { [0.92; 3] },
                            0.0,
                            if dark { AMBIENT + 28.0 } else { AMBIENT - 6.0 },
                        ));
                    }
                }
            }
            // moving hot blobs (sub-pixel energy-hop demo)
            for i in 0..5 {
                let k = i as f32;
                let span = 14.0;
                let x = span * ((tl * 0.8 + k * 1.3).sin());
                items.push(item(
                    Shape::Sphere { radius: 0.16 },
                    Mat4::from_translation(Vec3::new(x, 1.3 + 0.2 * k, -18.0 - 9.0 * k)),
                    [1.0, 1.0, 1.0],
                    0.6,
                    AMBIENT + 120.0,
                ));
            }
            // zoom sweep: sine over the fov ladder (wide<->narrow) exposes crawl
            let mag = 2.0 + 12.5 * (0.5 + 0.5 * (tl * 0.6).sin());
            let fov = (60.0 / mag).clamp(3.5, 55.0);
            let cam = Camera {
                eye: Vec3::new(0.0, 2.0, 6.0),
                look: Vec3::new(0.0, 2.0, -40.0),
                up: Vec3::Y,
                fov_y_deg: fov,
                aspect,
            };
            (cam, OpticMode::Thermal)
        }
        Kind::Strike | Kind::Aftermath => {
            items.push(ground());
            complex_items(&mut items);
            let impact = dur * 0.45;
            if tl < impact || matches!(kind, Kind::Aftermath) {
                if matches!(kind, Kind::Strike) {
                    target_intact(&mut items);
                    // incoming ballistic missile onto the aimpoint window
                    let s = (tl / impact).clamp(0.0, 1.0);
                    let bstart = Vec3::new(58.0, 52.0, 40.0);
                    let bend = TGT + Vec3::new(0.0, TGT_H * 0.62, TGT_D * 0.5);
                    let bs = smoothstep(s);
                    let bp = lerp3(bstart, bend, bs) + Vec3::Y * 20.0 * bs * (1.0 - bs);
                    let bn = lerp3(bstart, bend, bs + 0.02)
                        + Vec3::Y * 20.0 * (bs + 0.02) * (0.98 - bs);
                    missile(bp, bn - bp, 5.0, &mut items);
                } else {
                    // aftermath: the settled cratered building
                    cratered_target(9.0, &mut items, &mut heat);
                }
            } else {
                // post-impact: crater + debris + brief flash
                let td = tl - impact;
                cratered_target(td, &mut items, &mut heat);
                if td < 0.45 {
                    let r = 1.5 + td * 14.0;
                    items.push(item(
                        Shape::Sphere { radius: r },
                        Mat4::from_translation(TGT + Vec3::new(0.0, TGT_H * 0.6, TGT_D * 0.5)),
                        [1.0, 0.9, 0.7],
                        1.0,
                        AMBIENT + 1400.0 * (1.0 - td / 0.45),
                    ));
                }
            }
            // slow push-in, three-quarter view of the target face
            let sc = smoothstep(f);
            let eye = lerp3(Vec3::new(34.0, 14.0, 34.0), Vec3::new(20.0, 9.0, 22.0), sc);
            let cam = Camera { eye, look: TGT + Vec3::Y * 4.0, up: Vec3::Y, fov_y_deg: 30.0, aspect };
            (cam, OpticMode::Thermal)
        }
    };

    let settings = OpticSettings {
        mode: optic_mode,
        palette: ThermalPalette::WhiteHot,
        scope_mask: false,
        frame,
        eye_exposure: 1.6,
        ..Default::default()
    };
    let list = DrawList {
        items,
        ambient_f: AMBIENT,
        sky_temp_f: AMBIENT - 42.0,
        moonlight: 0.85,
        heat_decals: heat,
        eyeshine: vec![],
    };
    (list, cam, settings)
}

/// The target building after the strike: a boolean-style notch carved out of
/// the wall (drawn as the surviving solid pieces) + solid CSG debris on
/// ballistic arcs + a ground scorch. `td` = seconds since impact.
fn cratered_target(td: f32, out: &mut Vec<DrawItem>, heat: &mut Vec<da_render::HeatDecal>) {
    let warm = AMBIENT + (60.0 * (1.0 - (td / 9.0).min(1.0))).max(6.0);
    // surviving lower block (intact base)
    out.push(bldg(TGT.x, TGT.z, TGT_W, TGT_D, TGT_H * 0.45, [0.38, 0.36, 0.36], warm));
    // two leaning wall remnants flanking the crater
    for s in [-1.0f32, 1.0] {
        out.push(item(
            Shape::Box { half: Vec3::new(TGT_W * 0.22, TGT_H * 0.30, TGT_D * 0.5) },
            Mat4::from_rotation_translation(
                Quat::from_rotation_z(s * 0.18),
                Vec3::new(TGT.x + s * TGT_W * 0.30, TGT_H * 0.55, TGT.z),
            ),
            [0.34, 0.33, 0.33],
            0.0,
            warm,
        ));
    }
    // the crater void reads as a dark recessed box on the struck face
    out.push(item(
        Shape::Box { half: Vec3::new(TGT_W * 0.24, TGT_H * 0.22, 0.6) },
        Mat4::from_translation(Vec3::new(TGT.x, TGT_H * 0.55, TGT.z + TGT_D * 0.5 - 0.4)),
        [0.02, 0.02, 0.02],
        0.0,
        AMBIENT - 4.0,
    ));
    // solid CSG debris on ballistic arcs from the impact point
    let src = TGT + Vec3::new(0.0, TGT_H * 0.6, TGT_D * 0.5);
    for i in 0..14 {
        let a = hashf(i) * std::f32::consts::TAU;
        let sp = 6.0 + 8.0 * hashf(i + 51);
        let v = Vec3::new(a.cos() * sp * 0.7, 5.0 + 7.0 * hashf(i + 91), a.sin() * sp * 0.7 + 4.0);
        let mut p = src + v * td + Vec3::new(0.0, -0.5 * 9.8 * td * td, 0.0);
        if p.y < 0.25 {
            p.y = 0.25; // settled on the ground
        }
        let sz = 0.35 + 0.5 * hashf(i + 7);
        out.push(item(
            Shape::Box { half: Vec3::splat(sz * 0.5) },
            Mat4::from_rotation_translation(
                Quat::from_rotation_y(a) * Quat::from_rotation_x(td * (0.5 + hashf(i + 3))),
                p,
            ),
            [0.30, 0.29, 0.28],
            0.0,
            warm - 4.0,
        ));
    }
    heat.push(da_render::HeatDecal {
        pos: TGT + Vec3::new(0.0, 0.0, TGT_D * 0.5),
        radius_m: 5.0,
        delta_f: (50.0 * (1.0 - (td / 9.0).min(1.0))).max(4.0),
    });
}
