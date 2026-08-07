//! PARAM.SFO parsing: value-format decode, accessor typing, and
//! structural-corruption rejection.

use super::*;

/// A value to emit into a synthetic PARAM.SFO.
enum BuildVal {
    Int(u32),
    Str(&'static str),
    Arr(Vec<u8>),
}

/// Emit a well-formed PARAM.SFO image from `entries`, in the given
/// order. String entries are stored NUL-terminated; the key table is
/// 4-byte aligned.
fn build_sfo(entries: &[(&str, BuildVal)]) -> Vec<u8> {
    let mut key_table = Vec::new();
    let mut key_offs = Vec::new();
    for (k, _) in entries {
        key_offs.push(key_table.len() as u16);
        key_table.extend_from_slice(k.as_bytes());
        key_table.push(0);
    }
    while key_table.len() % 4 != 0 {
        key_table.push(0);
    }

    let mut data_table = Vec::new();
    // (key_off, fmt, param_len, param_max, data_off)
    let mut recs: Vec<(u16, u16, u32, u32, u32)> = Vec::new();
    for (i, (_, v)) in entries.iter().enumerate() {
        let data_off = data_table.len() as u32;
        let (fmt, len, max, bytes) = match v {
            BuildVal::Int(n) => (FMT_INTEGER, 4u32, 4u32, n.to_le_bytes().to_vec()),
            BuildVal::Str(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                let l = b.len() as u32;
                (FMT_STRING, l, l, b)
            }
            BuildVal::Arr(a) => {
                let l = a.len() as u32;
                (FMT_ARRAY, l, l, a.clone())
            }
        };
        data_table.extend_from_slice(&bytes);
        recs.push((key_offs[i], fmt, len, max, data_off));
    }

    let n = entries.len();
    let off_key_table = (HEADER_LEN + n * INDEX_LEN) as u32;
    let off_data_table = off_key_table + key_table.len() as u32;

    let mut buf = Vec::new();
    buf.extend_from_slice(&SFO_MAGIC);
    buf.extend_from_slice(&SFO_VERSION.to_le_bytes());
    buf.extend_from_slice(&off_key_table.to_le_bytes());
    buf.extend_from_slice(&off_data_table.to_le_bytes());
    buf.extend_from_slice(&(n as u32).to_le_bytes());
    for (key_off, fmt, len, max, data_off) in &recs {
        buf.extend_from_slice(&key_off.to_le_bytes());
        buf.extend_from_slice(&fmt.to_le_bytes());
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&max.to_le_bytes());
        buf.extend_from_slice(&data_off.to_le_bytes());
    }
    buf.extend_from_slice(&key_table);
    buf.extend_from_slice(&data_table);
    buf
}

/// Byte offset of field `field` within entry `i`'s index record.
fn rec_field(i: usize, field: usize) -> usize {
    HEADER_LEN + i * INDEX_LEN + field
}

#[test]
fn decodes_each_value_format() {
    let sfo = build_sfo(&[
        ("TITLE_ID", BuildVal::Str("NPUA80001")),
        ("CATEGORY", BuildVal::Str("HG")),
        ("PARENTAL_LEVEL", BuildVal::Int(1)),
        ("ACCOUNT_ID", BuildVal::Arr(vec![0xAA; 16])),
    ]);
    let p = parse(&sfo).expect("well-formed SFO parses");
    assert_eq!(p.get_string("TITLE_ID"), Some("NPUA80001"));
    assert_eq!(p.get_string("CATEGORY"), Some("HG"));
    assert_eq!(p.get_integer("PARENTAL_LEVEL"), Some(1));
    assert_eq!(p.get("ACCOUNT_ID"), Some(&SfoValue::Array(vec![0xAA; 16])));
    assert_eq!(p.len(), 4);
}

#[test]
fn accessors_are_type_checked() {
    let sfo = build_sfo(&[
        ("TITLE_ID", BuildVal::Str("NPUA80001")),
        ("PARENTAL_LEVEL", BuildVal::Int(1)),
    ]);
    let p = parse(&sfo).expect("parse");
    // Wrong-type lookups return None rather than coercing.
    assert_eq!(p.get_integer("TITLE_ID"), None);
    assert_eq!(p.get_string("PARENTAL_LEVEL"), None);
    assert_eq!(p.get_string("ABSENT"), None);
    assert_eq!(p.get_integer("ABSENT"), None);
}

#[test]
fn string_value_trims_at_first_nul() {
    // param_len covers trailing padding NULs; the decoded string stops
    // at the first NUL.
    let mut sfo = build_sfo(&[("TITLE", BuildVal::Str("flOw"))]);
    // Widen entry 0's data to include extra padding bytes after the
    // NUL by appending padding and bumping param_len/param_max.
    let extra = b"\0\0\0";
    sfo.extend_from_slice(extra);
    let new_len = (b"flOw\0".len() + extra.len()) as u32;
    sfo[rec_field(0, 0x04)..rec_field(0, 0x04) + 4].copy_from_slice(&new_len.to_le_bytes());
    sfo[rec_field(0, 0x08)..rec_field(0, 0x08) + 4].copy_from_slice(&new_len.to_le_bytes());
    let p = parse(&sfo).expect("parse");
    assert_eq!(p.get_string("TITLE"), Some("flOw"));
}

#[test]
fn rejects_too_small() {
    assert!(matches!(
        parse(&[0u8; 10]).unwrap_err(),
        SfoError::TooSmall { len: 10 }
    ));
}

#[test]
fn rejects_bad_magic() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    sfo[1] ^= 0xFF;
    assert!(matches!(parse(&sfo).unwrap_err(), SfoError::BadMagic(_)));
}

#[test]
fn rejects_bad_version() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    sfo[0x04..0x08].copy_from_slice(&0x0102u32.to_le_bytes());
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::UnsupportedVersion { found: 0x0102 }
    ));
}

#[test]
fn rejects_key_offset_out_of_range() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    sfo[rec_field(0, 0x00)..rec_field(0, 0x00) + 2].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::KeyOffsetOutOfRange { .. }
    ));
}

#[test]
fn rejects_param_len_over_max() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    sfo[rec_field(0, 0x04)..rec_field(0, 0x04) + 4].copy_from_slice(&0xFFFFu32.to_le_bytes());
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::ParamLenExceedsMax { .. }
    ));
}

#[test]
fn rejects_data_escaping_file() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    // Push data_off past the end of the file.
    sfo[rec_field(0, 0x0C)..rec_field(0, 0x0C) + 4].copy_from_slice(&0xFFFFu32.to_le_bytes());
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::DataOutOfRange { .. }
    ));
}

#[test]
fn rejects_data_table_past_eof() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    sfo[0x0C..0x10].copy_from_slice(&0xFFFFu32.to_le_bytes());
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::DataTableOutOfRange { .. }
    ));
}

#[test]
fn rejects_truncated_index_table() {
    let mut sfo = build_sfo(&[("CATEGORY", BuildVal::Str("HG"))]);
    // Claim 1000 entries; the file cannot hold that index table.
    sfo[0x10..0x14].copy_from_slice(&1000u32.to_le_bytes());
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::IndexTruncated { .. }
    ));
}

#[test]
fn rejects_duplicate_key() {
    let sfo = build_sfo(&[
        ("CATEGORY", BuildVal::Str("HG")),
        ("CATEGORY", BuildVal::Str("GD")),
    ]);
    assert!(matches!(
        parse(&sfo).unwrap_err(),
        SfoError::DuplicateKey(k) if k == "CATEGORY"
    ));
}

#[test]
fn skips_unknown_format_entry() {
    let sfo_ref = build_sfo(&[
        ("CATEGORY", BuildVal::Str("HG")),
        ("TITLE_ID", BuildVal::Str("NPUA80001")),
    ]);
    let mut sfo = sfo_ref.clone();
    // Rewrite entry 0's format tag to an unknown value; it is dropped.
    sfo[rec_field(0, 0x02)..rec_field(0, 0x02) + 2].copy_from_slice(&0x9999u16.to_le_bytes());
    let p = parse(&sfo).expect("unknown format is skipped, not fatal");
    assert_eq!(p.get_string("CATEGORY"), None);
    assert_eq!(p.get_string("TITLE_ID"), Some("NPUA80001"));
    assert_eq!(p.len(), 1);
}

#[test]
fn parses_zero_entry_table() {
    let sfo = build_sfo(&[]);
    let p = parse(&sfo).expect("an empty table is structurally valid");
    assert!(p.is_empty());
}
