//! The hex shapes the watch variables accept, and the ones refused.

use super::hex_u64;
use crate::game::taps::error::TapError;

#[test]
fn one_prefix_in_either_case_is_accepted() {
    assert_eq!(hex_u64("V", "0x1f").unwrap(), 0x1F);
    assert_eq!(hex_u64("V", "0X1f").unwrap(), 0x1F);
    assert_eq!(hex_u64("V", "1f").unwrap(), 0x1F);
}

#[test]
fn a_doubled_prefix_is_refused() {
    for bad in ["0x0x10", "0X0x10", "0x0X10"] {
        assert!(
            matches!(hex_u64("V", bad), Err(TapError::BadNumber { .. })),
            "{bad}"
        );
    }
}

#[test]
fn a_signed_number_is_refused() {
    for bad in ["+10", "0x+10", "-10", "0x-10"] {
        assert!(
            matches!(hex_u64("V", bad), Err(TapError::BadShape { .. })),
            "{bad}"
        );
    }
}
