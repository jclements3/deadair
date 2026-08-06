//! [`kira`]-backed output. Enabled by the `kira-backend` feature.
//!
//! Chosen over `rodio` because kira is a *game* audio library: it owns a
//! fire-and-forget voice pool, per-sound panning and volume as first-class
//! parameters with tweens, and it accepts raw in-memory PCM frames — which
//! is exactly what [`crate::synth`] produces, with no asset files or
//! decoders in the loop.
//!
//! This backend deliberately does no spatial maths of its own. The pure
//! [`crate::scene`] layer already resolved pan and volume; kira only mixes.

use std::sync::Arc;

use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::{AudioManager, AudioManagerSettings, DefaultBackend, Decibels, Frame, Panning};

use crate::backend::{AudioError, Backend, PlayCue};
use crate::scene::Listener;

/// Volume below which we do not bother waking the mixer.
const MIN_AUDIBLE_DB: f32 = -55.0;

/// A real sound-card backend.
pub struct KiraBackend {
    manager: AudioManager<DefaultBackend>,
}

impl std::fmt::Debug for KiraBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KiraBackend").finish_non_exhaustive()
    }
}

impl KiraBackend {
    /// Open the default output device.
    ///
    /// Returns [`AudioError::Device`] when there is no usable sink — call
    /// sites should fall back to [`crate::NullBackend`] rather than abort,
    /// since the subtitle channel still works without audio (SRS NFR-3).
    pub fn new() -> Result<Self, AudioError> {
        let manager = AudioManager::<DefaultBackend>::new(AudioManagerSettings::default())
            .map_err(|e| AudioError::Device(e.to_string()))?;
        Ok(Self { manager })
    }
}

/// Linear amplitude in 0..1 to decibels, floored at silence.
fn to_decibels(amplitude: f32) -> Decibels {
    if amplitude <= 1.0e-4 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * amplitude.log10())
    }
}

/// One-pole low-pass used to voice the occlusion hook. Cheap, and enough to
/// make a muffled sound read as "through a wall".
fn muffle_filter(frames: &mut [Frame], muffle: f32) {
    if muffle <= 0.0 {
        return;
    }
    let cutoff = (1.0 - muffle).clamp(0.02, 1.0);
    let mut z = Frame::new(0.0, 0.0);
    for f in frames.iter_mut() {
        z.left += cutoff * (f.left - z.left);
        z.right += cutoff * (f.right - z.right);
        *f = z;
    }
}

impl Backend for KiraBackend {
    fn play(&mut self, cue: PlayCue) -> Result<(), AudioError> {
        let db = to_decibels(cue.volume);
        if db.0 <= MIN_AUDIBLE_DB {
            return Ok(());
        }

        // Mono voice -> stereo frames; kira applies the panning itself, so
        // we duplicate the channel rather than pre-panning.
        let mut frames: Vec<Frame> = cue
            .voice
            .samples
            .iter()
            .map(|&s| Frame::new(s, s))
            .collect();
        muffle_filter(&mut frames, cue.muffle);

        let data = StaticSoundData {
            sample_rate: cue.voice.sample_rate,
            frames: Arc::from(frames),
            settings: StaticSoundSettings::default(),
            slice: None,
        }
        .volume(db)
        .panning(Panning(cue.pan.clamp(-1.0, 1.0)));

        self.manager
            .play(data)
            .map(|_handle| ())
            .map_err(|e| AudioError::Playback(e.to_string()))
    }

    fn set_listener(&mut self, _listener: &Listener) {
        // Panning is pre-resolved by AudioScene; nothing to tell kira.
    }

    fn stop_all(&mut self) {
        // Placeholder voices are all sub-two-second one-shots, so letting
        // them ring out is correct. A future looping-ambience layer will
        // need handles retained here.
    }
}
