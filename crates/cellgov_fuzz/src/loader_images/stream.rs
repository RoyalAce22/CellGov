//! The fuzz byte stream the image decoders read fields from.

/// Fixed-width fields read from a fuzz byte stream; an exhausted stream reads as zero.
#[derive(Debug, Clone)]
pub struct FieldStream<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> FieldStream<'a> {
    /// Reads `data` from its first byte.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn take(&mut self, n: usize) -> [u8; 8] {
        let mut out = [0u8; 8];
        for slot in out.iter_mut().take(n) {
            if let Some(&b) = self.data.get(self.pos) {
                *slot = b;
                self.pos += 1;
            }
        }
        out
    }

    /// The next byte.
    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }

    /// The next two bytes, little-endian.
    pub fn u16(&mut self) -> u16 {
        let b = self.take(2);
        u16::from_le_bytes([b[0], b[1]])
    }

    /// The next four bytes, little-endian.
    pub fn u32(&mut self) -> u32 {
        let b = self.take(4);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }

    /// The next eight bytes, little-endian.
    pub fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8))
    }

    /// A value below `n`; zero when `n` is zero.
    pub fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.u32() % n
        }
    }

    /// The next `len` bytes.
    pub fn bytes(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| self.u8()).collect()
    }

    /// A value from the set a byte-level fuzzer substitutes as interesting.
    /// [Padhye2019 p:331 s:2.2 Coverage-Guided Fuzzing]
    pub fn interesting_u64(&mut self) -> u64 {
        match self.below(8) {
            0 => 0,
            1 => u64::MAX,
            2 => 1 << 63,
            3 => i64::MAX as u64,
            4 => 1 << 32,
            5 => u64::from(u32::MAX),
            6 => u64::from(self.u8()),
            _ => 1u64 << (self.u8() % 64),
        }
    }

    /// True when no unread byte remains.
    pub fn is_exhausted(&self) -> bool {
        self.pos >= self.data.len()
    }
}

#[cfg(test)]
#[path = "tests/stream_tests.rs"]
mod tests;
