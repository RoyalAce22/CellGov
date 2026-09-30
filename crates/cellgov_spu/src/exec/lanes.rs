//! A 128-bit register as its eight halfwords, four words or two
//! doublewords, slot 0 the
//! leftmost, each big-endian.

// [SPU-ISA p:16 s:Preface] bytes are numbered in ascending order from left to right, big-endian.
// [SPU-ISA p:26 s:2.1] a halfword spans bytes 0:1 and a word bytes 0:3, the most significant byte first.

/// The eight halfwords of `reg`.
pub(super) fn halfwords(reg: [u8; 16]) -> [u16; 8] {
    std::array::from_fn(|i| u16::from_be_bytes([reg[2 * i], reg[2 * i + 1]]))
}

/// The register whose halfwords are `h`.
pub(super) fn from_halfwords(h: [u16; 8]) -> [u8; 16] {
    std::array::from_fn(|i| h[i / 2].to_be_bytes()[i % 2])
}

/// The four words of `reg`.
pub(super) fn words(reg: [u8; 16]) -> [u32; 4] {
    std::array::from_fn(|i| {
        u32::from_be_bytes([reg[4 * i], reg[4 * i + 1], reg[4 * i + 2], reg[4 * i + 3]])
    })
}

/// The register whose words are `w`.
pub(super) fn from_words(w: [u32; 4]) -> [u8; 16] {
    std::array::from_fn(|i| w[i / 4].to_be_bytes()[i % 4])
}

/// The two doublewords of `reg`.
pub(super) fn doublewords(reg: [u8; 16]) -> [u64; 2] {
    std::array::from_fn(|i| u64::from_be_bytes(std::array::from_fn(|byte| reg[8 * i + byte])))
}

/// The register whose doublewords are `d`.
pub(super) fn from_doublewords(d: [u64; 2]) -> [u8; 16] {
    std::array::from_fn(|i| d[i / 8].to_be_bytes()[i % 8])
}

#[cfg(test)]
#[path = "tests/lanes_tests.rs"]
mod tests;
