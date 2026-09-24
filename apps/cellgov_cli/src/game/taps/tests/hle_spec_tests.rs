//! The HLE return watch's spec, read from its three variables.

use super::parse_hle;
use crate::game::taps::error::TapError;

#[test]
fn nothing_set_is_no_watch() {
    assert_eq!(parse_hle(None, None, None).unwrap(), None);
    assert_eq!(parse_hle(Some(""), Some(" , "), Some("")).unwrap(), None);
}

#[test]
fn nids_and_raw_pcs_parse_with_or_without_the_hex_prefix() {
    let spec = parse_hle(
        Some("0xE6F2C1E7, 9a0e0d6e"),
        Some("10010=entry_a,0X10020=entry_b"),
        Some("watch.bin"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(spec.nids, vec![0xE6F2_C1E7, 0x9A0E_0D6E]);
    assert_eq!(
        spec.raw_pcs,
        vec![
            (0x10010, "entry_a".to_string()),
            (0x10020, "entry_b".to_string())
        ]
    );
}

#[test]
fn a_watch_and_a_path_without_each_other_are_refused() {
    assert!(matches!(
        parse_hle(Some("1234"), None, None),
        Err(TapError::Unpaired { .. })
    ));
    assert!(matches!(
        parse_hle(None, None, Some("watch.bin")),
        Err(TapError::Unpaired { .. })
    ));
}

#[test]
fn a_malformed_token_names_its_variable() {
    let err = parse_hle(Some("zz"), None, Some("w")).unwrap_err();
    assert!(
        err.to_string().starts_with("CELLGOV_HLE_RETURN_WATCH:"),
        "{err}"
    );
    let err = parse_hle(None, Some("10010"), Some("w")).unwrap_err();
    assert!(matches!(err, TapError::BadShape { .. }), "{err}");
    let err = parse_hle(Some("100000000"), None, Some("w")).unwrap_err();
    assert!(matches!(err, TapError::OutOfRange { .. }), "{err}");
}

#[test]
fn a_raw_pc_whose_on_wire_id_is_a_watched_nid_is_refused() {
    let err = parse_hle(Some("80010010"), Some("10010=f"), Some("w")).unwrap_err();
    assert!(matches!(
        err,
        TapError::RawPcCollides {
            pc: 0x10010,
            id: 0x8001_0010
        }
    ));
}

#[test]
fn raw_pcs_sharing_an_on_wire_id_are_refused() {
    for pcs in ["10010=f,10010=g", "10010=f,80010010=g"] {
        let err = parse_hle(None, Some(pcs), Some("w")).unwrap_err();
        assert!(
            matches!(
                err,
                TapError::RawPcsCollide {
                    first: 0x10010,
                    id: 0x8001_0010,
                    ..
                }
            ),
            "{pcs}: {err}"
        );
    }
}

#[test]
fn a_raw_pc_name_the_resolution_record_cannot_carry_is_refused() {
    let fits = format!("10010={}", "n".repeat(255));
    assert!(parse_hle(None, Some(&fits), Some("w")).is_ok());
    let over = format!("10010={}", "n".repeat(256));
    assert!(matches!(
        parse_hle(None, Some(&over), Some("w")),
        Err(TapError::BadShape { .. })
    ));
}
