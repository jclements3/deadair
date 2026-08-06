//! Deterministic PRNG (xoshiro256**). No external dependency so every crate
//! shares one reproducible source of randomness — parametric scene expansion
//! and AI must replay identically from the same seed.

#[derive(Debug, Clone)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// Seed via splitmix64 so nearby seeds diverge immediately.
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        let mut next = || {
            sm = sm.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = sm;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        Self {
            s: [next(), next(), next(), next()],
        }
    }

    /// Derive an independent stream (e.g. per entity) from this one.
    pub fn fork(&mut self, salt: u64) -> Rng {
        Rng::new(self.next_u64() ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        let r = self.s[1]
            .wrapping_mul(5)
            .rotate_left(7)
            .wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 * (1.0 / (1u64 << 24) as f32)
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.f32() * (hi - lo)
    }

    /// Uniform integer in [0, n). n must be > 0.
    pub fn below(&mut self, n: u64) -> u64 {
        // Multiply-shift; bias is negligible for game-scale n.
        ((self.next_u64() as u128 * n as u128) >> 64) as u64
    }

    /// True with probability p.
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn f32_in_unit_range() {
        let mut r = Rng::new(7);
        for _ in 0..10_000 {
            let x = r.f32();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn below_in_range_and_covers() {
        let mut r = Rng::new(9);
        let mut seen = [false; 6];
        for _ in 0..1000 {
            let v = r.below(6) as usize;
            assert!(v < 6);
            seen[v] = true;
        }
        assert!(seen.iter().all(|&s| s));
    }
}
