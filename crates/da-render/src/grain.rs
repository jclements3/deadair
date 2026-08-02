//! Deterministic NV sensor grain (SDD §4: "gain noise").
//!
//! Real digital NV noise is heavier in dark regions (photon-starved pixels
//! get more gain). The hash is a pure function of (x, y, frame, seed) so a
//! given frame is reproducible — golden-image tests stay byte-stable — while
//! successive frames shimmer like a live sensor.

/// Cheap 2D+t hash → [0, 1). Same scheme the WGSL pass uses.
fn hash(x: u32, y: u32, frame: u32, seed: u32) -> f32 {
    let mut h = x
        .wrapping_mul(0x8DA6_B343)
        .wrapping_add(y.wrapping_mul(0xD816_3841))
        .wrapping_add(frame.wrapping_mul(0xCB1A_B31F))
        .wrapping_add(seed);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Grain offset to add to a luminance value `v` (0..1). Noise amplitude
/// scales with darkness and with `gain` (weather-degraded NV runs hotter
/// gain → grainier image, per FR-O3 fog/rain degradation).
pub fn nv_grain(v: f32, x: u32, y: u32, frame: u32, seed: u32, gain: f32) -> f32 {
    let n = hash(x, y, frame, seed) - 0.5;
    let amplitude = gain * (0.02 + 0.10 * (1.0 - v.clamp(0.0, 1.0)));
    (v + n * amplitude).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_per_frame() {
        assert_eq!(
            nv_grain(0.5, 10, 20, 3, 7, 1.0),
            nv_grain(0.5, 10, 20, 3, 7, 1.0)
        );
        assert_ne!(
            nv_grain(0.5, 10, 20, 3, 7, 1.0),
            nv_grain(0.5, 10, 20, 4, 7, 1.0)
        );
    }

    #[test]
    fn dark_regions_noisier() {
        let spread = |v: f32| {
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for f in 0..500 {
                let g = nv_grain(v, 5, 5, f, 1, 1.0) - v;
                lo = lo.min(g);
                hi = hi.max(g);
            }
            hi - lo
        };
        assert!(spread(0.1) > spread(0.9) * 1.5);
    }

    #[test]
    fn output_stays_in_unit_range() {
        for f in 0..100 {
            for &v in &[0.0, 0.05, 0.5, 0.95, 1.0] {
                let g = nv_grain(v, f, f * 3, f, 42, 2.0);
                assert!((0.0..=1.0).contains(&g));
            }
        }
    }
}
