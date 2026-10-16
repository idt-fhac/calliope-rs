//! numpy's legacy `RandomState` (MT19937), as far as SPORES `random` scoring needs it.
//!
//! `np.random.seed(s)` seeds MT19937 with `init_genrand(s)`; `np.random.choice([a, b], n)`
//! draws `randint(0, 2, n)`, which for a range of 2 takes one 32-bit output per draw and
//! masks its lowest bit. With the same seed the port therefore draws the same scores as
//! Calliope.

/// The Mersenne Twister MT19937 (32-bit), numpy's legacy generator.
#[derive(Debug, Clone)]
pub struct Mt19937 {
    mt: [u32; 624],
    idx: usize,
}

impl Mt19937 {
    /// `init_genrand(seed)` (`np.random.seed(seed)` for a 32-bit integer seed).
    pub fn new(seed: u32) -> Self {
        let mut mt = [0u32; 624];
        mt[0] = seed;
        for i in 1..624 {
            let prev = mt[i - 1];
            mt[i] = 1_812_433_253u32
                .wrapping_mul(prev ^ (prev >> 30))
                .wrapping_add(u32::try_from(i).expect("< 624"));
        }
        Self { mt, idx: 624 }
    }

    fn twist(&mut self) {
        for i in 0..624 {
            let y = (self.mt[i] & 0x8000_0000) | (self.mt[(i + 1) % 624] & 0x7fff_ffff);
            let mut v = self.mt[(i + 397) % 624] ^ (y >> 1);
            if y & 1 != 0 {
                v ^= 0x9908_b0df;
            }
            self.mt[i] = v;
        }
        self.idx = 0;
    }

    /// `genrand_int32`.
    pub fn next_u32(&mut self) -> u32 {
        if self.idx >= 624 {
            self.twist();
        }
        let mut y = self.mt[self.idx];
        self.idx += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// `np.random.choice([a, b])`.
    pub fn choice2(&mut self, a: f64, b: f64) -> f64 {
        if self.next_u32() & 1 == 0 { a } else { b }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_numpy() {
        // np.random.seed(0); np.random.randint(0, 2**32, 3, dtype=np.uint32)
        let mut r = Mt19937::new(0);
        assert_eq!(
            [r.next_u32(), r.next_u32(), r.next_u32()],
            [2_357_136_044, 2_546_248_239, 3_071_714_933]
        );
        // np.random.seed(0); np.random.choice([0, 1000], size=8)
        let mut r = Mt19937::new(0);
        let v: Vec<f64> = (0..8).map(|_| r.choice2(0.0, 1000.0)).collect();
        assert_eq!(
            v,
            [0.0, 1000.0, 1000.0, 0.0, 1000.0, 1000.0, 1000.0, 1000.0]
        );
    }
}
