//! Where a relative or absolute branch goes.

/// The target of a branch at `cia` whose displacement is `offset`.
///
/// `aa` takes the sign-extended displacement as the absolute address;
/// otherwise it is added to the branch's own address, wrapping. An
/// I-form `b` carries a 26-bit and a B-form `bc` a 16-bit displacement,
/// both already shifted left by two, so the low two bits are zero.
///
/// [PPC-Book1 p:24 s:2.4] Branch I-form: NIA <- EXTS(LI||0b00) when AA, else CIA+EXTS(LI||0b00).
#[must_use]
pub fn branch_target(cia: u64, offset: i32, aa: bool) -> u64 {
    // `i64::from(offset) as u64` sign-extends, so a negative absolute
    // target lands at 0xFFFF_FFFF_FFFF_xxxx.
    let displacement = i64::from(offset) as u64;
    if aa {
        displacement & 0xFFFF_FFFF_FFFF_FFFC
    } else {
        cia.wrapping_add(displacement)
    }
}

/// A relative branch displacement in bytes, held in three bytes so a
/// fused pair keeps the 2-byte alignment of
/// [`FusedPair`](crate::instruction::FusedPair).
///
/// [PPC-Book1 p:24 s:2.4] the I-form displacement is EXTS(LI||0b00), with
/// LI 24 bits wide. Three bytes hold LI, so every I-form displacement fits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BranchOffset([u8; 3]);

impl BranchOffset {
    /// The displacement `bytes`, or `None` when no I-form branch encodes it.
    pub const fn new(bytes: i32) -> Option<Self> {
        if bytes & 3 != 0 || bytes < -(1 << 25) || bytes >= 1 << 25 {
            return None;
        }
        let [_, hi, mid, lo] = (bytes >> 2).to_be_bytes();
        Some(Self([hi, mid, lo]))
    }

    /// The displacement in bytes.
    pub const fn bytes(self) -> i32 {
        let [hi, mid, lo] = self.0;
        (i32::from_be_bytes([hi, mid, lo, 0]) >> 8) << 2
    }
}

#[cfg(test)]
#[path = "tests/branch_tests.rs"]
mod tests;
