//! Thermal display palettes and auto-gain control (FR-O4, NFR-3).
//!
//! A thermal sensor doesn't show absolute temperature — it windows the
//! scene's current min/max (AGC) and maps that span through a palette.
//! This is why panning from ground to sky visibly shifts the whole image
//! in real footage: the window re-stretches. We reproduce that.

use serde::{Deserialize, Serialize};

/// Palette options per NFR-3 (accessibility): white-hot, black-hot, and a
/// colorblind-safe ramp (luminance-monotonic blue→yellow, no red/green axis).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThermalPalette {
    WhiteHot,
    BlackHot,
    ColorblindSafe,
}

impl ThermalPalette {
    /// Map normalized intensity (0 = window cold end, 1 = hot end) to RGB8.
    pub fn map(self, x: f32) -> [u8; 3] {
        let x = x.clamp(0.0, 1.0);
        match self {
            ThermalPalette::WhiteHot => {
                let v = (x * 255.0).round() as u8;
                [v, v, v]
            }
            ThermalPalette::BlackHot => {
                let v = 255 - (x * 255.0).round() as u8;
                [v, v, v]
            }
            // Cividis-inspired: dark blue → gray → yellow. Luminance rises
            // monotonically, hue never crosses a red/green confusion axis.
            ThermalPalette::ColorblindSafe => {
                let r = (x * x * 255.0).min(255.0) as u8;
                let g = (x * 230.0) as u8;
                let b = ((0.35 + 0.4 * (1.0 - x) - 0.3 * x * x).clamp(0.0, 1.0) * 255.0) as u8;
                [r, g, b]
            }
        }
    }

    /// Bake a 256-entry LUT for the GPU pass.
    pub fn lut(self) -> [[u8; 3]; 256] {
        let mut out = [[0u8; 3]; 256];
        for (i, e) in out.iter_mut().enumerate() {
            *e = self.map(i as f32 / 255.0);
        }
        out
    }
}

/// Auto-gain control: tracks the scene temperature window and normalizes
/// display temps into it, easing toward the instantaneous window so whole-
/// frame value shifts when panning (sky↔ground) look like the real device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agc {
    pub lo_f: f32,
    pub hi_f: f32,
    /// Seconds to converge ~63% toward a new window.
    pub response_sec: f32,
    /// Minimum window span in °F — stops noise amplification on flat scenes
    /// (this is what makes crossover LOOK flat instead of stretched).
    pub min_span_f: f32,
}

impl Agc {
    pub fn new() -> Self {
        Self {
            lo_f: 30.0,
            hi_f: 100.0,
            response_sec: 0.35,
            min_span_f: 8.0,
        }
    }

    /// Feed the frame's observed min/max scene temps; advances the window.
    pub fn update(&mut self, frame_lo: f32, frame_hi: f32, dt: f32) {
        let span = (frame_hi - frame_lo).max(self.min_span_f);
        let mid = 0.5 * (frame_lo + frame_hi);
        let (tlo, thi) = (mid - 0.5 * span, mid + 0.5 * span);
        let k = (dt / self.response_sec).clamp(0.0, 1.0);
        self.lo_f += (tlo - self.lo_f) * k;
        self.hi_f += (thi - self.hi_f) * k;
    }

    /// Normalize a display temperature into the current window.
    pub fn normalize(&self, temp_f: f32) -> f32 {
        ((temp_f - self.lo_f) / (self.hi_f - self.lo_f)).clamp(0.0, 1.0)
    }
}

impl Default for Agc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_hot_is_monotonic_and_black_hot_mirrors_it() {
        for i in 0..255 {
            let a = ThermalPalette::WhiteHot.map(i as f32 / 255.0);
            let b = ThermalPalette::WhiteHot.map((i + 1) as f32 / 255.0);
            assert!(b[0] >= a[0]);
            let inv = ThermalPalette::BlackHot.map(i as f32 / 255.0);
            assert_eq!(inv[0], 255 - a[0]);
        }
    }

    #[test]
    fn colorblind_safe_luminance_monotonic() {
        // Rec.601 luma must rise with intensity — hot things always read
        // brighter regardless of color perception.
        let luma = |c: [u8; 3]| 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
        let mut prev = -1.0;
        for i in 0..=255 {
            let l = luma(ThermalPalette::ColorblindSafe.map(i as f32 / 255.0));
            assert!(l >= prev - 0.75, "luminance dipped at {i}: {l} < {prev}");
            prev = l;
        }
    }

    #[test]
    fn agc_narrow_scene_clamps_to_min_span() {
        let mut agc = Agc::new();
        // Crossover: everything within 2°F. Window must not stretch it.
        for _ in 0..100 {
            agc.update(50.0, 52.0, 0.1);
        }
        assert!((agc.hi_f - agc.lo_f) >= agc.min_span_f * 0.99);
        // A pest 1°F above background maps to a small normalized delta:
        let d = agc.normalize(52.0) - agc.normalize(51.0);
        assert!(d < 0.2, "crossover contrast was artificially stretched: {d}");
    }

    #[test]
    fn agc_pans_toward_new_window() {
        let mut agc = Agc::new();
        for _ in 0..200 {
            agc.update(30.0, 101.0, 0.1); // ground + rabbits
        }
        let ground_before = agc.normalize(45.0);
        for _ in 0..200 {
            agc.update(-20.0, 45.0, 0.1); // tilt up to cold sky
        }
        // Same ground temp now reads near the hot end of the window.
        assert!(agc.normalize(45.0) > ground_before + 0.3);
    }

    #[test]
    fn lut_matches_map() {
        let lut = ThermalPalette::WhiteHot.lut();
        assert_eq!(lut[0], [0, 0, 0]);
        assert_eq!(lut[255], [255, 255, 255]);
    }
}
