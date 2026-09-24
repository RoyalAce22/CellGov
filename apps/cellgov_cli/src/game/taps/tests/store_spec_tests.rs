//! The store watch's spec, read from its two variables.

use super::parse_store;
use crate::game::taps::error::TapError;

#[test]
fn the_window_parses_and_its_length_is_bounded() {
    let spec = parse_store(Some("0x91fe90:0x20"), Some("w.bin"))
        .unwrap()
        .unwrap();
    assert_eq!((spec.addr, spec.len), (0x91_FE90, 0x20));
    assert_eq!(parse_store(None, Some("")).unwrap(), None);
    for bad in ["10:0", "10:10001"] {
        assert!(
            matches!(
                parse_store(Some(bad), Some("w")),
                Err(TapError::OutOfRange { .. })
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        parse_store(Some("10"), Some("w")),
        Err(TapError::BadShape { .. })
    ));
    assert!(matches!(
        parse_store(Some("10:4"), None),
        Err(TapError::Unpaired { .. })
    ));
}
#[test]
fn a_window_past_4_gib_is_refused_not_truncated() {
    let last = parse_store(Some("ffff0000:10000"), Some("w"))
        .unwrap()
        .unwrap();
    assert_eq!((last.addr, last.len), (0xFFFF_0000, 0x1_0000));
    for bad in ["ffff0001:10000", "100000000:4", "ffffffffffffffff:1"] {
        assert!(
            matches!(
                parse_store(Some(bad), Some("w")),
                Err(TapError::OutOfRange { .. })
            ),
            "{bad}"
        );
    }
    assert!(matches!(
        parse_store(Some("100000000:4"), Some("w")),
        Err(TapError::OutOfRange {
            value: 0x1_0000_0000,
            ..
        })
    ));
}
