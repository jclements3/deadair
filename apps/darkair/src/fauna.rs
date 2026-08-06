//! Articulated animal rigs — the "realistic view" pass for the hunter.
//!
//! Each animal is a small jointed rig of renderer CSG primitives
//! ([`da_render::draw::Shape`]) posed per frame by a procedural gait. This
//! module is pure math: pose in, world-space parts out. No randomness, no
//! clocks — idle micro-motion (breathing, tail wag, head loll) is keyed off
//! `gait_phase` only, so the same pose always yields the same parts.
//!
//! Sizing tracks `hunt::body_size` / `da_sim::hit::Target::for_species` so
//! the existing hit colliders still roughly cover the visual. Silhouettes are
//! honest per `assets/reference/optics-look.md`: at distance the rig reads as
//! the same blob the colliders describe; up close the ears/tails/legs carry
//! the species ID (raccoon tail rings, rabbit ears, groundhog sit).
//!
//! Conventions:
//! - Local space: the animal faces **+X**, up is **+Y**. `heading` rotates
//!   the rig with [`Mat4::from_rotation_y`], so a positive heading swings the
//!   nose from +X toward −Z (glam right-handed).
//! - `Shape::Cylinder` is Y-axis with its **base at the local origin**
//!   (y ∈ [0, height]); `Shape::Box` and `Shape::Sphere` are centered.
//! - Part order is deterministic: body first, then head, snout, ears, legs,
//!   tail last.
//!
//! Temperature and emissive are deliberately **not** part of this API — the
//! integrator (hunt.rs `draw_list`) owns per-frame `temp_f` when it wraps
//! these parts into `DrawItem`s.

use da_render::draw::Shape;
use da_sim::Species;
use glam::{Mat4, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

/// One primitive of a posed animal, in world space.
#[derive(Debug, Clone)]
pub struct FaunaPart {
    /// Renderer primitive.
    pub shape: Shape,
    /// World transform (translation column is the part's anchor point).
    pub world: Mat4,
    /// Base surface color for the eye/NV geometry pass.
    pub albedo: [f32; 3],
    /// Head parts carry the headshot/eyeshine anchor. Exactly one per rig.
    pub is_head: bool,
    /// Thermal offset from core body temperature, °F. The black-hot boar
    /// clip is the spec: bare skin (head, ears) reads core-hot, legs nearly
    /// so, while the insulated coat over the trunk reads several degrees
    /// cooler — that difference is what turns a silhouette into an animal
    /// at close range. Integrators apply `body_temp + temp_bias` for
    /// warm-blooded species and ignore it entirely for ambient bodies
    /// (zombies stay exactly ambient — the invariant survives).
    pub temp_bias: f32,
    /// Coat-interior mottle amplitude, °F (`DrawItem::coat_f`). The bias
    /// above sets a part's *mean*; this sets its *variance* — patchy guard
    /// hair over hot skin, strongest on the insulated trunk, zero on a
    /// zombie (uniform surface is the tell).
    pub coat_f: f32,
}

/// Everything needed to pose an animal this frame.
#[derive(Debug, Clone, Copy)]
pub struct FaunaPose {
    /// Ground position (feet), world meters.
    pub pos: Vec3,
    /// World yaw the animal faces, radians (0 ⇒ nose along +X).
    pub heading: f32,
    /// 0 = still, 1 = full gait speed. Scales leg swing and body bob.
    pub speed_norm: f32,
    /// Gait cycles, monotonically increasing (distance / stride — advance
    /// with [`advance_phase`]). Also seeds all idle micro-motion.
    pub gait_phase: f32,
    /// Possum death-feign / groundhog alert-sit. Gait is ignored while set.
    pub frozen: bool,
}

/// Build the posed rig for `species`: a deterministic list of world-space
/// primitives. Never empty; exactly one part has `is_head` set.
pub fn build(species: Species, pose: &FaunaPose) -> Vec<FaunaPart> {
    build_rig(&rig_of(species), pose)
}

/// World-space head anchor (for eyeshine / selection / headshot display).
/// Always matches the translation of the rig's `is_head` part.
pub fn head_pos(species: Species, pose: &FaunaPose) -> Vec3 {
    build(species, pose)
        .iter()
        .find(|p| p.is_head)
        .map(|p| p.world.w_axis.truncate())
        // Unreachable — every rig emits a head — but stay total.
        .unwrap_or(pose.pos + Vec3::Y * 0.5)
}

/// Advance a gait phase by distance moved. Longer-strided species (cow, hog)
/// accumulate phase slower per meter than a rat; the result is monotonic in
/// `dist_m` and pure.
pub fn advance_phase(species: Species, phase: f32, dist_m: f32) -> f32 {
    phase + dist_m.abs() / stride_m(species)
}

/// Hit colliders built FROM the posed visual rig (FR-A3: what you see is
/// what you hit). Every rendered part becomes one or more spheres in the
/// part's world transform, so heading, posture, and gait all move the
/// colliders exactly as they move the pixels — a rabbit sitting up is a
/// taller mark than one grazing, a hog end-on is a narrower mark than one
/// broadside, and the gap between two ears is a miss.
pub fn colliders(id: da_core::EntityId, species: Species, pose: &FaunaPose) -> da_sim::Target {
    use da_sim::Sphere;
    let parts = build(species, pose);
    let mut head = None;
    let mut body = Vec::new();
    for part in &parts {
        let (sx, sy, sz) = (
            part.world.x_axis.truncate().length(),
            part.world.y_axis.truncate().length(),
            part.world.z_axis.truncate().length(),
        );
        let center = part.world.w_axis.truncate();
        let spheres: Vec<Sphere> = match part.shape {
            Shape::Sphere { radius } => {
                // Scaled sphere = ellipsoid. Chain spheres of the smallest
                // cross-section along the longest local axis so a stretched
                // trunk stays trunk-shaped instead of ballooning.
                let (rx, ry, rz) = (radius * sx, radius * sy, radius * sz);
                let r_min = rx.min(ry).min(rz);
                let r_max = rx.max(ry).max(rz);
                if r_max / r_min.max(1e-4) < 1.4 {
                    vec![Sphere { center, r: (rx + ry + rz) / 3.0 }]
                } else {
                    let axis = if rx >= ry && rx >= rz {
                        part.world.x_axis.truncate() / sx.max(1e-6)
                    } else if ry >= rz {
                        part.world.y_axis.truncate() / sy.max(1e-6)
                    } else {
                        part.world.z_axis.truncate() / sz.max(1e-6)
                    };
                    let n = ((r_max / r_min).ceil() as usize).clamp(2, 4);
                    let reach = r_max - r_min;
                    (0..n)
                        .map(|i| {
                            let f = i as f32 / (n - 1) as f32 * 2.0 - 1.0;
                            Sphere { center: center + axis * (f * reach), r: r_min }
                        })
                        .collect()
                }
            }
            Shape::Cylinder { radius, height } => {
                // Base-at-origin along local +Y; chain along the axis.
                let r = radius * (sx + sz) * 0.5;
                let axis = part.world.y_axis.truncate();
                let n = ((height * sy / (r * 2.0).max(1e-4)).ceil() as usize).clamp(1, 4);
                (0..n)
                    .map(|i| {
                        let f = (i as f32 + 0.5) / n as f32;
                        Sphere { center: center + axis * (height * f), r }
                    })
                    .collect()
            }
            Shape::Box { half } => {
                // Ears, tail paddles: one sphere covering the slab's larger
                // face — generous for thin members, but pellet-scale fair.
                let h = Vec3::new(half.x * sx, half.y * sy, half.z * sz);
                vec![Sphere { center, r: h.length() * 0.75 }]
            }
            Shape::Mesh { .. } | Shape::GroundPatch { .. } => vec![],
        };
        if part.is_head {
            // The head's FIRST sphere is the kill zone; any extras (long
            // ellipsoid heads) fold into the body chain.
            let mut it = spheres.into_iter();
            head = it.next();
            body.extend(it);
        } else {
            body.extend(spheres);
        }
    }
    da_sim::Target {
        id,
        species,
        pos: pose.pos,
        head: head.unwrap_or(da_sim::Sphere {
            center: pose.pos + Vec3::Y * 0.5,
            r: 0.05,
        }),
        body,
    }
}

// ---------------------------------------------------------------------------
// Rig descriptors
// ---------------------------------------------------------------------------

/// Ear silhouette on the head — a primary night-optics ID feature.
#[derive(Debug, Clone, Copy, PartialEq)]
enum EarStyle {
    None,
    /// Small round ears (rat, cat, dog, sheep).
    Round,
    /// Long upright ears angled back — the rabbit tell.
    Tall,
    /// Flat drooping flaps (hog).
    Floppy,
}

/// Tail construction.
#[derive(Debug, Clone, Copy, PartialEq)]
enum TailStyle {
    None,
    /// Chain of thin trailing cylinders with a slight sine wag.
    /// `droop` is radians below horizontal per segment (negative curls up).
    Thin { segments: u32, len: f32, droop: f32 },
    /// 5 alternating dark/light sphere segments — the raccoon ring pattern.
    Ringed,
    /// Flat box paddle (beaver).
    Paddle,
    /// Single tiny sphere (hog curl stub, rabbit puff).
    Nub,
}

/// What `frozen` means for this species.
#[derive(Debug, Clone, Copy, PartialEq)]
enum FreezeStyle {
    /// Just stop the gait.
    None,
    /// Possum death-feign: flattened body, splayed legs, limp tail.
    Flatten,
    /// Groundhog alert: upright sit, head high — the classic silhouette.
    SitUp,
}

/// Quadruped rig parameters (also carries the rabbit's dims).
#[derive(Debug, Clone, Copy)]
struct Quad {
    /// Body cylinder radius (m).
    r: f32,
    /// Body length nose-to-rump (m).
    len: f32,
    /// Nominal leg clearance under the belly (m).
    leg: f32,
    /// Head sphere radius (m).
    head_r: f32,
    ears: EarStyle,
    tail: TailStyle,
    freeze: FreezeStyle,
    /// How much the coat's surface reads below core temperature, °F.
    /// Winter guard hair insulates hard: a hog's coat surface can sit
    /// 25+ °F under its skin — which is exactly why the black-hot clip
    /// shows a mid-gray coat around a black head. Wool is even better.
    coat_f: f32,
    /// Coat albedo (muted night palette).
    coat: [f32; 3],
    /// Second coat for patchy animals (cow): rear body half.
    coat2: Option<[f32; 3]>,
    /// Meters travelled per full gait cycle.
    stride: f32,
}

/// Rig family for a species.
enum Rig {
    Quad(Quad),
    /// Bound-hop gait, tall ears, crouched trunk.
    Rabbit(Quad),
    /// The zombie shamble.
    Biped,
}

/// Species → rig. The wildcard arm keeps this compiling as new `Species`
/// variants land concurrently: a variant debug-named "Rabbit" gets the full
/// rabbit rig, anything else falls back to a generic quadruped.
#[allow(unreachable_patterns)]
fn rig_of(species: Species) -> Rig {
    let quad = |r, len, leg, head_r, ears, tail, freeze, coat_f, coat, coat2, stride| {
        Rig::Quad(Quad {
            r, len, leg, head_r, ears, tail, freeze, coat_f, coat, coat2, stride,
        })
    };
    use {EarStyle as E, FreezeStyle as F, TailStyle as T};
    match species {
        Species::Rat => quad(
            0.05, 0.17, 0.04, 0.038,
            E::Round, T::Thin { segments: 3, len: 0.15, droop: 0.22 },
            F::None, 6.0, [0.28, 0.25, 0.22], None, 0.15,
        ),
        Species::Possum => quad(
            0.09, 0.30, 0.07, 0.05,
            E::Round, T::Thin { segments: 2, len: 0.20, droop: 0.08 },
            F::Flatten, 12.0, [0.34, 0.32, 0.30], None, 0.30,
        ),
        Species::Raccoon => quad(
            0.11, 0.34, 0.11, 0.06,
            E::Round, T::Ringed,
            F::None, 16.0, [0.26, 0.24, 0.22], None, 0.35,
        ),
        Species::Groundhog => quad(
            0.115, 0.32, 0.07, 0.062,
            E::Round, T::Nub,
            F::SitUp, 12.0, [0.30, 0.25, 0.18], None, 0.28,
        ),
        Species::Beaver => quad(
            0.13, 0.38, 0.07, 0.07,
            E::Round, T::Paddle,
            F::None, 18.0, [0.28, 0.22, 0.16], None, 0.35,
        ),
        Species::JuvenileFeralHog => quad(
            0.20, 0.62, 0.30, 0.10,
            E::Floppy, T::Nub,
            F::None, 26.0, [0.30, 0.26, 0.22], None, 0.70,
        ),
        Species::Dog => quad(
            0.15, 0.55, 0.34, 0.08,
            E::Round, T::Thin { segments: 1, len: 0.25, droop: -0.5 },
            F::None, 14.0, [0.32, 0.27, 0.20], None, 0.60,
        ),
        Species::Cat => quad(
            0.08, 0.32, 0.15, 0.05,
            E::Round, T::Thin { segments: 2, len: 0.24, droop: -0.3 },
            F::None, 10.0, [0.30, 0.28, 0.26], None, 0.40,
        ),
        Species::Sheep => quad(
            0.28, 0.65, 0.38, 0.09,
            E::Round, T::None,
            F::None, 32.0, [0.55, 0.53, 0.50], None, 0.70,
        ),
        Species::Cow => quad(
            0.45, 1.25, 0.70, 0.16,
            E::None, T::Thin { segments: 1, len: 0.60, droop: 1.2 },
            F::None, 20.0, [0.50, 0.48, 0.45], Some([0.14, 0.12, 0.11]), 1.40,
        ),
        Species::Zombie => Rig::Biped,
        // Species added after this module was written. A variant named
        // "Rabbit" (being added concurrently) gets its real rig; anything
        // else renders as a generic quadruped rather than breaking.
        other => {
            if format!("{other:?}") == "Rabbit" {
                Rig::Rabbit(rabbit_dims())
            } else {
                Rig::Quad(Quad {
                    r: 0.12, len: 0.35, leg: 0.12, head_r: 0.06,
                    ears: E::Round, tail: T::None, freeze: F::None,
                    coat_f: 12.0, coat: [0.30, 0.26, 0.22], coat2: None, stride: 0.50,
                })
            }
        }
    }
}

/// Rabbit dimensions: possum-sized but crouched, with the tall-ear tell.
fn rabbit_dims() -> Quad {
    Quad {
        r: 0.085, len: 0.26, leg: 0.06, head_r: 0.05,
        ears: EarStyle::Tall, tail: TailStyle::Nub, freeze: FreezeStyle::None,
        coat_f: 10.0, coat: [0.32, 0.28, 0.24], coat2: None,
        stride: 0.90, // one bound per cycle
    }
}

fn stride_m(species: Species) -> f32 {
    match rig_of(species) {
        Rig::Quad(q) | Rig::Rabbit(q) => q.stride,
        Rig::Biped => 0.90,
    }
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Zombie pallid coat — bright in NV, nothing in thermal (matches hunt.rs).
const ZOMBIE_COAT: [f32; 3] = [0.62, 0.60, 0.55];
/// Raccoon tail ring albedos, dark / light.
const RING_DARK: [f32; 3] = [0.12, 0.11, 0.10];
const RING_LIGHT: [f32; 3] = [0.45, 0.42, 0.38];

/// Collector that pre-multiplies every local transform by the pose frame.
struct Parts {
    frame: Mat4,
    /// Coat surface depression for this rig, °F (species-dependent).
    insulation_f: f32,
    out: Vec<FaunaPart>,
}

impl Parts {
    fn new(pose: &FaunaPose) -> Self {
        Parts {
            frame: Mat4::from_translation(pose.pos) * Mat4::from_rotation_y(pose.heading),
            insulation_f: 12.0,
            out: Vec::with_capacity(12),
        }
    }
    fn push(&mut self, shape: Shape, local: Mat4, albedo: [f32; 3], is_head: bool) {
        // Part-role heuristic for the thermal bias (clip-calibrated):
        // heads and thin ear boxes are bare skin (core temp); thin leg
        // cylinders run barely cooler; big trunk masses wear the coat.
        let ins = self.insulation_f;
        let (temp_bias, coat_f) = if is_head {
            // Faces still carry a little texture (eye sockets, muzzle) but
            // read near-uniform hot in the clips.
            (0.0, ins * 0.10)
        } else {
            match shape {
                // Bare ears: hot and glassy-smooth in the footage.
                Shape::Box { half } if half.y > half.x * 2.5 => (0.0, 1.0),
                Shape::Cylinder { radius, .. } if radius <= 0.075 => {
                    (-ins * 0.25, ins * 0.15) // legs: thin fur
                }
                // Trunk mottle spans most of the coat's depression: the
                // clip's streaks run from near skin-hot guard-hair gaps to
                // patches nearly at ground tone.
                Shape::Cylinder { radius, .. } if radius >= 0.11 => (-ins, ins * 0.7),
                Shape::Sphere { radius } if radius >= 0.11 => (-ins, ins * 0.7),
                _ => (-ins * 0.5, ins * 0.25), // snout, tail, small joints
            }
        };
        self.out.push(FaunaPart {
            shape,
            world: self.frame * local,
            albedo,
            is_head,
            temp_bias,
            coat_f,
        });
    }
}

/// A cylinder lying along local +X, centered at the origin, with vertical
/// thickness scaled by `squash` (breathing / possum flatten).
fn lying(len: f32, squash: f32) -> Mat4 {
    Mat4::from_scale(Vec3::new(1.0, squash, 1.0))
        * Mat4::from_rotation_z(-FRAC_PI_2)
        * Mat4::from_translation(Vec3::new(0.0, -len * 0.5, 0.0))
}

/// A leg hanging from `hip`, swung `swing` radians (positive = forward).
fn hanging(hip: Vec3, swing: f32) -> Mat4 {
    Mat4::from_translation(hip) * Mat4::from_rotation_z(swing) * Mat4::from_rotation_x(PI)
}

fn build_rig(rig: &Rig, pose: &FaunaPose) -> Vec<FaunaPart> {
    let mut parts = Parts::new(pose);
    match rig {
        Rig::Quad(q) | Rig::Rabbit(q) => parts.insulation_f = q.coat_f,
        Rig::Biped => parts.insulation_f = 0.0, // ambient anyway
    }
    match rig {
        Rig::Quad(q) => build_quad(q, pose, &mut parts),
        Rig::Rabbit(q) => build_rabbit(q, pose, &mut parts),
        Rig::Biped => build_biped(pose, &mut parts),
    }
    parts.out
}

/// Trot layout: (side x, side z, phase offset in cycles). Diagonal pairs in
/// phase: FL+BR at 0, FR+BL at 0.5.
const TROT_LEGS: [(f32, f32, f32); 4] =
    [(1.0, 1.0, 0.0), (1.0, -1.0, 0.5), (-1.0, 1.0, 0.5), (-1.0, -1.0, 0.0)];

fn build_quad(q: &Quad, pose: &FaunaPose, parts: &mut Parts) {
    let flatten = pose.frozen && q.freeze == FreezeStyle::Flatten;
    if pose.frozen && q.freeze == FreezeStyle::SitUp {
        return build_sit(q, pose, parts);
    }
    let speed = if pose.frozen { 0.0 } else { pose.speed_norm.clamp(0.0, 1.0) };
    let w = pose.gait_phase * TAU;

    // Body axis height; flattening squashes everything toward the ground.
    let squash = if flatten { 0.55 } else { 1.0 };
    let y_b = (q.leg + q.r * 0.5) * squash;
    let bob = w.sin().abs() * q.r * 0.12 * speed;
    let head_bob = -w.sin().abs() * q.r * 0.08 * speed;
    // Idle breathing: ±1.5 % vertical scale, keyed off gait_phase only.
    // Suspended while feigning death — a playing possum does not breathe.
    let breath = if flatten { 1.0 } else { 1.0 + 0.015 * w.sin() };
    let leg_r = (q.r * 0.18).clamp(0.012, 0.06);

    // Body (two half-cylinders when the coat is patchy, e.g. cow).
    if let Some(coat2) = q.coat2 {
        let h = q.len * 0.5;
        parts.push(
            Shape::Cylinder { radius: q.r, height: h },
            Mat4::from_translation(Vec3::new(q.len * 0.25, y_b + bob, 0.0))
                * lying(h, breath * squash),
            q.coat,
            false,
        );
        parts.push(
            Shape::Cylinder { radius: q.r, height: h },
            Mat4::from_translation(Vec3::new(-q.len * 0.25, y_b + bob, 0.0))
                * lying(h, breath * squash),
            coat2,
            false,
        );
    } else {
        // Ellipsoid trunk: a lying cylinder reads as a RECTANGLE side-on at
        // close range (the black-hot boar clip shows a rounded back/belly
        // line). A body-length-stretched sphere keeps the same coverage
        // with an organic silhouette from every angle.
        parts.push(
            Shape::Sphere { radius: q.r },
            Mat4::from_translation(Vec3::new(0.0, y_b + bob, 0.0))
                * Mat4::from_scale(Vec3::new(
                    q.len * 0.5 / q.r,
                    breath * squash,
                    1.0,
                )),
            q.coat,
            false,
        );
    }

    // Head + snout. Flattened possum presses its head to the ground.
    let head_c = if flatten {
        Vec3::new(q.len * 0.52, y_b + q.r * 0.2, 0.0)
    } else {
        Vec3::new(q.len * 0.5 + q.head_r * 0.5, y_b + q.r * 0.7 + bob + head_bob, 0.0)
    };
    parts.push(
        Shape::Sphere { radius: q.head_r },
        Mat4::from_translation(head_c),
        q.coat,
        true,
    );
    parts.push(
        Shape::Sphere { radius: q.head_r * 0.45 },
        Mat4::from_translation(head_c + Vec3::new(q.head_r * 0.95, -q.head_r * 0.15, 0.0)),
        q.coat,
        false,
    );

    build_ears(q.ears, head_c, q.head_r, q.coat, parts);

    // Legs: pendulum swing, diagonal pairs in phase (trot). Feigning death
    // splays them stiffly instead.
    for (sx, sz, off) in TROT_LEGS {
        let swing = if flatten {
            sx * 0.95
        } else {
            (w + off * TAU).sin() * 0.55 * speed
        };
        let hip = Vec3::new(sx * q.len * 0.32, y_b, sz * q.r * 0.65);
        parts.push(
            Shape::Cylinder { radius: leg_r, height: y_b },
            hanging(hip, swing),
            q.coat,
            false,
        );
    }

    // Tail — always emitted last (deterministic ordering contract).
    let tail_base = Vec3::new(-q.len * 0.5, y_b + q.r * 0.3 * squash + bob * 0.5, 0.0);
    match q.tail {
        TailStyle::None => {}
        TailStyle::Thin { segments, len, droop } => {
            let limp = if flatten { 0.22 } else { 0.0 };
            let wag_amp = if flatten { 0.0 } else { 0.3 * (0.25 + 0.75 * speed) };
            let seg_len = len / segments.max(1) as f32;
            let tail_r = (q.r * 0.055).max(0.008);
            let mut base = tail_base;
            for k in 0..segments.max(1) {
                let yaw = (w + k as f32 * 0.9).sin() * wag_amp;
                let pitch = FRAC_PI_2 + (droop + limp) * (k + 1) as f32;
                let rot = Mat4::from_rotation_y(yaw) * Mat4::from_rotation_z(pitch);
                parts.push(
                    Shape::Cylinder { radius: tail_r * (1.0 - k as f32 * 0.15), height: seg_len },
                    Mat4::from_translation(base) * rot,
                    q.coat,
                    false,
                );
                base += rot.transform_vector3(Vec3::Y * seg_len);
            }
        }
        TailStyle::Ringed => {
            // Alternating dark/light rings — the raccoon's NV ID feature.
            for k in 0..5u32 {
                let albedo = if k % 2 == 0 { RING_DARK } else { RING_LIGHT };
                let c = tail_base
                    + Vec3::new(-(k as f32 + 0.5) * q.r * 0.5, k as f32 * q.r * 0.09, 0.0);
                parts.push(
                    Shape::Sphere { radius: q.r * 0.28 },
                    Mat4::from_translation(c),
                    albedo,
                    false,
                );
            }
        }
        TailStyle::Paddle => parts.push(
            Shape::Box { half: Vec3::new(q.len * 0.15, q.r * 0.06, q.r * 0.5) },
            Mat4::from_translation(Vec3::new(-q.len * 0.65, q.r * 0.35 * squash, 0.0)),
            q.coat,
            false,
        ),
        TailStyle::Nub => parts.push(
            Shape::Sphere { radius: q.r * 0.22 },
            Mat4::from_translation(tail_base + Vec3::new(-q.r * 0.1, q.r * 0.15, 0.0)),
            q.coat,
            false,
        ),
    }
}

fn build_ears(style: EarStyle, head_c: Vec3, head_r: f32, coat: [f32; 3], parts: &mut Parts) {
    match style {
        EarStyle::None => {}
        EarStyle::Round => {
            for sz in [1.0f32, -1.0] {
                parts.push(
                    Shape::Sphere { radius: head_r * 0.4 },
                    Mat4::from_translation(
                        head_c + Vec3::new(-head_r * 0.1, head_r * 0.75, sz * head_r * 0.65),
                    ),
                    coat,
                    false,
                );
            }
        }
        EarStyle::Tall => {
            // Two thin boxes angled back — the rabbit silhouette.
            let half = Vec3::new(head_r * 0.16, head_r * 1.15, head_r * 0.3);
            for sz in [1.0f32, -1.0] {
                let base = head_c + Vec3::new(-head_r * 0.35, head_r * 0.5, sz * head_r * 0.45);
                parts.push(
                    Shape::Box { half },
                    Mat4::from_translation(base)
                        * Mat4::from_rotation_z(0.5)
                        * Mat4::from_translation(Vec3::new(0.0, half.y, 0.0)),
                    coat,
                    false,
                );
            }
        }
        EarStyle::Floppy => {
            let half = Vec3::new(head_r * 0.5, head_r * 0.55, head_r * 0.14);
            for sz in [1.0f32, -1.0] {
                parts.push(
                    Shape::Box { half },
                    Mat4::from_translation(
                        head_c + Vec3::new(0.0, head_r * 0.5, sz * head_r * 0.75),
                    ) * Mat4::from_rotation_x(sz * 0.9),
                    coat,
                    false,
                );
            }
        }
    }
}

/// Groundhog alert-sit: vertical trunk, head high, forepaws tucked.
fn build_sit(q: &Quad, pose: &FaunaPose, parts: &mut Parts) {
    let h = q.len * 1.15;
    let breath = 1.0 + 0.015 * (pose.gait_phase * TAU).sin();
    parts.push(
        Shape::Cylinder { radius: q.r * 0.85, height: h },
        Mat4::from_scale(Vec3::new(1.0, breath, 1.0)),
        q.coat,
        false,
    );
    let head_c = Vec3::new(q.head_r * 0.3, h + q.head_r * 0.7, 0.0);
    parts.push(Shape::Sphere { radius: q.head_r }, Mat4::from_translation(head_c), q.coat, true);
    parts.push(
        Shape::Sphere { radius: q.head_r * 0.45 },
        Mat4::from_translation(head_c + Vec3::new(q.head_r * 0.95, -q.head_r * 0.15, 0.0)),
        q.coat,
        false,
    );
    build_ears(q.ears, head_c, q.head_r, q.coat, parts);
    // Haunches on the ground, forepaws hanging tucked against the chest.
    let paw_r = ((q.r * 0.18).clamp(0.012, 0.06)) * 0.8;
    for sz in [1.0f32, -1.0] {
        parts.push(
            Shape::Sphere { radius: q.r * 0.55 },
            Mat4::from_translation(Vec3::new(-0.02, q.r * 0.45, sz * q.r * 0.5)),
            q.coat,
            false,
        );
        parts.push(
            Shape::Cylinder { radius: paw_r, height: q.len * 0.28 },
            hanging(Vec3::new(q.r * 0.5, h * 0.60, sz * q.r * 0.38), -0.35),
            q.coat,
            false,
        );
    }
}

/// Rabbit rig, calibrated against the 40 m HIKMICRO crops
/// (assets/reference/rabbit_comparison.png). Three real postures:
///
/// * **graze** (slow): the footage silhouette is a bright RUMP DOME
///   tapering to a small head AT GROUND LEVEL — with the tall-ear spike
///   still up. Motion is an inchworm creep: stretch forward, regather.
/// * **sit-up** (frozen): upright alert scan, ears in a V — the classic
///   pause a hunter shoots on.
/// * **bound** (fast): the |sin| hop arc, hind pair driving together.
fn build_rabbit(q: &Quad, pose: &FaunaPose, parts: &mut Parts) {
    let speed = pose.speed_norm.clamp(0.0, 1.0);
    let w = pose.gait_phase * TAU;
    let breath = 1.0 + 0.015 * w.sin();

    if pose.frozen {
        // Sit-up: rump on the ground, trunk near-vertical, ears tall.
        let rump_r = q.r * 1.15;
        parts.push(
            Shape::Sphere { radius: rump_r },
            Mat4::from_translation(Vec3::new(0.0, rump_r * 0.8, 0.0)),
            q.coat,
            false,
        );
        parts.push(
            Shape::Sphere { radius: q.r * 0.8 },
            Mat4::from_translation(Vec3::new(q.r * 0.25, rump_r * 1.7, 0.0)),
            q.coat,
            false,
        );
        let head_c = Vec3::new(q.r * 0.35, rump_r * 1.7 + q.r * 0.75 + q.head_r * 0.6, 0.0);
        parts.push(
            Shape::Sphere { radius: q.head_r * breath },
            Mat4::from_translation(head_c),
            q.coat,
            true,
        );
        build_ears(EarStyle::Tall, head_c, q.head_r, q.coat, parts);
        return;
    }

    if speed < 0.35 {
        // Graze: rump dome dominant, spine sloping down to a ground-level
        // head. The inchworm: head-to-rump gap breathes with the phase.
        let stretch = 0.5 + 0.5 * w.sin(); // 0 = gathered, 1 = extended
        let rump_r = q.r * 1.2;
        let gap = q.len * (0.55 + 0.45 * stretch);
        parts.push(
            Shape::Sphere { radius: rump_r },
            Mat4::from_translation(Vec3::new(-gap * 0.5, rump_r * 0.85, 0.0)),
            q.coat,
            false,
        );
        // Mid-body taper.
        parts.push(
            Shape::Sphere { radius: q.r * 0.85 },
            Mat4::from_translation(Vec3::new(0.0, q.r * 0.7, 0.0)),
            q.coat,
            false,
        );
        // Head low, at the grass.
        let head_c = Vec3::new(gap * 0.5 + q.head_r * 0.4, q.head_r * 0.9, 0.0);
        parts.push(
            Shape::Sphere { radius: q.head_r * breath },
            Mat4::from_translation(head_c),
            q.coat,
            true,
        );
        // The ear spike stays up even head-down — the tell that survives
        // feeding in the footage.
        let half = Vec3::new(q.head_r * 0.16, q.head_r * 1.05, q.head_r * 0.28);
        for sz in [1.0f32, -1.0] {
            let base = head_c + Vec3::new(-q.head_r * 0.3, q.head_r * 0.4, sz * q.head_r * 0.4);
            parts.push(
                Shape::Box { half },
                Mat4::from_translation(base)
                    * Mat4::from_rotation_z(0.55) // raked up-forward
                    * Mat4::from_translation(Vec3::new(0.0, half.y, 0.0)),
                q.coat,
                false,
            );
        }
        return;
    }

    // Bound: the relocation/flee hop. The scatter clip's airborne read is
    // a LONG LOW STREAK — the trunk stretches through the leap and gathers
    // for the landing, ears raked flat by speed.
    let hop = w.sin().abs() * 0.15 * speed;
    let airborne = w.sin().abs(); // 0 = gathered on the ground, 1 = mid-leap
    let stretch = 1.0 + 0.45 * airborne * speed;
    let pitch = w.sin() * 0.22 * speed * (2.0 - airborne); // flatter mid-air
    let y_b = q.leg + q.r * 0.5;
    let trunk = Mat4::from_translation(Vec3::new(0.0, y_b + hop, 0.0))
        * Mat4::from_rotation_z(pitch);

    parts.push(
        Shape::Cylinder {
            radius: q.r * (0.85 / stretch.sqrt()), // volume roughly conserved
            height: q.len * stretch,
        },
        trunk * lying(q.len * stretch, breath),
        q.coat,
        false,
    );
    let head_c = Vec3::new(q.len * 0.42 + q.head_r * 0.4, q.r * 0.75, 0.0);
    parts.push(
        Shape::Sphere { radius: q.head_r },
        trunk * Mat4::from_translation(head_c),
        q.coat,
        true,
    );
    parts.push(
        Shape::Sphere { radius: q.head_r * 0.45 },
        trunk * Mat4::from_translation(head_c + Vec3::new(q.head_r * 0.9, -q.head_r * 0.1, 0.0)),
        q.coat,
        false,
    );
    let half = Vec3::new(q.head_r * 0.16, q.head_r * 1.15, q.head_r * 0.3);
    // Ears rake back toward the spine as speed rises (flat in full flight).
    let rake = 0.5 - 1.6 * speed * airborne;
    for sz in [1.0f32, -1.0] {
        let base = head_c + Vec3::new(-q.head_r * 0.35, q.head_r * 0.5, sz * q.head_r * 0.45);
        parts.push(
            Shape::Box { half },
            trunk
                * Mat4::from_translation(base)
                * Mat4::from_rotation_z(rake)
                * Mat4::from_translation(Vec3::new(0.0, half.y, 0.0)),
            q.coat,
            false,
        );
    }
    let leg_r = (q.r * 0.18).clamp(0.012, 0.06);
    let front = -w.sin() * 0.6 * speed;
    let hind = w.sin() * 0.95 * speed;
    for sz in [1.0f32, -1.0] {
        parts.push(
            Shape::Cylinder { radius: leg_r, height: y_b },
            trunk * hanging(Vec3::new(q.len * 0.30, 0.0, sz * q.r * 0.6), front),
            q.coat,
            false,
        );
    }
    for sz in [1.0f32, -1.0] {
        parts.push(
            Shape::Cylinder { radius: leg_r * 1.35, height: y_b },
            trunk * hanging(Vec3::new(-q.len * 0.34, 0.0, sz * q.r * 0.6), hind),
            q.coat,
            false,
        );
    }
    parts.push(
        Shape::Sphere { radius: q.r * 0.35 },
        trunk * Mat4::from_translation(Vec3::new(-q.len * 0.52, q.r * 0.4, 0.0)),
        q.coat,
        false,
    );
}

fn build_biped(pose: &FaunaPose, parts: &mut Parts) {
    let speed = if pose.frozen { 0.0 } else { pose.speed_norm.clamp(0.0, 1.0) };
    let w = pose.gait_phase * TAU;
    let coat = ZOMBIE_COAT;

    // Upper body leans forward (and sways) about the pelvis top.
    let sway = 0.03 * (w * 0.5).sin() * speed;
    let lean = Mat4::from_translation(Vec3::new(0.0, 0.95, 0.0))
        * Mat4::from_rotation_z(-0.09 - sway)
        * Mat4::from_rotation_x(0.05)
        * Mat4::from_translation(Vec3::new(0.0, -0.95, 0.0));

    parts.push(
        Shape::Box { half: Vec3::new(0.16, 0.09, 0.11) },
        Mat4::from_translation(Vec3::new(0.0, 0.95, 0.0)),
        coat,
        false,
    );
    parts.push(
        Shape::Box { half: Vec3::new(0.17, 0.23, 0.12) },
        lean * Mat4::from_translation(Vec3::new(0.0, 1.27, 0.0)),
        coat,
        false,
    );
    // Head with a slow loll — keyed off gait_phase only (deterministic).
    let loll = Vec3::new(0.02 * (w * 0.4 + 1.3).sin(), 0.0, 0.04 * (w * 0.5).sin());
    parts.push(
        Shape::Sphere { radius: 0.11 },
        lean * Mat4::from_translation(Vec3::new(0.0, 1.63, 0.0) + loll),
        coat,
        true,
    );

    // Arms: hanging cylinders with a slight asymmetric dangle-swing.
    let dangle = 0.3 + 0.7 * speed;
    for (sz, amp, off) in [(1.0f32, 0.10, 0.3), (-1.0, 0.22, PI + 0.9)] {
        let swing = (w + off).sin() * amp * dangle;
        parts.push(
            Shape::Cylinder { radius: 0.045, height: 0.60 },
            lean * Mat4::from_translation(Vec3::new(0.0, 1.44, sz * 0.23))
                * Mat4::from_rotation_z(swing)
                * Mat4::from_rotation_x(PI + sz * 0.08),
            coat,
            false,
        );
    }

    // Legs: uneven shamble — the right leg swings less and off-beat.
    for (sz, amp, off) in [(1.0f32, 0.40, 0.0), (-1.0, 0.18, PI + 0.7)] {
        let swing = (w + off).sin() * amp * speed;
        parts.push(
            Shape::Cylinder { radius: 0.06, height: 0.88 },
            hanging(Vec3::new(0.0, 0.88, sz * 0.105), swing),
            coat,
            false,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn all_species() -> Vec<Species> {
        vec![
            Species::Rat,
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
        ]
    }

    fn pose(pos: Vec3, heading: f32, speed: f32, phase: f32, frozen: bool) -> FaunaPose {
        FaunaPose { pos, heading, speed_norm: speed, gait_phase: phase, frozen }
    }

    fn translation(p: &FaunaPart) -> Vec3 {
        p.world.w_axis.truncate()
    }

    fn mat_close(a: &Mat4, b: &Mat4, eps: f32) -> bool {
        a.to_cols_array()
            .iter()
            .zip(b.to_cols_array().iter())
            .all(|(x, y)| (x - y).abs() <= eps)
    }

    /// Representative world-space surface points of a part.
    fn sample_points(p: &FaunaPart) -> Vec<Vec3> {
        let local: Vec<Vec3> = match p.shape {
            Shape::Box { half } => (0..8)
                .map(|i| {
                    Vec3::new(
                        if i & 1 == 0 { half.x } else { -half.x },
                        if i & 2 == 0 { half.y } else { -half.y },
                        if i & 4 == 0 { half.z } else { -half.z },
                    )
                })
                .collect(),
            Shape::Cylinder { radius, height } => vec![
                Vec3::ZERO,
                Vec3::new(0.0, height, 0.0),
                Vec3::new(radius, 0.0, 0.0),
                Vec3::new(-radius, 0.0, 0.0),
                Vec3::new(0.0, 0.0, radius),
                Vec3::new(0.0, 0.0, -radius),
                Vec3::new(radius, height, 0.0),
                Vec3::new(-radius, height, 0.0),
            ],
            Shape::Sphere { radius } => vec![
                Vec3::new(radius, 0.0, 0.0),
                Vec3::new(-radius, 0.0, 0.0),
                Vec3::new(0.0, radius, 0.0),
                Vec3::new(0.0, -radius, 0.0),
                Vec3::new(0.0, 0.0, radius),
                Vec3::new(0.0, 0.0, -radius),
            ],
            _ => vec![Vec3::ZERO],
        };
        local.iter().map(|q| p.world.transform_point3(*q)).collect()
    }

    #[test]
    fn every_species_rig_has_one_head_and_head_pos_matches() {
        let p = pose(Vec3::new(50.0, 0.0, 30.0), 0.7, 0.5, 0.37, false);
        for s in all_species() {
            let rig = build(s, &p);
            assert!(!rig.is_empty(), "{s:?} rig is empty");
            let heads: Vec<_> = rig.iter().filter(|q| q.is_head).collect();
            assert_eq!(heads.len(), 1, "{s:?} must have exactly one head part");
            let hp = head_pos(s, &p);
            assert!(
                hp.distance(translation(heads[0])) < 1e-4,
                "{s:?} head_pos {hp:?} != head part translation"
            );
        }
    }

    #[test]
    fn rig_parts_stay_near_pose_and_above_ground() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let p = pose(at, 1.0, 1.0, 0.6, false);
        let mut rigs: Vec<(String, Vec<FaunaPart>)> = all_species()
            .into_iter()
            .map(|s| (format!("{s:?}"), build(s, &p)))
            .collect();
        // The rabbit rig ships even before the Species variant lands.
        rigs.push(("Rabbit".into(), build_rig(&Rig::Rabbit(rabbit_dims()), &p)));
        for (name, rig) in rigs {
            for part in &rig {
                for q in sample_points(&part) {
                    let flat = Vec3::new(q.x - at.x, 0.0, q.z - at.z).length();
                    assert!(flat < 4.0, "{name} part strays {flat} m from pose");
                    assert!(q.y > -0.05, "{name} part dips to y={} (below ground)", q.y);
                    assert!(q.y < 3.5, "{name} part floats to y={}", q.y);
                }
            }
        }
    }

    #[test]
    fn heading_rotates_the_snout() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let east = head_pos(Species::Rat, &pose(at, 0.0, 0.0, 0.13, false)) - at;
        let turned = head_pos(Species::Rat, &pose(at, FRAC_PI_2, 0.0, 0.13, false)) - at;
        // Heading 0 puts the nose along +X.
        assert!(east.x > 0.05 && east.z.abs() < 1e-5, "east offset {east:?}");
        let expect = Mat4::from_rotation_y(FRAC_PI_2).transform_vector3(east);
        assert!(
            (turned - expect).length() < 1e-4,
            "heading must rotate the snout: got {turned:?}, want {expect:?}"
        );
    }

    /// The four dog legs, keyed by hip quadrant (heading 0).
    fn dog_legs(rig: &[FaunaPart], at: Vec3) -> [Mat4; 4] {
        let legs: Vec<&FaunaPart> = rig
            .iter()
            .filter(|p| {
                matches!(p.shape, Shape::Cylinder { radius, .. }
                    if radius > 0.015 && radius < 0.05)
            })
            .collect();
        assert_eq!(legs.len(), 4, "dog has four legs");
        let mut out = [Mat4::IDENTITY; 4]; // FL, FR, BL, BR
        for leg in legs {
            let o = translation(leg) - at;
            let idx = match (o.x > 0.0, o.z > 0.0) {
                (true, true) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (false, false) => 3,
            };
            out[idx] = leg.world;
        }
        out
    }

    #[test]
    fn gait_swings_legs_and_speed_gates_it() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        // At full speed, phase 0.0 vs 0.25 must move the legs.
        let a = dog_legs(&build(Species::Dog, &pose(at, 0.0, 1.0, 0.0, false)), at);
        let b = dog_legs(&build(Species::Dog, &pose(at, 0.0, 1.0, 0.25, false)), at);
        assert!(
            (0..4).any(|i| !mat_close(&a[i], &b[i], 1e-4)),
            "legs must move between phases at speed 1"
        );
        // At speed 0 the legs are identical across phases (breathing only
        // touches the body, never the legs).
        let a = dog_legs(&build(Species::Dog, &pose(at, 0.0, 0.0, 0.0, false)), at);
        let b = dog_legs(&build(Species::Dog, &pose(at, 0.0, 0.0, 0.25, false)), at);
        for i in 0..4 {
            assert!(mat_close(&a[i], &b[i], 1e-6), "leg {i} moved at speed 0");
        }
    }

    #[test]
    fn trot_pairs_diagonal_legs() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let [fl, fr, bl, br] = dog_legs(&build(Species::Dog, &pose(at, 0.0, 1.0, 0.15, false)), at);
        // Diagonal pairs share the same swing rotation.
        assert!((fl.x_axis - br.x_axis).length() < 1e-5, "FL and BR must match");
        assert!((fr.x_axis - bl.x_axis).length() < 1e-5, "FR and BL must match");
        // Opposing pairs swing opposite: the swing shows up as the y
        // component of the leg's x basis vector (Rz(s) ⇒ (cos s, sin s, 0)).
        assert!(fl.x_axis.y.abs() > 1e-3, "legs are actually swung at phase 0.15");
        assert!(
            (fl.x_axis.y + fr.x_axis.y).abs() < 1e-5,
            "FL ({}) must oppose FR ({})",
            fl.x_axis.y,
            fr.x_axis.y
        );
    }

    #[test]
    fn thermal_bias_matches_the_boar_clip_structure() {
        let pose = FaunaPose {
            pos: Vec3::new(10.0, 0.0, 10.0),
            heading: 0.0,
            speed_norm: 0.5,
            gait_phase: 0.1,
            frozen: false,
        };
        let hog = build(Species::JuvenileFeralHog, &pose);
        let head_bias = hog.iter().find(|p| p.is_head).expect("head").temp_bias;
        assert_eq!(head_bias, 0.0, "bare head reads core-hot");
        let coldest = hog.iter().map(|p| p.temp_bias).fold(f32::MAX, f32::min);
        assert!(coldest <= -20.0, "a hog coat insulates hard: {coldest}");
        let kinds: std::collections::BTreeSet<i32> =
            hog.iter().map(|p| p.temp_bias as i32).collect();
        assert!(kinds.len() >= 3, "at least three thermal roles: {kinds:?}");
    }

    #[test]
    fn colliders_follow_heading_and_posture() {
        use da_core::EntityId;
        let at = Vec3::new(20.0, 0.0, -30.0);
        let mk = |heading: f32, frozen: bool, species| FaunaPose {
            pos: at,
            heading,
            speed_norm: 0.1,
            gait_phase: 0.2,
            frozen,
        };

        // Broadside hog (nose +X): a ray down -Z offset along X strikes the
        // long trunk. Swing the hog end-on (nose -Z) and the same ray finds
        // only air where the trunk used to be — orientation-free canonical
        // colliders can never produce this, pose-true ones must.
        let hog = |h| colliders(EntityId(1), Species::JuvenileFeralHog, &mk(h, false, ()));
        let origin = Vec3::new(at.x + 0.38, 0.45, at.z + 5.0);
        let ray = -Vec3::Z;
        let broadside = da_sim::hit::ray_hits(origin, ray, &[hog(0.0)]);
        assert!(!broadside.is_empty(), "broadside trunk is a hit");
        let end_on = da_sim::hit::ray_hits(origin, ray, &[hog(FRAC_PI_2)]);
        assert!(end_on.is_empty(), "end-on the same ray misses the trunk");

        // A frozen (sit-up) rabbit's head rides higher than a grazing one's:
        // the collider must climb with the visual.
        let graze = colliders(EntityId(2), Species::Rabbit, &mk(0.0, false, ()));
        let sit = colliders(EntityId(2), Species::Rabbit, &mk(0.0, true, ()));
        assert!(
            sit.head.center.y > graze.head.center.y + 0.05,
            "sit-up head {} vs graze head {}",
            sit.head.center.y,
            graze.head.center.y
        );

        // The gap between the ears is a MISS: a ray between the two ear
        // slabs above the skull must pass clean through.
        let head = sit.head.center;
        let gap_origin = Vec3::new(head.x, head.y + sit.head.r + 0.06, head.z + 5.0);
        let through_gap = da_sim::hit::ray_hits(gap_origin, -Vec3::Z, &[sit.clone()]);
        // Ears live off-center; straight over the skull centerline is air.
        assert!(
            through_gap.iter().all(|h| h.zone != da_sim::hit::HitZone::Head),
            "above the skull is never a headshot"
        );

        // Zombie: the biped rig yields a man-height head collider.
        let z = colliders(EntityId(3), Species::Zombie, &mk(0.0, false, ()));
        assert!(z.head.center.y > 1.3, "zombie head at head height: {}", z.head.center.y);
        assert!(!z.body.is_empty(), "zombie torso chain exists");
    }

    #[test]
    fn coat_mottle_strongest_on_trunk_absent_on_zombies() {
        let pose = FaunaPose {
            pos: Vec3::new(10.0, 0.0, 10.0),
            heading: 0.0,
            speed_norm: 0.5,
            gait_phase: 0.1,
            frozen: false,
        };
        let hog = build(Species::JuvenileFeralHog, &pose);
        // The trunk carries the strongest interior texture (the black-hot
        // close-up's streaked coat), and it spans a real fraction of the
        // insulation depth — a couple of degrees would not resolve.
        let strongest = hog.iter().map(|p| p.coat_f).fold(0.0f32, f32::max);
        assert!(strongest >= 10.0, "hog trunk mottle resolves: {strongest}");
        let head = hog.iter().find(|p| p.is_head).expect("head");
        assert!(
            head.coat_f < strongest * 0.5,
            "faces read near-uniform hot vs trunk"
        );
        // Zombies: every part must come out with zero mottle — a perfectly
        // uniform surface is the second thermal tell after "no ΔT".
        let z = build(Species::Zombie, &pose);
        assert!(
            z.iter().all(|p| p.coat_f == 0.0),
            "zombie surfaces are uniform"
        );
    }

    #[test]
    fn rabbit_postures_match_the_footage() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let base = FaunaPose {
            pos: at,
            heading: 0.0,
            speed_norm: 0.1,
            gait_phase: 0.2,
            frozen: false,
        };
        // Graze: head at grass level, rump dome above it.
        let graze = build(Species::Rabbit, &base);
        let head = graze.iter().find(|p| p.is_head).expect("head");
        let head_y = head.world.w_axis.y;
        assert!(head_y < 0.12, "feeding head sits at the grass: {head_y}");
        let top = graze
            .iter()
            .map(|p| p.world.w_axis.y)
            .fold(f32::MIN, f32::max);
        assert!(top > head_y, "ear spike/rump rises above the head");
        // Ears exist in graze — the tell survives feeding.
        assert!(graze.iter().filter(|p| matches!(p.shape, da_render::draw::Shape::Box { .. })).count() >= 2);

        // Sit-up: the head goes well above the graze head.
        let sit = build(
            Species::Rabbit,
            &FaunaPose { frozen: true, ..base },
        );
        let sit_head = sit.iter().find(|p| p.is_head).expect("head").world.w_axis.y;
        assert!(sit_head > head_y + 0.12, "alert sit-up is tall: {sit_head} vs {head_y}");

        // Inchworm: silhouette length breathes with the phase while grazing.
        let len_at = |phase: f32| {
            let parts = build(
                Species::Rabbit,
                &FaunaPose { gait_phase: phase, ..base },
            );
            let xs: Vec<f32> = parts.iter().map(|p| p.world.w_axis.x).collect();
            xs.iter().fold(f32::MIN, |a, b| a.max(*b))
                - xs.iter().fold(f32::MAX, |a, b| a.min(*b))
        };
        assert!(
            (len_at(0.25) - len_at(0.75)).abs() > 0.02,
            "stretch vs gather must differ"
        );
    }

    #[test]
    fn bound_stretches_airborne_like_the_scatter_clip() {
        let at = Vec3::new(20.0, 0.0, 20.0);
        let mk = |phase: f32| FaunaPose {
            pos: at,
            heading: 0.0,
            speed_norm: 1.0,
            gait_phase: phase,
            frozen: false,
        };
        // The trunk is the longest cylinder in the rig; its length is the
        // streak. Mid-leap (phase 0.25 → |sin| = 1) vs gathered (~0).
        let trunk_len = |parts: &[FaunaPart]| {
            parts
                .iter()
                .filter_map(|p| match p.shape {
                    da_render::draw::Shape::Cylinder { height, radius } if radius > 0.05 => {
                        Some(height)
                    }
                    _ => None,
                })
                .fold(0.0f32, f32::max)
        };
        let air = trunk_len(&build(Species::Rabbit, &mk(0.25)));
        let ground = trunk_len(&build(Species::Rabbit, &mk(0.005)));
        assert!(
            air > ground * 1.3,
            "airborne trunk is the long streak: {air} vs {ground}"
        );
    }

    #[test]
    fn rabbit_hops_and_wears_tall_ears() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let rig = Rig::Rabbit(rabbit_dims());
        let grounded = build_rig(&rig, &pose(at, 0.0, 1.0, 0.0, false));
        let airborne = build_rig(&rig, &pose(at, 0.0, 1.0, 0.25, false));
        // Body (part 0) rides the |sin| hop arc: top of the bound at 0.25.
        assert!(
            translation(&airborne[0]).y > translation(&grounded[0]).y + 0.05,
            "rabbit body must rise mid-bound"
        );
        // Two long upright ears — the anti-rat silhouette.
        let tall_ears: Vec<_> = grounded
            .iter()
            .filter(|p| matches!(p.shape, Shape::Box { half } if half.y > half.x * 2.0))
            .collect();
        assert_eq!(tall_ears.len(), 2, "rabbit wears two tall ears");
        // Both hind legs drive together (bound, not trot): same rotation.
        let hind: Vec<&FaunaPart> = grounded
            .iter()
            .filter(|p| {
                matches!(p.shape, Shape::Cylinder { radius, .. }
                    if radius > 0.012 && radius < 0.05)
                    && (translation(p) - at).x < 0.0
            })
            .collect();
        assert_eq!(hind.len(), 2, "two hind legs");
        assert!(
            (hind[0].world.x_axis - hind[1].world.x_axis).length() < 1e-5,
            "hind legs move as one in a bound"
        );
    }

    #[test]
    fn zombie_is_a_tall_biped_with_two_arms_two_legs() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let rig = build(Species::Zombie, &pose(at, 0.0, 0.7, 0.4, false));
        let cylinders: Vec<&FaunaPart> = rig
            .iter()
            .filter(|p| matches!(p.shape, Shape::Cylinder { .. }))
            .collect();
        assert_eq!(cylinders.len(), 4, "two arms + two legs");
        let arms = cylinders.iter().filter(|p| translation(p).y > 1.2).count();
        let legs = cylinders.iter().filter(|p| translation(p).y <= 1.2).count();
        assert_eq!(arms, 2, "arms hang from the shoulders");
        assert_eq!(legs, 2, "legs hang from the pelvis");
        // Taller than wide.
        let pts: Vec<Vec3> = rig.iter().flat_map(|p| sample_points(p)).collect();
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for q in pts {
            min = min.min(q);
            max = max.max(q);
        }
        let size = max - min;
        assert!(size.y > size.x && size.y > size.z, "zombie bbox {size:?} must be tall");
        assert!(size.y > 1.5, "zombie stands person-high");
    }

    #[test]
    fn raccoon_tail_alternates_rings() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let rig = build(Species::Raccoon, &pose(at, 0.0, 0.5, 0.2, false));
        // Ordering contract: the tail is emitted last — 5 ring spheres.
        let rings = &rig[rig.len() - 5..];
        for (k, part) in rings.iter().enumerate() {
            assert!(matches!(part.shape, Shape::Sphere { .. }), "ring {k} is a sphere");
            let expect = if k % 2 == 0 { RING_DARK } else { RING_LIGHT };
            assert_eq!(part.albedo, expect, "ring {k} albedo alternates");
        }
    }

    #[test]
    fn possum_freeze_changes_the_rig() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let walking = build(Species::Possum, &pose(at, 0.0, 0.0, 0.1, false));
        let frozen = build(Species::Possum, &pose(at, 0.0, 0.0, 0.1, true));
        assert_eq!(walking.len(), frozen.len(), "same part inventory");
        assert!(
            walking
                .iter()
                .zip(frozen.iter())
                .any(|(a, b)| !mat_close(&a.world, &b.world, 1e-5)),
            "feigning death must repose the rig"
        );
        // Flattened: the frozen body sits lower than the standing one.
        assert!(translation(&frozen[0]).y < translation(&walking[0]).y - 0.01);
    }

    #[test]
    fn groundhog_freeze_sits_taller() {
        let at = Vec3::new(50.0, 0.0, 30.0);
        let top = |rig: &[FaunaPart]| {
            rig.iter()
                .flat_map(|p| sample_points(p))
                .map(|q| q.y)
                .fold(f32::MIN, f32::max)
        };
        let walking = build(Species::Groundhog, &pose(at, 0.0, 0.0, 0.1, false));
        let sitting = build(Species::Groundhog, &pose(at, 0.0, 0.0, 0.1, true));
        assert!(
            top(&sitting) > top(&walking) + 0.05,
            "alert sit ({}) must stand taller than the walk ({})",
            top(&sitting),
            top(&walking)
        );
        assert_eq!(sitting.iter().filter(|p| p.is_head).count(), 1);
    }

    #[test]
    fn advance_phase_scales_with_stride() {
        let gain = |s: Species| advance_phase(s, 0.0, 1.0);
        assert!(gain(Species::Rat) > gain(Species::Cow), "cow strides longer than a rat");
        assert!(
            gain(Species::Rat) > gain(Species::JuvenileFeralHog),
            "hog strides longer than a rat"
        );
        // Monotonic and additive from any starting phase.
        assert!(advance_phase(Species::Rat, 2.0, 0.5) > 2.0);
        assert!(
            (advance_phase(Species::Cat, 1.5, 0.8) - 1.5 - gain(Species::Cat) * 0.8).abs() < 1e-5
        );
    }

    #[test]
    fn build_is_pure() {
        let p = pose(Vec3::new(12.0, 0.0, -7.0), 2.3, 0.8, 5.31, false);
        for s in all_species() {
            let a = build(s, &p);
            let b = build(s, &p);
            assert_eq!(a.len(), b.len());
            for (x, y) in a.iter().zip(b.iter()) {
                assert_eq!(x.world.to_cols_array(), y.world.to_cols_array());
                assert_eq!(x.albedo, y.albedo);
                assert_eq!(x.is_head, y.is_head);
            }
        }
    }
}
