//! Subtitles for audio cues (SRS NFR-3).
//!
//! Since audio is a load-bearing detection channel, a deaf or hard-of-hearing
//! player must receive the same information the headphones carry. Every
//! audible source therefore yields a [`Subtitle`] the HUD can render as
//! "raccoon rummaging — near, left".

use serde::{Deserialize, Serialize};

/// Eight-way compass direction **relative to the listener's facing**.
/// [`Direction8::N`] means "dead ahead", not "world north".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Direction8 {
    /// Ahead.
    N,
    /// Ahead-right.
    NE,
    /// Right.
    E,
    /// Behind-right.
    SE,
    /// Behind.
    S,
    /// Behind-left.
    SW,
    /// Left.
    W,
    /// Ahead-left.
    NW,
}

impl Direction8 {
    /// Bucket a relative bearing in degrees (0 = ahead, +90 = right,
    /// increasing clockwise) into one of eight octants.
    pub fn from_bearing_deg(bearing_deg: f32) -> Self {
        let b = bearing_deg.rem_euclid(360.0);
        let idx = ((b / 45.0).round() as usize) % 8;
        [
            Direction8::N,
            Direction8::NE,
            Direction8::E,
            Direction8::SE,
            Direction8::S,
            Direction8::SW,
            Direction8::W,
            Direction8::NW,
        ][idx]
    }

    /// Plain-language label for the HUD.
    pub fn label(self) -> &'static str {
        match self {
            Direction8::N => "ahead",
            Direction8::NE => "ahead-right",
            Direction8::E => "right",
            Direction8::SE => "behind-right",
            Direction8::S => "behind",
            Direction8::SW => "behind-left",
            Direction8::W => "left",
            Direction8::NW => "ahead-left",
        }
    }

    /// Is the source in the rear hemisphere? Panning is front/back
    /// ambiguous, so the caption is the only rear disambiguator.
    pub fn is_behind(self) -> bool {
        matches!(self, Direction8::SE | Direction8::S | Direction8::SW)
    }
}

/// Coarse distance bucket. Bands are expressed as a fraction of the sound's
/// own falloff radius, so "close" means close *for that kind of sound*.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DistanceBand {
    /// Within 25% of the falloff radius — act now.
    Close,
    /// 25%–60% of the falloff radius.
    Near,
    /// Beyond 60% of the falloff radius but still audible.
    Far,
}

/// Upper bound of the [`DistanceBand::Close`] band, as a fraction of falloff.
pub const CLOSE_FRACTION: f32 = 0.25;
/// Upper bound of the [`DistanceBand::Near`] band, as a fraction of falloff.
pub const NEAR_FRACTION: f32 = 0.60;

impl DistanceBand {
    /// Classify a distance against a falloff radius.
    pub fn classify(distance_m: f32, falloff_m: f32) -> Self {
        if falloff_m <= 0.0 {
            return DistanceBand::Close;
        }
        let f = distance_m / falloff_m;
        if f <= CLOSE_FRACTION {
            DistanceBand::Close
        } else if f <= NEAR_FRACTION {
            DistanceBand::Near
        } else {
            DistanceBand::Far
        }
    }

    /// Plain-language label for the HUD.
    pub fn label(self) -> &'static str {
        match self {
            DistanceBand::Close => "close",
            DistanceBand::Near => "near",
            DistanceBand::Far => "far",
        }
    }
}

/// One caption line for one audible source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subtitle {
    /// What is making the noise, e.g. `"raccoon rummaging"`.
    pub text: &'static str,
    /// Where it is, relative to where the player is looking.
    pub direction: Direction8,
    /// How far, relative to how far that sound carries.
    pub distance_band: DistanceBand,
}

impl Subtitle {
    /// Render the HUD line, e.g. `"raccoon rummaging — near, left"`.
    pub fn to_line(&self) -> String {
        format!(
            "{} — {}, {}",
            self.text,
            self.distance_band.label(),
            self.direction.label()
        )
    }
}
