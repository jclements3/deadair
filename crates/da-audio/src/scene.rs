//! The pure, testable spatial core: listener + sources in, panned and
//! attenuated cues out. No backend, no I/O, no allocation beyond the result
//! vector.
//!
//! DeadAir is a headphone game. Everything here is a horizontal-plane
//! (XZ) model — elevation is deliberately ignored because the gameplay
//! question is always "which way do I turn and how far do I walk".

use glam::{Vec2, Vec3};

use crate::kinds::{SoundKind, AUDIBLE_EPSILON, REFERENCE_DISTANCE_M};
use crate::subtitle::{Direction8, DistanceBand, Subtitle};

/// Project a world position onto the horizontal plane.
fn flat(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// Where the player's ears are and which way they point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// World position of the head.
    pub pos: Vec3,
    /// Facing direction. Only the XZ components matter; need not be
    /// normalised. A zero/degenerate facing is treated as `-Z`.
    pub facing: Vec3,
}

impl Listener {
    /// A listener at `pos` looking along `facing`.
    pub fn new(pos: Vec3, facing: Vec3) -> Self {
        Self { pos, facing }
    }

    /// Normalised forward vector in the horizontal plane.
    pub fn forward2(&self) -> Vec2 {
        let f = flat(self.facing);
        if f.length_squared() < 1.0e-12 {
            Vec2::new(0.0, -1.0)
        } else {
            f.normalize()
        }
    }

    /// Normalised right-hand vector in the horizontal plane
    /// (`forward × up` for a Y-up world).
    pub fn right2(&self) -> Vec2 {
        let f = self.forward2();
        Vec2::new(-f.y, f.x)
    }
}

impl Default for Listener {
    fn default() -> Self {
        Self {
            pos: Vec3::ZERO,
            facing: Vec3::new(0.0, 0.0, -1.0),
        }
    }
}

/// One thing making noise in the world this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundSource {
    /// What it is.
    pub kind: SoundKind,
    /// Where it is.
    pub pos: Vec3,
    /// Per-instance gain multiplier on top of the kind's nominal loudness.
    /// 1.0 is "a typical one of these".
    pub gain: f32,
    /// Occlusion hook, 0..1. `0.0` = line of sight, `1.0` = fully blocked.
    /// This crate does **not** raytrace; whoever owns the world geometry
    /// (or a cheap wall query) sets this. It scales attenuation linearly and
    /// is also intended to drive a low-pass in the backend.
    pub muffle: f32,
}

impl SoundSource {
    /// A source with unit gain and no occlusion.
    pub fn new(kind: SoundKind, pos: Vec3) -> Self {
        Self {
            kind,
            pos,
            gain: 1.0,
            muffle: 0.0,
        }
    }

    /// Builder: set the per-instance gain.
    pub fn with_gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }

    /// Builder: set the occlusion factor.
    pub fn with_muffle(mut self, muffle: f32) -> Self {
        self.muffle = muffle.clamp(0.0, 1.0);
        self
    }
}

/// A source that survived culling, resolved against the listener.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudibleSource {
    /// The originating source.
    pub source: SoundSource,
    /// Stereo pan, `-1.0` hard left .. `0.0` centre .. `+1.0` hard right.
    /// Front/back ambiguous by construction — the subtitle disambiguates.
    pub pan: f32,
    /// Distance attenuation in 0..1, including `muffle`.
    pub attenuation: f32,
    /// Final playback volume: `attenuation * gain * kind loudness`.
    pub volume: f32,
    /// Horizontal distance to the listener, meters.
    pub distance_m: f32,
    /// Bearing relative to listener facing, degrees clockwise
    /// (`0` ahead, `+90` right, `180` behind).
    pub bearing_deg: f32,
}

impl AudibleSource {
    /// The HUD caption for this source (SRS NFR-3).
    pub fn subtitle(&self) -> Subtitle {
        Subtitle {
            text: self.source.kind.caption(),
            direction: Direction8::from_bearing_deg(self.bearing_deg),
            distance_band: DistanceBand::classify(
                self.distance_m,
                self.source.kind.falloff_m(),
            ),
        }
    }
}

/// Distance attenuation for one kind: inverse-distance past the reference
/// distance, windowed so it reaches **exactly zero** at the falloff radius
/// and stays there. Monotonically non-increasing in `distance_m`.
pub fn attenuation(kind: SoundKind, distance_m: f32) -> f32 {
    let falloff = kind.falloff_m();
    let d = distance_m.max(0.0);
    if falloff <= 0.0 || d >= falloff {
        return 0.0;
    }
    let inverse = REFERENCE_DISTANCE_M / d.max(REFERENCE_DISTANCE_M);
    let window = 1.0 - d / falloff;
    (inverse * window).clamp(0.0, 1.0)
}

/// The spatialiser. Stateless apart from a master volume, so it is trivially
/// testable and can be reused per frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioScene {
    /// Master gain applied to every source, 0..1.
    pub master_gain: f32,
    /// Volume below which a source is dropped instead of mixed.
    pub cull_threshold: f32,
}

impl Default for AudioScene {
    fn default() -> Self {
        Self {
            master_gain: 1.0,
            cull_threshold: AUDIBLE_EPSILON,
        }
    }
}

impl AudioScene {
    /// A scene with the default cull threshold and the given master gain.
    pub fn new(master_gain: f32) -> Self {
        Self {
            master_gain,
            ..Self::default()
        }
    }

    /// Resolve one source against a listener, ignoring culling.
    ///
    /// Ambient beds ([`SoundKind::is_ambient_bed`]) bypass the spatial model:
    /// they play centred at full attenuation.
    pub fn resolve(&self, listener: &Listener, source: &SoundSource) -> AudibleSource {
        let muffle = source.muffle.clamp(0.0, 1.0);
        let loudness = source.kind.loudness();

        if source.kind.is_ambient_bed() {
            let attenuation = 1.0 - muffle;
            return AudibleSource {
                source: *source,
                pan: 0.0,
                attenuation,
                volume: (attenuation * source.gain * loudness * self.master_gain).max(0.0),
                distance_m: 0.0,
                bearing_deg: 0.0,
            };
        }

        let offset = flat(source.pos) - flat(listener.pos);
        let distance_m = offset.length();
        let forward = listener.forward2();
        let right = listener.right2();

        let (pan, bearing_deg) = if distance_m < 1.0e-6 {
            (0.0, 0.0)
        } else {
            let dir = offset / distance_m;
            let lateral = dir.dot(right);
            let axial = dir.dot(forward);
            (
                lateral.clamp(-1.0, 1.0),
                lateral.atan2(axial).to_degrees(),
            )
        };

        let attenuation = attenuation(source.kind, distance_m) * (1.0 - muffle);
        let volume = (attenuation * source.gain.max(0.0) * loudness * self.master_gain).max(0.0);

        AudibleSource {
            source: *source,
            pan,
            attenuation,
            volume,
            distance_m,
            bearing_deg,
        }
    }

    /// Is this source worth mixing?
    pub fn is_audible(&self, listener: &Listener, source: &SoundSource) -> bool {
        self.resolve(listener, source).volume > self.cull_threshold
    }

    /// Resolve every source and drop the inaudible ones. Results are sorted
    /// loudest-first so a caller with a voice budget can simply truncate,
    /// and so the HUD captions the most urgent cue at the top.
    pub fn mix(&self, listener: &Listener, sources: &[SoundSource]) -> Vec<AudibleSource> {
        let mut out: Vec<AudibleSource> = sources
            .iter()
            .map(|s| self.resolve(listener, s))
            .filter(|a| a.volume > self.cull_threshold)
            .collect();
        out.sort_by(|a, b| {
            b.volume
                .partial_cmp(&a.volume)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out
    }

    /// Captions for an already-mixed frame, loudest cue first (SRS NFR-3).
    pub fn subtitles(&self, mixed: &[AudibleSource]) -> Vec<Subtitle> {
        mixed.iter().map(AudibleSource::subtitle).collect()
    }
}

/// Equal-power stereo channel gains for a pan in `-1..1`.
/// Returns `(left, right)`; `left² + right² == 1`.
pub fn equal_power_gains(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    let angle = (p + 1.0) * 0.25 * std::f32::consts::PI;
    (angle.cos(), angle.sin())
}
