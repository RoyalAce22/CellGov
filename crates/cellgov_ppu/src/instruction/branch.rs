//! Where a relative or absolute branch goes.

/// The target of a branch at `cia` whose displacement is `offset`.
///
/// `aa` takes the sign-extended displacement as the absolute address;
/// otherwise it is added to the branch's own address, wrapping. An
/// I-form `b` carries a 26-bit and a B-form `bc` a 16-bit displacement,
/// both already shifted left by two, so the low two bits are zero.
// [PPC-Book1 p:24 s:2.4] Branch I-form: NIA <- EXTS(LI||0b00) when AA, else CIA+EXTS(LI||0b00).
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

#[cfg(test)]
#[path = "tests/branch_tests.rs"]
mod tests;
