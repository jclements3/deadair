//! Translating the sim's event stream into sound.
//!
//! [`da_sim::events::SimEvent`] is the decoupling seam: audio reads it and
//! never reaches into sim state. Some events carry no position (they name an
//! [`da_core::EntityId`] instead); for those the caller supplies a resolver
//! so the sound lands where the entity actually is, and we fall back to the
//! player's own position when the entity is gone.

use da_core::EntityId;
use da_sim::events::{DamageCause, SimEvent};
use da_sim::hit::Species;
use da_sim::noise::NoiseKind;
use glam::Vec3;

use crate::kinds::{SoundKind, Surface};
use crate::scene::SoundSource;

/// Per-frame context the mapping needs but the event stream does not carry.
#[derive(Debug, Clone, Copy)]
pub struct EventAudioCtx {
    /// Where the player is — the fallback position for positionless events.
    pub player_pos: Vec3,
    /// Whether the player's rifle currently wears a moderator. Selects
    /// [`SoundKind::RifleDischargeModerated`] over [`SoundKind::RifleDischarge`].
    pub moderated: bool,
    /// Surface under the player, for generic movement noise.
    pub surface: Surface,
}

impl Default for EventAudioCtx {
    fn default() -> Self {
        Self {
            player_pos: Vec3::ZERO,
            moderated: false,
            surface: Surface::Grass,
        }
    }
}

/// The idle/movement sound a species makes — the rustle the player must
/// classify before the trigger (SDD §6). Friendlies map to their call.
pub fn species_sound(species: Species) -> Option<SoundKind> {
    Some(match species {
        Species::Rat => SoundKind::RatScurry,
        Species::Possum => SoundKind::PossumShuffle,
        Species::Raccoon => SoundKind::RaccoonRummage,
        Species::Groundhog => SoundKind::GroundhogScrabble,
        Species::Beaver => SoundKind::BeaverSlap,
        Species::JuvenileFeralHog => SoundKind::HogRoot,
        Species::Dog => SoundKind::DogBark,
        Species::Cat => SoundKind::CatMeow,
        Species::Cow => SoundKind::CowLow,
        Species::Sheep => SoundKind::SheepBleat,
        Species::Zombie => SoundKind::ZombieMoan,
    })
}

/// The distress vocalisation a species makes when hit — hogs grunt (and then
/// charge), friendlies cry out, rodents are effectively silent.
pub fn species_hurt_sound(species: Species) -> Option<SoundKind> {
    match species {
        Species::JuvenileFeralHog => Some(SoundKind::HogGrunt),
        Species::Dog => Some(SoundKind::DogBark),
        Species::Cow => Some(SoundKind::CowLow),
        Species::Sheep => Some(SoundKind::SheepBleat),
        Species::Cat => Some(SoundKind::CatMeow),
        Species::Zombie => Some(SoundKind::ZombieMoan),
        _ => None,
    }
}

/// Map one [`SimEvent`] to the sounds it should make this frame.
///
/// `locate` resolves an [`EntityId`] to a world position; return `None` and
/// the sound is placed at [`EventAudioCtx::player_pos`]. Pass
/// `|_| None` if you have no entity index.
pub fn sounds_for_event<F>(
    event: &SimEvent,
    ctx: &EventAudioCtx,
    locate: F,
) -> Vec<SoundSource>
where
    F: Fn(EntityId) -> Option<Vec3>,
{
    let at = |id: EntityId| locate(id).unwrap_or(ctx.player_pos);
    let mut out = Vec::new();

    match event {
        SimEvent::NoiseMade { pos, kind, .. } => match kind {
            NoiseKind::Discharge => out.push(SoundSource::new(
                if ctx.moderated {
                    SoundKind::RifleDischargeModerated
                } else {
                    SoundKind::RifleDischarge
                },
                *pos,
            )),
            NoiseKind::PumpStroke => out.push(SoundSource::new(SoundKind::PumpStroke, *pos)),
            NoiseKind::Other => {
                out.push(SoundSource::new(SoundKind::Footstep(ctx.surface), *pos))
            }
        },

        SimEvent::KillConfirmed { species, pos, .. } => {
            out.push(SoundSource::new(SoundKind::PelletImpact, *pos));
            if let Some(k) = species_hurt_sound(*species) {
                // A clean kill cuts the cry short.
                out.push(SoundSource::new(k, *pos).with_gain(0.5));
            }
        }

        SimEvent::Wounded { species, pos, .. } => {
            out.push(SoundSource::new(SoundKind::PelletImpact, *pos));
            if let Some(k) = species_hurt_sound(*species) {
                out.push(SoundSource::new(k, *pos));
            }
        }

        SimEvent::FriendlyHit { id, species } => {
            let pos = at(*id);
            out.push(SoundSource::new(SoundKind::PelletImpact, pos));
            if let Some(k) = species_hurt_sound(*species) {
                out.push(SoundSource::new(k, pos).with_gain(1.25));
            }
        }

        SimEvent::ZombieDestroyed { id } => {
            out.push(SoundSource::new(SoundKind::PelletImpact, at(*id)));
        }

        SimEvent::ZombieStaggered { id } => {
            let pos = at(*id);
            out.push(SoundSource::new(SoundKind::PelletImpact, pos));
            out.push(SoundSource::new(SoundKind::ZombieMoan, pos).with_gain(0.7));
        }

        SimEvent::PlayerDamaged { amount, cause } => {
            let gain = (0.5 + amount / 40.0).clamp(0.5, 1.5);
            out.push(SoundSource::new(SoundKind::PlayerGrunt, ctx.player_pos).with_gain(gain));
            if matches!(cause, DamageCause::ZombieContact) {
                out.push(SoundSource::new(SoundKind::ZombieMoan, ctx.player_pos));
            }
        }

        SimEvent::PestFled { id, species } => {
            if let Some(k) = species_sound(*species) {
                // A bolting animal is louder than a feeding one.
                out.push(SoundSource::new(k, at(*id)).with_gain(1.5));
            }
        }

        SimEvent::Missed { impact } => {
            if let Some(p) = impact {
                out.push(SoundSource::new(SoundKind::PelletImpact, *p));
            }
        }

        // A dry trigger is a click, not a shot — same mechanism, tiny sound.
        SimEvent::DryFire => {
            out.push(SoundSource::new(SoundKind::PumpStroke, ctx.player_pos).with_gain(0.4));
        }

        // Purely visual — the thermal layer owns it.
        SimEvent::HeatResidue { .. } => {}
    }

    out
}

/// Map a whole tick's worth of events.
pub fn sounds_for_events<F>(
    events: &[SimEvent],
    ctx: &EventAudioCtx,
    locate: F,
) -> Vec<SoundSource>
where
    F: Fn(EntityId) -> Option<Vec3> + Copy,
{
    events
        .iter()
        .flat_map(|e| sounds_for_event(e, ctx, locate))
        .collect()
}
