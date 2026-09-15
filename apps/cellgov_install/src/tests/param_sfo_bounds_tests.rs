//! The key-offset and index-count refusals at their boundaries.

use super::*;
use crate::test_support::build_param_sfo;

// Header offsets of the fields these tests rewrite.
const OFF_KEY_TABLE_FIELD: std::ops::Range<usize> = 0x08..0x0C;
const OFF_DATA_TABLE_FIELD: std::ops::Range<usize> = 0x0C..0x10;
const ENTRIES_NUM_FIELD: std::ops::Range<usize> = 0x10..0x14;
/// Record 0's `key_off`.
const KEY_OFF_FIELD: std::ops::Range<usize> = HEADER_LEN..HEADER_LEN + 2;

fn header_offset(sfo: &[u8], field: std::ops::Range<usize>) -> usize {
    u32::from_le_bytes(sfo[field].try_into().unwrap()) as usize
}

#[test]
fn a_key_offset_at_the_key_region_end_is_out_of_range_not_unterminated() {
    let mut sfo = build_param_sfo(&[("CATEGORY", "HG")]);
    let region_len =
        header_offset(&sfo, OFF_DATA_TABLE_FIELD) - header_offset(&sfo, OFF_KEY_TABLE_FIELD);
    let key_off = u16::try_from(region_len).unwrap();
    sfo[KEY_OFF_FIELD].copy_from_slice(&key_off.to_le_bytes());
    let err = parse(&sfo).unwrap_err();
    assert!(
        matches!(
            err,
            SfoError::KeyOffsetOutOfRange { index: 0, key_off: k, key_region_len }
                if k == key_off && key_region_len == region_len
        ),
        "got {err:?}"
    );
}

#[test]
fn an_empty_key_region_holds_no_key_offset() {
    let mut sfo = build_param_sfo(&[("CATEGORY", "HG")]);
    let data_table = sfo[OFF_DATA_TABLE_FIELD].to_vec();
    sfo[OFF_KEY_TABLE_FIELD].copy_from_slice(&data_table);
    let err = parse(&sfo).unwrap_err();
    assert!(
        matches!(
            err,
            SfoError::KeyOffsetOutOfRange {
                index: 0,
                key_off: 0,
                key_region_len: 0
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn an_index_table_one_record_past_the_file_carries_the_declared_count() {
    let mut sfo = build_param_sfo(&[("CATEGORY", "HG")]);
    let len = sfo.len();
    let one_past = u32::try_from((len - HEADER_LEN) / INDEX_LEN + 1).unwrap();
    for entries in [one_past, u32::MAX] {
        sfo[ENTRIES_NUM_FIELD].copy_from_slice(&entries.to_le_bytes());
        let err = parse(&sfo).unwrap_err();
        assert!(
            matches!(
                err,
                SfoError::IndexTruncated { entries: e, len: l } if e == entries && l == len
            ),
            "entries {entries}: got {err:?}"
        );
    }
}
