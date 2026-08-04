//! The sound taxonomy and its per-kind acoustic profile.
//!
//! [`SoundKind`] is deliberately backend-free: it is a gameplay vocabulary,
//! not a list of files. Every kind carries a *nominal loudness* (0..1, the
//! gain at the reference distance) and a *falloff radius* in meters beyond
//! which the source is silent. Those two numbers are the balance knobs for
//! the "audio is the fourth optic" design (SDD §9, SRS NFR-4).

use serde::{Deserialize, Serialize};

/// Ground material under the player's boot — footsteps are surface-coded so
/// the player can hear their own noise discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Surface {
    /// Soft, quietest.
    Grass,
    /// Loud and crunchy — the noise-discipline punisher.
    Gravel,
    /// Wet suck, quiet but distinctive.
    Mud,
    /// Porch/barn boards — hollow knock.
    Wood,
    /// Creek crossing — splash.
    Water,
}

/// Weather bed layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WeatherBed {
    /// Wind through trees — masks pest rustle.
    Wind,
    /// Rain — masks everything, the hardest night.
    Rain,
}

/// Every sound the game can emit, as a gameplay concept.
///
/// Pest rustles are **class-distinguishable** on purpose: identifying a
/// raccoon rummage from a rat scurry before the trigger is the intended
/// skill (SDD §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SoundKind {
    /// Rat — fast, high, ticking scurry.
    RatScurry,
    /// Rabbit — soft grass-cropping nibble; sits between rat and possum in
    /// loudness and reach.
    RabbitRustle,
    /// Possum — slow, dragging shuffle.
    PossumShuffle,
    /// Raccoon — busy, clattering rummage.
    RaccoonRummage,
    /// Feral hog — wet rooting in dirt.
    HogRoot,
    /// Feral hog — low grunt (the warning before a charge).
    HogGrunt,
    /// Groundhog — dry scrabble at a burrow mouth.
    GroundhogScrabble,
    /// Beaver — water slap / gnawing.
    BeaverSlap,
    /// Zombie — the long moan. Primary warning channel: thermal cannot see
    /// zombies, so this must carry farther than any pest sound.
    ZombieMoan,
    /// Zombie — asymmetric drag-step footfall.
    ZombieDragStep,
    /// Friendly dog barking — proximity warning, "do not shoot that blob".
    DogBark,
    /// Friendly cow lowing.
    CowLow,
    /// Friendly sheep bleating.
    SheepBleat,
    /// Friendly cat.
    CatMeow,
    /// Running creek — hazard ambience and a masking bed.
    CreekAmbience,
    /// Unmoderated rifle discharge — the loudest thing in the night.
    RifleDischarge,
    /// Moderated discharge — same event, dramatically smaller footprint.
    RifleDischargeModerated,
    /// Multi-pump stroke.
    PumpStroke,
    /// Pellet striking terrain or a body.
    PelletImpact,
    /// Player footstep on a given surface.
    Footstep(Surface),
    /// Player pain grunt.
    PlayerGrunt,
    /// Non-positional weather bed.
    Weather(WeatherBed),
}

/// Static acoustic profile of a [`SoundKind`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoundProfile {
    /// Gain at the reference distance, 0..1.
    pub loudness: f32,
    /// Distance in meters at which the source becomes exactly inaudible.
    pub falloff_m: f32,
    /// Nominal duration of the placeholder sample, seconds.
    pub duration_s: f32,
    /// HUD caption text (SRS NFR-3).
    pub caption: &'static str,
}

/// Distance at and below which no inverse-distance attenuation is applied.
pub const REFERENCE_DISTANCE_M: f32 = 1.0;

/// Effective gain below which a source is culled entirely.
pub const AUDIBLE_EPSILON: f32 = 1.0e-3;

impl SoundKind {
    /// The kind's acoustic profile. This is the single balance table.
    pub fn profile(self) -> SoundProfile {
        use SoundKind::*;
        let (loudness, falloff_m, duration_s, caption) = match self {
            RatScurry => (0.15, 8.0, 0.35, "rat scurrying"),
            RabbitRustle => (0.17, 9.0, 0.40, "rabbit nibbling"),
            PossumShuffle => (0.20, 11.0, 0.60, "possum shuffling"),
            RaccoonRummage => (0.30, 16.0, 0.80, "raccoon rummaging"),
            HogRoot => (0.40, 20.0, 0.90, "hog rooting"),
            HogGrunt => (0.55, 26.0, 0.50, "hog grunting"),
            GroundhogScrabble => (0.22, 12.0, 0.45, "groundhog scrabbling"),
            BeaverSlap => (0.45, 24.0, 0.30, "beaver slapping water"),
            // Deliberately the longest-carrying creature sound in the game:
            // thermal hides zombies, so the moan is the detection channel.
            ZombieMoan => (0.70, 60.0, 1.80, "zombie moaning"),
            ZombieDragStep => (0.38, 28.0, 0.55, "dragging footsteps"),
            DogBark => (0.85, 75.0, 0.40, "dog barking"),
            CowLow => (0.60, 55.0, 1.20, "cow lowing"),
            SheepBleat => (0.45, 40.0, 0.70, "sheep bleating"),
            CatMeow => (0.30, 25.0, 0.60, "cat meowing"),
            CreekAmbience => (0.50, 35.0, 2.00, "running water"),
            RifleDischarge => (1.00, 240.0, 0.45, "gunshot"),
            // Mirrors da_sim::noise::MODERATOR_FACTOR (0.3) on radius, with a
            // further cut in loudness — a moderated shot is a thud, not a bang.
            RifleDischargeModerated => (0.15, 72.0, 0.30, "muffled shot"),
            PumpStroke => (0.25, 14.0, 0.25, "pump stroke"),
            PelletImpact => (0.30, 26.0, 0.20, "pellet impact"),
            Footstep(s) => match s {
                Surface::Grass => (0.16, 9.0, 0.18, "footsteps on grass"),
                Surface::Gravel => (0.34, 18.0, 0.18, "footsteps on gravel"),
                Surface::Mud => (0.20, 11.0, 0.24, "footsteps in mud"),
                Surface::Wood => (0.30, 16.0, 0.18, "footsteps on wood"),
                Surface::Water => (0.32, 17.0, 0.28, "footsteps in water"),
            },
            PlayerGrunt => (0.60, 6.0, 0.50, "you grunt in pain"),
            Weather(w) => match w {
                WeatherBed::Wind => (0.35, 10_000.0, 3.00, "wind"),
                WeatherBed::Rain => (0.45, 10_000.0, 3.00, "rain"),
            },
        };
        SoundProfile {
            loudness,
            falloff_m,
            duration_s,
            caption,
        }
    }

    /// Convenience: nominal loudness at the reference distance.
    pub fn loudness(self) -> f32 {
        self.profile().loudness
    }

    /// Convenience: silence radius in meters.
    pub fn falloff_m(self) -> f32 {
        self.profile().falloff_m
    }

    /// HUD caption for subtitles (SRS NFR-3).
    pub fn caption(self) -> &'static str {
        self.profile().caption
    }

    /// A weather bed and other global layers are not positional; the scene
    /// plays them centred with no attenuation.
    pub fn is_ambient_bed(self) -> bool {
        matches!(self, SoundKind::Weather(_))
    }

    /// Stable small integer used to seed deterministic synthesis. Never
    /// reorder these values — saved seeds would change their sound.
    pub fn synth_id(self) -> u64 {
        use SoundKind::*;
        match self {
            RatScurry => 1,
            PossumShuffle => 2,
            RaccoonRummage => 3,
            HogRoot => 4,
            HogGrunt => 5,
            GroundhogScrabble => 6,
            BeaverSlap => 7,
            ZombieMoan => 8,
            ZombieDragStep => 9,
            DogBark => 10,
            CowLow => 11,
            SheepBleat => 12,
            CatMeow => 13,
            CreekAmbience => 14,
            RifleDischarge => 15,
            RifleDischargeModerated => 16,
            PumpStroke => 17,
            PelletImpact => 18,
            PlayerGrunt => 19,
            Footstep(s) => {
                20 + match s {
                    Surface::Grass => 0,
                    Surface::Gravel => 1,
                    Surface::Mud => 2,
                    Surface::Wood => 3,
                    Surface::Water => 4,
                }
            }
            Weather(w) => {
                30 + match w {
                    WeatherBed::Wind => 0,
                    WeatherBed::Rain => 1,
                }
            }
            // Appended (never renumber): 20–24 and 30–31 are taken above.
            RabbitRustle => 32,
        }
    }
}
