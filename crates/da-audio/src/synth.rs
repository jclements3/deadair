//! Procedural placeholder sounds.
//!
//! **These are programmer art.** Every voice here is a short noise burst or
//! filtered tone shaped by an envelope, chosen only to be *class
//! distinguishable* through headphones — a rat ticks, a raccoon clatters, a
//! zombie moans low. Real recorded assets replace these later; the intended
//! swap is to keep [`SoundKind`] and [`Voice`] identical and change
//! [`synthesize`] into an asset lookup with the same signature.
//!
//! Synthesis is **deterministic**: the same `(kind, seed)` always produces
//! bit-identical samples, because it draws from [`da_core::Rng`] seeded from
//! the kind's stable [`SoundKind::synth_id`].

use std::f32::consts::TAU;
use std::sync::Arc;

use da_core::Rng;

use crate::kinds::{SoundKind, Surface, WeatherBed};

/// Sample rate of all synthesized placeholder audio.
pub const SAMPLE_RATE: u32 = 48_000;

/// A rendered mono voice, ready for a backend to pan and play.
#[derive(Debug, Clone, PartialEq)]
pub struct Voice {
    /// What this is a rendering of.
    pub kind: SoundKind,
    /// Mono PCM in roughly `-1..1`.
    pub samples: Arc<[f32]>,
    /// Samples per second.
    pub sample_rate: u32,
}

impl Voice {
    /// Duration in seconds.
    pub fn duration_s(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// One-pole low-pass, in place. `cutoff` is a 0..1 coefficient (1 = bypass).
fn low_pass(buf: &mut [f32], cutoff: f32) {
    let a = cutoff.clamp(0.001, 1.0);
    let mut z = 0.0;
    for s in buf.iter_mut() {
        z += a * (*s - z);
        *s = z;
    }
}

/// One-pole high-pass, in place.
fn high_pass(buf: &mut [f32], cutoff: f32) {
    let a = cutoff.clamp(0.001, 1.0);
    let mut z = 0.0;
    for s in buf.iter_mut() {
        z += a * (*s - z);
        *s -= z;
    }
}

/// Normalise to a peak of `peak` (no-op on silence).
fn normalize(buf: &mut [f32], peak: f32) {
    let max = buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    if max > 1.0e-9 {
        let k = peak / max;
        for s in buf.iter_mut() {
            *s *= k;
        }
    }
}

/// Short fade in/out so nothing clicks.
fn declick(buf: &mut [f32], sample_rate: u32) {
    let n = (sample_rate as usize / 400).max(1).min(buf.len() / 2);
    let len = buf.len();
    for i in 0..n {
        let k = i as f32 / n as f32;
        buf[i] *= k;
        buf[len - 1 - i] *= k;
    }
}

/// White noise burst with an exponential decay envelope.
fn noise_burst(buf: &mut [f32], rng: &mut Rng, decay: f32) {
    let n = buf.len() as f32;
    for (i, s) in buf.iter_mut().enumerate() {
        let env = (-decay * (i as f32 / n)).exp();
        *s = rng.range(-1.0, 1.0) * env;
    }
}

/// Sum of a fundamental and two harmonics with vibrato.
fn tone(buf: &mut [f32], sample_rate: u32, hz: f32, vibrato_hz: f32, vibrato_depth: f32) {
    let n = buf.len() as f32;
    let mut phase = 0.0_f32;
    for (i, s) in buf.iter_mut().enumerate() {
        let t = i as f32 / sample_rate as f32;
        let f = hz * (1.0 + vibrato_depth * (TAU * vibrato_hz * t).sin());
        phase += TAU * f / sample_rate as f32;
        let env = {
            let x = i as f32 / n;
            // Gentle attack, long release.
            (x * 12.0).min(1.0) * (1.0 - x).powf(1.5)
        };
        *s += (phase.sin() + 0.35 * (2.0 * phase).sin() + 0.15 * (3.0 * phase).sin()) * env;
    }
}

/// Repeated ticks — the shared skeleton of every pest rustle. `count` ticks
/// spread across the buffer, each a filtered noise chirp.
fn ticks(buf: &mut [f32], rng: &mut Rng, sample_rate: u32, count: usize, tick_s: f32, jitter: f32) {
    let tick_len = ((tick_s * sample_rate as f32) as usize).max(4);
    let len = buf.len();
    for k in 0..count {
        let nominal = (k as f32 + 0.5) / count as f32;
        let pos = (nominal + rng.range(-jitter, jitter)).clamp(0.0, 0.999);
        let start = (pos * len as f32) as usize;
        let amp = rng.range(0.5, 1.0);
        for i in 0..tick_len {
            let idx = start + i;
            if idx >= len {
                break;
            }
            let env = (-6.0 * (i as f32 / tick_len as f32)).exp();
            buf[idx] += rng.range(-1.0, 1.0) * env * amp;
        }
    }
}

/// Render the placeholder voice for `kind`, deterministically from `seed`.
///
/// The same `(kind, seed)` pair always yields identical samples; different
/// seeds give variation so repeated rat scurries are not audibly looped.
pub fn synthesize(kind: SoundKind, seed: u64) -> Voice {
    let profile = kind.profile();
    let sr = SAMPLE_RATE;
    let n = ((profile.duration_s * sr as f32) as usize).max(16);
    let mut buf = vec![0.0_f32; n];
    // Mix the caller's seed with the kind's stable id so kinds never collide.
    let mut rng = Rng::new(seed ^ (kind.synth_id().wrapping_mul(0x1000_0000_1B3)));

    use SoundKind::*;
    match kind {
        // --- pests: all ticks, distinguished by rate, filter and weight ---
        RatScurry => {
            ticks(&mut buf, &mut rng, sr, 14, 0.010, 0.03);
            high_pass(&mut buf, 0.55);
        }
        RabbitRustle => {
            // Slower than a rat's tick, softer than a possum's drag — a
            // rhythmic crop-crop-crop with the top rolled off.
            ticks(&mut buf, &mut rng, sr, 7, 0.022, 0.05);
            low_pass(&mut buf, 0.30);
        }
        PossumShuffle => {
            ticks(&mut buf, &mut rng, sr, 5, 0.070, 0.06);
            low_pass(&mut buf, 0.20);
        }
        RaccoonRummage => {
            ticks(&mut buf, &mut rng, sr, 9, 0.035, 0.10);
            high_pass(&mut buf, 0.30);
            low_pass(&mut buf, 0.65);
        }
        HogRoot => {
            ticks(&mut buf, &mut rng, sr, 4, 0.120, 0.05);
            low_pass(&mut buf, 0.12);
        }
        GroundhogScrabble => {
            ticks(&mut buf, &mut rng, sr, 11, 0.014, 0.05);
            high_pass(&mut buf, 0.40);
            low_pass(&mut buf, 0.80);
        }
        BeaverSlap => {
            noise_burst(&mut buf, &mut rng, 9.0);
            low_pass(&mut buf, 0.35);
        }

        // --- vocalisations ---
        HogGrunt => {
            tone(&mut buf, sr, 90.0, 11.0, 0.10);
            low_pass(&mut buf, 0.30);
        }
        ZombieMoan => {
            tone(&mut buf, sr, 78.0, 3.5, 0.06);
            let mut breath = vec![0.0_f32; n];
            noise_burst(&mut breath, &mut rng, 1.2);
            low_pass(&mut breath, 0.06);
            for (s, b) in buf.iter_mut().zip(breath) {
                *s += b * 0.5;
            }
            low_pass(&mut buf, 0.25);
        }
        ZombieDragStep => {
            ticks(&mut buf, &mut rng, sr, 2, 0.150, 0.02);
            low_pass(&mut buf, 0.10);
        }
        DogBark => {
            tone(&mut buf, sr, 320.0, 22.0, 0.18);
            noise_burst(&mut buf, &mut rng, 14.0);
            high_pass(&mut buf, 0.20);
        }
        CowLow => {
            tone(&mut buf, sr, 130.0, 2.0, 0.03);
            low_pass(&mut buf, 0.28);
        }
        SheepBleat => {
            tone(&mut buf, sr, 300.0, 18.0, 0.22);
        }
        CatMeow => {
            tone(&mut buf, sr, 480.0, 4.0, 0.14);
        }
        PlayerGrunt => {
            tone(&mut buf, sr, 110.0, 6.0, 0.05);
            low_pass(&mut buf, 0.22);
        }

        // --- world / weapon ---
        CreekAmbience => {
            noise_burst(&mut buf, &mut rng, 0.0);
            high_pass(&mut buf, 0.50);
            low_pass(&mut buf, 0.85);
        }
        RifleDischarge => {
            noise_burst(&mut buf, &mut rng, 22.0);
            high_pass(&mut buf, 0.70);
        }
        RifleDischargeModerated => {
            noise_burst(&mut buf, &mut rng, 30.0);
            low_pass(&mut buf, 0.18);
        }
        PumpStroke => {
            ticks(&mut buf, &mut rng, sr, 2, 0.030, 0.0);
            high_pass(&mut buf, 0.35);
            low_pass(&mut buf, 0.75);
        }
        PelletImpact => {
            noise_burst(&mut buf, &mut rng, 40.0);
            high_pass(&mut buf, 0.60);
        }
        Footstep(s) => {
            noise_burst(&mut buf, &mut rng, 26.0);
            match s {
                Surface::Grass => low_pass(&mut buf, 0.25),
                Surface::Gravel => high_pass(&mut buf, 0.55),
                Surface::Mud => low_pass(&mut buf, 0.10),
                Surface::Wood => {
                    tone(&mut buf, sr, 180.0, 0.0, 0.0);
                    low_pass(&mut buf, 0.45);
                }
                Surface::Water => {
                    high_pass(&mut buf, 0.30);
                    low_pass(&mut buf, 0.70);
                }
            }
        }
        Weather(w) => {
            noise_burst(&mut buf, &mut rng, 0.0);
            match w {
                WeatherBed::Wind => low_pass(&mut buf, 0.02),
                WeatherBed::Rain => high_pass(&mut buf, 0.45),
            }
        }
    }

    normalize(&mut buf, 0.9);
    declick(&mut buf, sr);

    Voice {
        kind,
        samples: buf.into(),
        sample_rate: sr,
    }
}
