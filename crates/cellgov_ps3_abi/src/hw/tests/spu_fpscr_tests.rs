//! The FPSCR's defined bits are exactly the fields the ISA lists.

use super::*;

/// [SPU-ISA p:200 s:9.3] and [SPU-ISA p:201 s:9.3]: 4 control bits, 4 x 3
/// single-precision flags, 2 x 6 double-precision flags and 4 DBZ flags.
#[test]
fn the_defined_mask_is_the_listed_fields() {
    assert_eq!(FPSCR_DEFINED.count_ones(), 4 + 12 + 12 + 4);
    // Bit 0 is the most significant; bit 127 the least.
    assert_eq!(fpscr_field(127, 1), 1);
    assert_eq!(fpscr_field(0, 1), 1 << 127);
    for bit in [
        20, 23, 29, 31, 50, 55, 61, 63, 82, 87, 93, 95, 116, 119, 125, 127,
    ] {
        assert_ne!(FPSCR_DEFINED & fpscr_field(bit, 1), 0, "bit {bit}");
    }
    for bit in [
        0, 19, 24, 28, 32, 49, 56, 60, 64, 81, 88, 92, 96, 115, 120, 124,
    ] {
        assert_eq!(FPSCR_DEFINED & fpscr_field(bit, 1), 0, "bit {bit}");
    }
}
