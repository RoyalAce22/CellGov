//! Deterministic SplitMix64 generator for reproducible fuzz cases.

#[derive(Clone, Copy, Debug)]
pub struct Rng(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WordGenerationFailure {
    DecoderPanic(u32),
    Exhausted(u32),
}

const WORD_GENERATION_ATTEMPTS: usize = 64;

impl Rng {
    pub fn for_iter(seed: u64, iter: u64) -> Self {
        let mut r = Self(seed ^ iter.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0, "Rng::below(0)");
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }

    /// Draws values that emphasize arithmetic edge cases.
    pub fn interesting_u64(&mut self) -> u64 {
        match self.below(8) {
            0 => 0,
            1 => u64::MAX,
            2 => 0x8000_0000_0000_0000,
            3 => 0x7FFF_FFFF_FFFF_FFFF,
            4 => 0x0000_0000_8000_0000,
            5 => 0x0000_0000_7FFF_FFFF,
            6 => self.below(16),
            _ => 1u64 << self.below(64),
        }
    }

    pub fn mixed_u64(&mut self) -> u64 {
        if self.chance(1, 2) {
            self.next_u64()
        } else {
            self.interesting_u64()
        }
    }

    /// Draws IEEE-754 double bit patterns, including special values.
    pub fn fp_bits(&mut self) -> u64 {
        match self.below(8) {
            0 => 0,
            1 => 0x8000_0000_0000_0000,
            2 => f64::INFINITY.to_bits(),
            3 => f64::NEG_INFINITY.to_bits(),
            4 => 0x7FF8_0000_0000_0000 | self.below(1 << 20), // quiet NaN
            5 => 0x7FF0_0000_0000_0001 | self.below(1 << 20), // signalling NaN
            6 => self.below(1 << 52),                         // denormal
            _ => self.next_u64(),
        }
    }

    pub fn fill(&mut self, buf: &mut [u8]) {
        for chunk in buf.chunks_mut(8) {
            let v = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }

    /// Limits candidate generation to preserve deterministic case size.
    pub(crate) fn decoder_accepted_words(
        &mut self,
        count: usize,
        mut decode: impl FnMut(u32) -> Result<bool, ()>,
    ) -> Result<Vec<u32>, WordGenerationFailure> {
        let mut words = Vec::with_capacity(count);
        for _ in 0..count {
            let mut last_raw = 0;
            let mut accepted = None;
            for _ in 0..WORD_GENERATION_ATTEMPTS {
                let raw = self.next_u32();
                last_raw = raw;
                match decode(raw) {
                    Ok(true) => {
                        accepted = Some(raw);
                        break;
                    }
                    Ok(false) => {}
                    Err(()) => return Err(WordGenerationFailure::DecoderPanic(raw)),
                }
            }
            let Some(raw) = accepted else {
                return Err(WordGenerationFailure::Exhausted(last_raw));
            };
            words.push(raw);
        }
        Ok(words)
    }
}

#[cfg(test)]
#[path = "tests/rng_tests.rs"]
mod tests;
