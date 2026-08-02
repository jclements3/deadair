use serde::{Deserialize, Serialize};

/// Temperature in degrees Fahrenheit (the game's display unit; the thermal
/// sim works directly in °F since only differences matter).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct TempF(pub f32);

impl TempF {
    pub const PEST_BODY: TempF = TempF(101.0);

    pub fn lerp(self, other: TempF, t: f32) -> TempF {
        TempF(self.0 + (other.0 - self.0) * t)
    }
}

impl std::ops::Sub for TempF {
    type Output = f32;
    fn sub(self, rhs: TempF) -> f32 {
        self.0 - rhs.0
    }
}

impl std::ops::Add<f32> for TempF {
    type Output = TempF;
    fn add(self, rhs: f32) -> TempF {
        TempF(self.0 + rhs)
    }
}

/// Yards → meters (world units are meters; bounty tables and scope talk in yards).
pub fn yards_to_m(yd: f32) -> f32 {
    yd * 0.9144
}

pub fn m_to_yards(m: f32) -> f32 {
    m / 0.9144
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temp_lerp_midpoint() {
        assert_eq!(TempF(50.0).lerp(TempF(70.0), 0.5).0, 60.0);
    }

    #[test]
    fn yard_round_trip() {
        let m = yards_to_m(100.0);
        assert!((m_to_yards(m) - 100.0).abs() < 1e-4);
    }
}
