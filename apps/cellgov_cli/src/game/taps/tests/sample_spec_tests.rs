//! The value sample's spec, read from its three variables.

use super::parse_sample;
use crate::game::taps::error::TapError;

#[test]
fn the_range_and_stride_parse_and_are_bounded() {
    let spec = parse_sample(Some("0x91FE9C:4"), Some("s.bin"), Some("16"))
        .unwrap()
        .unwrap();
    assert_eq!((spec.addr, spec.width, spec.stride), (0x91_FE9C, 4, 16));
    let default_stride = parse_sample(Some("10:100"), Some("s"), None)
        .unwrap()
        .unwrap();
    assert_eq!((default_stride.width, default_stride.stride), (256, 1));
    for (range, stride) in [("10:0", None), ("10:101", None), ("10:4", Some("0"))] {
        assert!(
            matches!(
                parse_sample(Some(range), Some("s"), stride),
                Err(TapError::OutOfRange { .. })
            ),
            "{range} {stride:?}"
        );
    }
    assert!(matches!(
        parse_sample(Some("10:4"), Some("s"), Some("x")),
        Err(TapError::BadNumber { .. })
    ));
    assert!(matches!(
        parse_sample(None, Some("s"), None),
        Err(TapError::Unpaired { .. })
    ));
}
#[test]
fn a_stride_with_no_range_or_path_is_refused() {
    assert!(matches!(
        parse_sample(None, None, Some("16")),
        Err(TapError::Unpaired { .. })
    ));
    assert_eq!(parse_sample(None, None, Some(" ")).unwrap(), None);
}
#[test]
fn an_address_past_32_bits_is_refused_not_truncated() {
    let last = parse_sample(Some("ffffffff:1"), Some("s"), None)
        .unwrap()
        .unwrap();
    assert_eq!(last.addr, 0xFFFF_FFFF);
    assert!(matches!(
        parse_sample(Some("100000000:4"), Some("s"), None),
        Err(TapError::OutOfRange {
            value: 0x1_0000_0000,
            ..
        })
    ));
}
