//! The loose-form grammar's edges: what a line the grammar cannot
//! place turns into, and what a keyfile block with a hole in it turns
//! into.

use std::path::Path;

use crate::keys::{IgnoreReason, KeyVault, KeyVaultError, SelfClass};

fn h(byte: u8, len: usize) -> String {
    format!("{byte:02x}").repeat(len)
}

fn parse_text(text: &str) -> Result<KeyVault, KeyVaultError> {
    KeyVault::parse(Path::new("keys.txt"), text.as_bytes())
}

fn parse_toml(text: &str) -> Result<KeyVault, KeyVaultError> {
    KeyVault::parse(Path::new("keys.toml"), text.as_bytes())
}

fn reasons(v: &KeyVault) -> Vec<IgnoreReason> {
    v.ignored().iter().map(|i| i.reason.clone()).collect()
}

#[test]
fn a_block_property_given_twice_with_different_values_is_a_conflict_naming_both_lines() {
    let text = format!(
        "[app-3.55]\nself_type=APP\nrevision=0A\nerk={}\nriv={}\nerk={}\n",
        h(0xB1, 32),
        h(0xB2, 16),
        h(0xB3, 32)
    );
    let err = parse_text(&text).unwrap_err();
    let KeyVaultError::Conflict {
        what,
        first,
        second,
    } = &err
    else {
        panic!("expected Conflict, got {err}");
    };
    assert_eq!(what, "[app-3.55] erk");
    assert_eq!(first.line_number(), Some(4));
    assert_eq!(second.line_number(), Some(6));
}

#[test]
fn a_block_property_repeated_with_the_same_value_in_another_case_is_not_a_conflict() {
    let text = format!(
        "[app-3.55]\nself_type=APP\nrevision=0A\nerk={}\nriv={}\nERK={}\n",
        h(0xB1, 32),
        h(0xB2, 16),
        h(0xB1, 32).to_ascii_uppercase()
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.self_key(SelfClass::App, 0x0A).unwrap().erk, [0xB1u8; 32]);
}

#[test]
fn a_self_block_missing_one_half_is_set_aside_as_an_unpaired_half_not_an_unused_keyset() {
    let erk_only = format!(
        "[app-3.55]\nself_type=APP\nrevision=0A\nerk={}\n",
        h(0xB1, 32)
    );
    let v = parse_text(&erk_only).unwrap();
    assert_eq!(v.unlabeled_count(SelfClass::App), 0);
    assert!(v.labeled_revisions(SelfClass::App).next().is_none());
    assert_eq!(
        reasons(&v),
        vec![IgnoreReason::UnpairedHalf {
            what: "[app-3.55]".to_string(),
            present: "erk",
            missing: "riv",
        }]
    );

    let riv_only = format!("[npdrm-3.55]\nself_type=NPDRM\nriv={}\n", h(0xC2, 16));
    let v = parse_text(&riv_only).unwrap();
    assert_eq!(
        reasons(&v),
        vec![IgnoreReason::UnpairedHalf {
            what: "[npdrm-3.55]".to_string(),
            present: "riv",
            missing: "erk",
        }]
    );
}

#[test]
fn a_type_other_block_is_a_loose_key_never_a_keyset_filed_by_its_name() {
    let text = format!(
        "[NP_something]\ntype=OTHER\nerk={}\nriv={}\n\n[NP_tid]\ntype=OTHER\nkey={}\n",
        h(0xC1, 32),
        h(0xC2, 16),
        h(0x45, 16)
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.unlabeled_count(SelfClass::Npdrm), 0);
    assert!(v.self_key_candidates(SelfClass::Npdrm, 0).next().is_none());
    assert_eq!(
        reasons(&v),
        vec![
            IgnoreReason::UnusedKeyset {
                name: "NP_something".to_string()
            },
            IgnoreReason::UnusedKeyset {
                name: "NP_tid".to_string()
            },
        ]
    );
}

#[test]
fn a_scetool_debug_keyset_is_set_aside_and_the_rest_of_the_file_still_loads() {
    for debug in ["8000", "0x8000", "800A"] {
        let text = format!(
            "[app-debug]\nself_type=APP\nrevision={debug}\nerk={}\nriv={}\n\n\
             [app-3.55]\nself_type=APP\nrevision=0A\nerk={}\nriv={}\n",
            h(0xD1, 32),
            h(0xD2, 16),
            h(0xB1, 32),
            h(0xB2, 16)
        );
        let v = parse_text(&text).unwrap_or_else(|e| panic!("{debug}: {e}"));
        assert_eq!(v.self_key(SelfClass::App, 0x0A).unwrap().erk, [0xB1u8; 32]);
        assert_eq!(v.unlabeled_count(SelfClass::App), 0, "{debug}");
        assert_eq!(
            reasons(&v),
            vec![IgnoreReason::UnusedKeyset {
                name: "app-debug".to_string()
            }],
            "{debug}"
        );
    }
    let bad = format!(
        "[app-x]\nself_type=APP\nrevision=zz\nerk={}\nriv={}\n",
        h(0xB1, 32),
        h(0xB2, 16)
    );
    let err = parse_text(&bad).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::BadRevision { ref value, .. } if value == "zz"),
        "{err}"
    );
}

#[test]
fn a_table_row_with_no_type_column_reports_a_redacted_first_column_not_the_erk() {
    let text = format!(
        "{} {} {} {} 0x33\n",
        h(0xB1, 32),
        h(0xB2, 16),
        h(0xEE, 40),
        h(0xEF, 21)
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(
        reasons(&v),
        vec![IgnoreReason::UnusedKeyset {
            name: "<hex>".to_string()
        }]
    );
    let rendered: Vec<String> = v.ignored().iter().map(|i| i.reason.to_string()).collect();
    assert!(!rendered[0].contains("b1b1"), "{rendered:?}");
}

#[test]
fn a_named_half_that_swallowed_a_neighbouring_value_reports_a_redacted_name() {
    let text = format!(
        "ss::Kf1_u0: {} npdrm_key_0a: {}\n",
        h(0x21, 16),
        h(0xC1, 32)
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.ignored().len(), 1, "{:?}", v.ignored());
    let reason = v.ignored()[0].reason.to_string();
    assert!(
        matches!(&v.ignored()[0].reason, IgnoreReason::UnpairedHalf { .. }),
        "{reason}"
    );
    assert!(reason.contains("<hex>"), "{reason}");
    assert!(!reason.contains("2121"), "{reason}");
}

#[test]
fn a_named_value_written_as_colon_separated_bytes_is_read_from_the_first_separator() {
    let colons: Vec<String> = (0..16).map(|_| "33".to_string()).collect();
    let text = format!("klic_key: {}\n", colons.join(":"));
    let v = parse_text(&text).unwrap();
    assert_eq!(v.np_klic_key().unwrap(), &[0x33u8; 16]);
    assert!(v.ignored().is_empty(), "{:?}", v.ignored());
}

#[test]
fn a_table_row_reads_its_revision_only_from_before_the_erk() {
    // No revision column: the `0x33` curve type after the RIV must not
    // label the row.
    let text = format!(
        "app 3.55 {} {} {} {} 0x33\n",
        h(0xB1, 32),
        h(0xB2, 16),
        h(0xEE, 40),
        h(0xEF, 21)
    );
    let v = parse_text(&text).unwrap();
    assert!(v.labeled_revisions(SelfClass::App).next().is_none());
    assert_eq!(v.unlabeled_count(SelfClass::App), 1);
    assert!(v.ignored().is_empty(), "{:?}", v.ignored());
}

#[test]
fn a_line_the_grammar_cannot_place_that_carries_a_key_sized_token_is_set_aside_not_read_as_a_heading(
) {
    // The RIV before the ERK is no table row; the bare value beneath
    // must not inherit the row as its heading either.
    let text = format!(
        "app 3.55 0x0A {} {}\n{}\n",
        h(0xB2, 16),
        h(0xB1, 32),
        h(0x11, 64)
    );
    let v = parse_text(&text).unwrap();
    assert!(v.labeled_revisions(SelfClass::App).next().is_none());
    assert_eq!(v.unlabeled_count(SelfClass::App), 0);
    let got = reasons(&v);
    assert_eq!(
        got,
        vec![
            IgnoreReason::UnnamedValue { bytes: 32 },
            IgnoreReason::UnnamedValue { bytes: 64 },
        ],
        "{got:?}"
    );
    assert_eq!(v.ignored()[0].at.line_number(), Some(1));
    assert_eq!(v.ignored()[1].at.line_number(), Some(2));
}

#[test]
fn a_prose_line_without_a_key_sized_token_is_still_a_heading_for_the_value_beneath() {
    let text = format!("PS3 PUP HMAC\n{}\n", h(0x11, 64));
    let v = parse_text(&text).unwrap();
    assert_eq!(v.pup_hmac().unwrap(), &[0x11u8; 64]);
    assert!(v.ignored().is_empty());
}

#[test]
fn an_unknown_field_inside_a_keyset_table_is_refused_rather_than_filing_an_unlabeled_candidate() {
    let text = format!(
        "[[app]]\nrevison = 0x0a\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xB1, 32),
        h(0xB2, 16)
    );
    let err = parse_toml(&text).unwrap_err();
    assert!(matches!(err, KeyVaultError::Toml { .. }), "{err}");
    assert!(err.to_string().contains("revison"), "{err}");
}

#[test]
fn a_known_toml_field_with_a_value_of_the_wrong_shape_is_refused_as_that_not_as_an_unknown_name() {
    let out_of_range = format!("np_klic_free = [0x44, 300, {}]\n", ["0x44"; 14].join(", "));
    let err = parse_toml(&out_of_range).unwrap_err();
    assert!(matches!(err, KeyVaultError::Toml { .. }), "{err}");
    let rendered = err.to_string();
    assert!(rendered.contains("np_klic_free"), "{rendered}");
    assert!(rendered.contains("element 1"), "{rendered}");
    assert!(rendered.contains("0..=255"), "{rendered}");

    let err = parse_toml("pkg_aes = 12345\n").unwrap_err();
    assert!(matches!(err, KeyVaultError::Toml { .. }), "{err}");
    assert!(
        err.to_string().contains("hex string or byte array"),
        "{err}"
    );

    let err = parse_toml("[frobnicate]\nx = 1\n").unwrap_err();
    assert!(
        matches!(err, KeyVaultError::UnknownName { ref name, .. } if name == "frobnicate"),
        "{err}"
    );
}
