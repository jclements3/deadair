//! da-audio — positional audio as **the fourth optic** (SDD §9, SRS NFR-4).
//!
//! Thermal cannot see zombies and darkness hides everything else, so a moan
//! or a drag-step is often the only warning the player gets. This crate
//! treats that as a gameplay system, not a garnish:
//!
//! * [`kinds`] — the sound taxonomy and its balance table (nominal loudness
//!   and falloff radius per [`SoundKind`]). Pest rustles are deliberately
//!   class-distinguishable.
//! * [`scene`] — the pure core. Given a [`Listener`] and [`SoundSource`]s it
//!   produces pan, attenuation and culling with no backend in sight.
//! * [`subtitle`] — every audible source also yields a [`Subtitle`] so deaf
//!   and hard-of-hearing players get the same channel (SRS NFR-3).
//! * [`synth`] — deterministic procedural placeholder voices, seeded from
//!   [`da_core::Rng`]. Real assets replace these later.
//! * [`backend`] — the only device-touching layer, behind a [`Backend`]
//!   trait with a [`NullBackend`] so everything above is testable headless.
//! * [`event_map`] — turns [`da_sim::events::SimEvent`]s into sources.
//!
//! # Example
//!
//! ```
//! use da_audio::{AudioEngine, AudioScene, Listener, NullBackend, SoundKind, SoundSource};
//! use glam::Vec3;
//!
//! let mut engine = AudioEngine::new(NullBackend::new(), AudioScene::default(), 1234);
//! let listener = Listener::new(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0));
//! // A zombie moaning off to the player's right.
//! let sources = [SoundSource::new(SoundKind::ZombieMoan, Vec3::new(10.0, 0.0, 0.0))];
//!
//! let captions = engine.play_frame(&listener, &sources).expect("null backend never fails");
//! assert_eq!(captions[0].to_line(), "zombie moaning — close, right");
//! ```

#![warn(missing_docs)]

pub mod backend;
pub mod event_map;
pub mod kinds;
pub mod scene;
pub mod subtitle;
pub mod synth;

pub use backend::{AudioError, Backend, NullBackend, PlayCue};
pub use event_map::{sounds_for_event, sounds_for_events, EventAudioCtx};
pub use kinds::{SoundKind, SoundProfile, Surface, WeatherBed};
pub use scene::{attenuation, equal_power_gains, AudibleSource, AudioScene, Listener, SoundSource};
pub use subtitle::{Direction8, DistanceBand, Subtitle};
pub use synth::{synthesize, Voice, SAMPLE_RATE};

use da_core::Rng;

/// Ties the pure scene, the placeholder synthesizer and a [`Backend`]
/// together: the one type a game loop needs.
///
/// Voice variation is deterministic — the engine owns a seeded
/// [`da_core::Rng`] and advances it once per cue, so a replayed night sounds
/// identical.
#[derive(Debug)]
pub struct AudioEngine<B: Backend> {
    /// The output sink.
    pub backend: B,
    /// Spatialisation settings.
    pub scene: AudioScene,
    rng: Rng,
}

impl<B: Backend> AudioEngine<B> {
    /// Build an engine over `backend` with the given scene settings and
    /// synthesis seed.
    pub fn new(backend: B, scene: AudioScene, seed: u64) -> Self {
        Self {
            backend,
            scene,
            rng: Rng::new(seed),
        }
    }

    /// Spatialise `sources`, hand the audible ones to the backend loudest
    /// first, and return their captions for the HUD (SRS NFR-3).
    pub fn play_frame(
        &mut self,
        listener: &Listener,
        sources: &[SoundSource],
    ) -> Result<Vec<Subtitle>, AudioError> {
        self.backend.set_listener(listener);
        let mixed = self.scene.mix(listener, sources);
        let mut captions = Vec::with_capacity(mixed.len());
        for audible in &mixed {
            let voice = synthesize(audible.source.kind, self.rng.next_u64());
            captions.push(audible.subtitle());
            self.backend.play(PlayCue::new(audible, voice))?;
        }
        Ok(captions)
    }

    /// Convenience: map a tick of sim events and play them in one call.
    pub fn play_events<F>(
        &mut self,
        listener: &Listener,
        events: &[da_sim::events::SimEvent],
        ctx: &EventAudioCtx,
        locate: F,
    ) -> Result<Vec<Subtitle>, AudioError>
    where
        F: Fn(da_core::EntityId) -> Option<glam::Vec3> + Copy,
    {
        let sources = sounds_for_events(events, ctx, locate);
        self.play_frame(listener, &sources)
    }
}
