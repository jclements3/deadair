//! Output backends.
//!
//! Everything above this module is pure and device-free. A [`Backend`] is the
//! only thing that touches a sound card, and [`NullBackend`] satisfies the
//! trait without one — which is why the whole test suite runs headless
//! (CI and WSL2 have no reliable audio sink).

use crate::kinds::SoundKind;
use crate::scene::{AudibleSource, Listener};
use crate::synth::Voice;

/// Anything that can go wrong on the way to the speakers.
#[derive(Debug)]
pub enum AudioError {
    /// No output device, or the device rejected the stream.
    Device(String),
    /// The backend refused a cue (voice budget, bad data, ...).
    Playback(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Device(m) => write!(f, "audio device error: {m}"),
            AudioError::Playback(m) => write!(f, "audio playback error: {m}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// One fully-resolved request to make a noise.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayCue {
    /// What is playing.
    pub kind: SoundKind,
    /// Pan, -1 left .. +1 right.
    pub pan: f32,
    /// Final linear volume, 0..1.
    pub volume: f32,
    /// Occlusion amount, 0..1 — backends may map this to a low-pass.
    pub muffle: f32,
    /// The mono PCM to play.
    pub voice: Voice,
}

impl PlayCue {
    /// Build a cue from a resolved scene source and its rendered voice.
    pub fn new(audible: &AudibleSource, voice: Voice) -> Self {
        Self {
            kind: audible.source.kind,
            pan: audible.pan,
            volume: audible.volume,
            muffle: audible.source.muffle,
            voice,
        }
    }
}

/// A sink for [`PlayCue`]s.
pub trait Backend {
    /// Play one cue. Backends are expected to be non-blocking.
    fn play(&mut self, cue: PlayCue) -> Result<(), AudioError>;

    /// Tell the backend where the ears are. Purely informational for
    /// backends that mix from the pre-computed pan (all current ones);
    /// present so an HRTF backend can be dropped in later.
    fn set_listener(&mut self, _listener: &Listener) {}

    /// Stop everything currently sounding.
    fn stop_all(&mut self) {}
}

/// A backend that plays nothing and remembers everything.
///
/// Used by the test suite and by headless tools. `cues` is the full history
/// since the last [`NullBackend::clear`].
#[derive(Debug, Default)]
pub struct NullBackend {
    /// Every cue handed to this backend, in order.
    pub cues: Vec<PlayCue>,
    /// The most recent listener passed to [`Backend::set_listener`].
    pub listener: Option<Listener>,
    /// How many times [`Backend::stop_all`] was called.
    pub stops: usize,
}

impl NullBackend {
    /// An empty recorder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget all recorded cues.
    pub fn clear(&mut self) {
        self.cues.clear();
    }

    /// Kinds played so far, in order.
    pub fn kinds(&self) -> Vec<SoundKind> {
        self.cues.iter().map(|c| c.kind).collect()
    }
}

impl Backend for NullBackend {
    fn play(&mut self, cue: PlayCue) -> Result<(), AudioError> {
        self.cues.push(cue);
        Ok(())
    }

    fn set_listener(&mut self, listener: &Listener) {
        self.listener = Some(*listener);
    }

    fn stop_all(&mut self) {
        self.stops += 1;
    }
}

#[cfg(feature = "kira-backend")]
mod kira_backend;
#[cfg(feature = "kira-backend")]
pub use kira_backend::KiraBackend;
