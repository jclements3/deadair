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

/// One temperature observation of the frame, weighted by how much of the
/// screen it covers. The AGC histogram is built from these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempSample {
    /// Display temperature in °F.
    pub temp_f: f32,
    /// Approximate fraction of the frame this sample covers (any positive
    /// scale works — weights are normalized before the percentile scan).
    pub weight: f32,
}

/// Auto-gain control: tracks the scene temperature window and normalizes
/// display temps into it, easing toward the instantaneous window so whole-
/// frame value shifts when panning (sky↔ground) look like the real device.
///
/// The window is derived from a *coverage-weighted percentile* of the
/// frame's temperature distribution ([`Agc::update_weighted`]) rather than
/// its raw min/max: a real device does not let one sliver of cold sky
/// compress all the ground contrast, because that sliver is a negligible
/// slice of the histogram. [`Agc::update`] (raw min/max) is kept for
/// callers that only have a range.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agc {
    pub lo_f: f32,
    pub hi_f: f32,
    /// Seconds to converge ~63% toward a new window.
    pub response_sec: f32,
    /// Minimum window span in °F — stops noise amplification on flat scenes
    /// (this is what makes crossover LOOK flat instead of stretched).
    pub min_span_f: f32,
    /// Fraction of frame coverage clipped off the cold tail (0.02 = 2nd
    /// percentile). This is the cold-sky rejection.
    pub clip_lo: f32,
    /// Fraction of frame coverage clipped off the hot tail (98th percentile
    /// at 0.02).
    pub clip_hi: f32,
    /// How much of the clipped hot tail is folded back into the window
    /// (0 = discard it entirely, 1 = fall back to the raw max). Hot targets
    /// are the signal a hunter is looking for: real devices let them
    /// saturate white, but a scope that fully discarded the tail would map
    /// warm ground to white too. 0.5 keeps ground mid-gray and targets
    /// blazing, matching `thermal_ref_wide.png`.
    pub hot_headroom: f32,
    /// Same, for the cold tail. Defaults to 0 — cold sky is rejected hard,
    /// which is the whole point of the percentile window.
    pub cold_headroom: f32,
}

impl Agc {
    pub fn new() -> Self {
        Self {
            lo_f: 30.0,
            hi_f: 100.0,
            response_sec: 0.35,
            min_span_f: 8.0,
            clip_lo: 0.02,
            clip_hi: 0.02,
            hot_headroom: 0.5,
            cold_headroom: 0.0,
        }
    }

    /// Feed a coverage-weighted temperature histogram for this frame and
    /// advance the window toward its clipped percentile range.
    ///
    /// Samples with negligible coverage (a distant cold puddle, a sliver of
    /// sky above the treeline) fall inside the clipped tails and therefore
    /// cannot flatten the rest of the image. Falls back to no-op on an empty
    /// or zero-weight histogram.
    pub fn update_weighted(&mut self, samples: &[TempSample], dt: f32) {
        let total: f32 = samples.iter().map(|s| s.weight.max(0.0)).sum();
        if samples.is_empty() || !(total > 0.0) {
            return;
        }
        let mut sorted: Vec<TempSample> = samples
            .iter()
            .copied()
            .filter(|s| s.weight > 0.0 && s.temp_f.is_finite())
            .collect();
        if sorted.is_empty() {
            return;
        }
        sorted.sort_by(|a, b| a.temp_f.total_cmp(&b.temp_f));
        let pick = |frac: f32| -> f32 {
            let target = (frac.clamp(0.0, 0.49) * total).min(total);
            let mut acc = 0.0;
            for s in &sorted {
                acc += s.weight;
                if acc >= target {
                    return s.temp_f;
                }
            }
            sorted[sorted.len() - 1].temp_f
        };
        let p_lo = pick(self.clip_lo);
        // Upper percentile: scan from the hot end with the same rule.
        let hi_target = (self.clip_hi.clamp(0.0, 0.49) * total).min(total);
        let mut acc = 0.0;
        let mut p_hi = sorted[0].temp_f;
        for s in sorted.iter().rev() {
            acc += s.weight;
            p_hi = s.temp_f;
            if acc >= hi_target {
                break;
            }
        }
        let raw_lo = sorted[0].temp_f;
        let raw_hi = sorted[sorted.len() - 1].temp_f;
        let lo = p_lo - self.cold_headroom.clamp(0.0, 1.0) * (p_lo - raw_lo).max(0.0);
        let hi = p_hi + self.hot_headroom.clamp(0.0, 1.0) * (raw_hi - p_hi).max(0.0);
        self.ease_to(lo.min(hi), hi.max(lo), dt);
    }

    /// Feed the frame's observed min/max scene temps; advances the window.
    pub fn update(&mut self, frame_lo: f32, frame_hi: f32, dt: f32) {
        self.ease_to(frame_lo, frame_hi, dt)
    }

    /// Clamp a target window to `min_span_f` and ease the live window toward
    /// it. Shared by both feed paths so response and flatness are identical.
    fn ease_to(&mut self, frame_lo: f32, frame_hi: f32, dt: f32) {
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

    fn settle(agc: &mut Agc, samples: &[TempSample]) {
        for _ in 0..200 {
            agc.update_weighted(samples, 0.1);
        }
    }

    #[test]
    fn percentile_window_rejects_a_tiny_cold_sky_sliver() {
        // A narrow ground scene: 50..58 °F over the whole frame.
        let ground: Vec<TempSample> = (0..9)
            .map(|i| TempSample {
                temp_f: 50.0 + i as f32,
                weight: 0.11,
            })
            .collect();
        let mut a = Agc::new();
        settle(&mut a, &ground);
        let contrast = a.normalize(58.0) - a.normalize(50.0);

        // Now a single sliver of very cold sky enters frame (0.4% coverage).
        let mut with_sky = ground.clone();
        with_sky.push(TempSample {
            temp_f: -40.0,
            weight: 0.004,
        });
        let mut b = Agc::new();
        settle(&mut b, &with_sky);
        let contrast_sky = b.normalize(58.0) - b.normalize(50.0);
        assert!(
            contrast_sky > contrast * 0.9,
            "cold sliver collapsed ground contrast: {contrast_sky} vs {contrast}"
        );

        // The min/max window it replaces does exactly the thing we reject.
        let mut naive = Agc::new();
        for _ in 0..200 {
            naive.update(-40.0, 58.0, 0.1);
        }
        let naive_contrast = naive.normalize(58.0) - naive.normalize(50.0);
        assert!(
            naive_contrast < contrast_sky * 0.5,
            "min/max should be the compressed one: {naive_contrast}"
        );
    }

    #[test]
    fn percentile_window_keeps_hot_targets_saturated() {
        // Ground + sky + one small blazing pest, as in the reference wide shot.
        let samples = vec![
            TempSample { temp_f: 5.0, weight: 0.42 },
            TempSample { temp_f: 45.0, weight: 0.55 },
            TempSample { temp_f: 48.0, weight: 0.02 },
            TempSample { temp_f: 101.0, weight: 0.005 },
        ];
        let mut a = Agc::new();
        settle(&mut a, &samples);
        assert!(a.normalize(101.0) > 0.98, "pest must saturate");
        let ground = a.normalize(45.0);
        assert!(
            (0.3..0.8).contains(&ground),
            "ground must stay mid-gray, got {ground}"
        );
        assert!(a.normalize(5.0) < ground, "sky is the cold floor");
    }

    #[test]
    fn weighted_update_still_clamps_min_span_and_ignores_empty() {
        let mut a = Agc::new();
        settle(
            &mut a,
            &[
                TempSample { temp_f: 50.0, weight: 1.0 },
                TempSample { temp_f: 52.0, weight: 1.0 },
            ],
        );
        assert!((a.hi_f - a.lo_f) >= a.min_span_f * 0.99);
        let before = (a.lo_f, a.hi_f);
        a.update_weighted(&[], 0.1);
        a.update_weighted(&[TempSample { temp_f: 9.0, weight: 0.0 }], 0.1);
        assert_eq!(before, (a.lo_f, a.hi_f));
    }

    #[test]
    fn lut_matches_map() {
        let lut = ThermalPalette::WhiteHot.lut();
        assert_eq!(lut[0], [0, 0, 0]);
        assert_eq!(lut[255], [255, 255, 255]);
    }
}
