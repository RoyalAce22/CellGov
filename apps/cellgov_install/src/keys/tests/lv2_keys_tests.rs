//! The LV2 keyset class: every keyfile shape that files one, the
//! version-range labels, and the candidate order a kernel's version
//! selects.

use std::path::Path;

use cellgov_ps3_abi::format::sce::self_version;

use super::hex::hex;
use super::*;

const V3_55: u64 = self_version(3, 0x55);
const V3_60: u64 = self_version(3, 0x60);
const V3_61: u64 = self_version(3, 0x61);
const V4_93: u64 = self_version(4, 0x93);

fn h(byte: u8, len: usize) -> String {
    hex(&vec![byte; len])
}

fn parse(label: &str, text: &str) -> Result<KeyVault, KeyVaultError> {
    KeyVault::parse(Path::new(label), text.as_bytes())
}

fn erks<'a>(keys: impl Iterator<Item = &'a SelfKey>) -> Vec<u8> {
    keys.map(|k| k.erk[0]).collect()
}

fn lv2_toml(entries: &[(&str, u8)]) -> String {
    entries
        .iter()
        .map(|(label, byte)| {
            format!(
                "[[lv2]]\n{label}\nerk = \"{}\"\nriv = \"{}\"\n",
                h(*byte, 32),
                h(*byte, 16)
            )
        })
        .collect()
}

#[test]
fn a_toml_lv2_keyset_is_labeled_by_its_version_range_and_round_trips() {
    let v = parse(
        "keys.toml",
        &lv2_toml(&[
            ("version = \"3.60-3.61\"", 0xD1),
            ("version = \"0003005500000000\"", 0xD2),
            ("label = \"from-a-friend\"", 0xD3),
        ]),
    )
    .unwrap();
    let versions: Vec<String> = v.lv2_versions().map(|r| r.to_string()).collect();
    assert_eq!(versions, ["3.55", "3.60-3.61"]);
    assert_eq!(v.unlabeled_count(SelfClass::Lv2), 1);
    assert_eq!(v.keyset_count(SelfClass::Lv2), 3);
    assert_eq!(v.labels(SelfClass::Lv2), ["3.55", "3.60-3.61"]);

    let again = parse("again.toml", &v.to_toml()).unwrap();
    assert_eq!(
        again.lv2_versions().collect::<Vec<_>>(),
        v.lv2_versions().collect::<Vec<_>>()
    );
    assert!(v.to_toml().contains("[[lv2]]\nversion = \"3.60-3.61\"\n"));
    assert!(v.to_toml().contains("[[lv2]]\nlabel = \"from-a-friend\"\n"));
}

#[test]
fn the_candidates_for_a_version_lead_with_the_range_that_holds_it() {
    let v = parse(
        "keys.toml",
        &lv2_toml(&[
            ("version = \"4.20-4.93\"", 0xD1),
            ("version = \"3.60-3.61\"", 0xD2),
            ("label = \"loose\"", 0xD3),
            ("version = \"3.55\"", 0xD4),
        ]),
    )
    .unwrap();
    assert_eq!(erks(v.lv2_key_candidates(V3_61)), [0xD2, 0xD4, 0xD1, 0xD3]);
    assert_eq!(erks(v.lv2_key_candidates(V4_93)), [0xD1, 0xD4, 0xD2, 0xD3]);
    // A version no range holds still walks everything, labeled first.
    assert_eq!(
        erks(v.lv2_key_candidates(self_version(1, 0x02))),
        [0xD4, 0xD2, 0xD1, 0xD3]
    );
    let empty = KeyVault::empty();
    assert_eq!(erks(empty.lv2_key_candidates(V3_55)), []);
}

#[test]
fn a_revision_on_an_lv2_entry_and_a_version_on_an_app_entry_are_refused_by_name() {
    let err = parse("keys.toml", &lv2_toml(&[("revision = 0x0a", 0xD1)])).unwrap_err();
    assert!(
        matches!(
            &err,
            KeyVaultError::WrongLabelKind {
                class: SelfClass::Lv2,
                expected: "version",
                found: "revision",
                ..
            }
        ),
        "{err}"
    );
    let err = parse(
        "keys.toml",
        &format!(
            "[[app]]\nversion = \"3.55\"\nerk = \"{}\"\nriv = \"{}\"\n",
            h(0xB1, 32),
            h(0xB2, 16)
        ),
    )
    .unwrap_err();
    assert!(
        matches!(
            &err,
            KeyVaultError::WrongLabelKind {
                class: SelfClass::App,
                expected: "revision",
                found: "version",
                ..
            }
        ),
        "{err}"
    );
    let err = parse("keys.toml", &lv2_toml(&[("version = \"3.5\"", 0xD1)])).unwrap_err();
    assert!(
        matches!(&err, KeyVaultError::BadVersion { value, .. } if value == "3.5"),
        "{err}"
    );
}

#[test]
fn two_lv2_keysets_for_one_range_with_different_keys_are_a_conflict() {
    let err = parse(
        "keys.toml",
        &lv2_toml(&[("version = \"3.55\"", 0xD1), ("version = \"3.55\"", 0xD2)]),
    )
    .unwrap_err();
    assert!(
        matches!(&err, KeyVaultError::Conflict { what, .. } if what == "lv2 versions 3.55"),
        "{err}"
    );
    // The same key twice is one keyset.
    let v = parse(
        "keys.toml",
        &lv2_toml(&[("version = \"3.55\"", 0xD1), ("version = \"3.55\"", 0xD1)]),
    )
    .unwrap();
    assert_eq!(v.keyset_count(SelfClass::Lv2), 1);
}

#[test]
fn a_pasted_lv2_table_row_is_labeled_by_its_version_range_and_a_loader_row_is_set_aside() {
    let text = format!(
        "lv2 3.60~3.61 {} {}\n\
         lv2 4.20-4.93 {} {}\n\
         lv2ldr 3.60-3.61 {} {}\n\
         lv2ldr::rlist 4.20-4.93 {} {}\n\
         lv1ldr 3.60-3.61 {} {}\n",
        h(0xD1, 32),
        h(0xD1, 16),
        h(0xD2, 32),
        h(0xD2, 16),
        h(0xD3, 32),
        h(0xD3, 16),
        h(0xD4, 32),
        h(0xD4, 16),
        h(0xD5, 32),
        h(0xD5, 16),
    );
    let v = parse("keys.txt", &text).unwrap();
    let versions: Vec<String> = v.lv2_versions().map(|r| r.to_string()).collect();
    assert_eq!(versions, ["3.60-3.61", "4.20-4.93"]);
    assert_eq!(v.unlabeled_count(SelfClass::Lv2), 0);
    assert_eq!(v.keyset_count(SelfClass::App), 0);
    let ignored: Vec<String> = v.ignored().iter().map(|i| i.reason.to_string()).collect();
    assert_eq!(ignored.len(), 3, "the three loader rows: {ignored:?}");
    assert!(ignored[0].contains("lv2ldr"), "{ignored:?}");
}

#[test]
fn a_scetool_lv2_block_takes_its_version_word_over_its_name_and_its_revision() {
    let text = format!(
        "[lv2-3.60]\ntype=SELF\nrevision=0A\nversion=0003006100000000\nself_type=LV2\nerk={}\nriv={}\n",
        h(0xD1, 32),
        h(0xD1, 16)
    );
    let v = parse("keys", &text).unwrap();
    assert_eq!(
        v.lv2_versions().collect::<Vec<_>>(),
        [Lv2Versions::single(V3_61)]
    );
    let bad = format!(
        "[lv2-3.60]\nself_type=LV2\nversion=nope\nerk={}\nriv={}\n",
        h(0xD1, 32),
        h(0xD1, 16)
    );
    let err = parse("keys", &bad).unwrap_err();
    assert!(
        matches!(&err, KeyVaultError::BadVersion { value, .. } if value == "nope"),
        "{err}"
    );
}

#[test]
fn per_key_lv2_files_pair_into_one_labeled_keyset() {
    let dir = crate::scratch_dir::scratch();
    std::fs::write(dir.join("lv2-key-3.60-3.61"), h(0xD1, 32)).unwrap();
    std::fs::write(dir.join("lv2-iv-3.60-3.61"), h(0xD1, 16)).unwrap();
    let v = KeyVault::load_from_path(&dir).unwrap();
    assert_eq!(
        v.lv2_versions().collect::<Vec<_>>(),
        [Lv2Versions {
            lo: V3_60,
            hi: V3_61
        }]
    );
}

#[test]
fn a_merge_carries_the_lv2_table_and_the_inventory_names_an_absent_one() {
    let mut a = parse("a.toml", &lv2_toml(&[("label = \"loose\"", 0xD2)])).unwrap();
    let b = parse(
        "b.toml",
        &lv2_toml(&[("version = \"3.55\"", 0xD1), ("label = \"other\"", 0xD3)]),
    )
    .unwrap();
    a.merge(b).unwrap();
    assert_eq!(
        a.lv2_versions().collect::<Vec<_>>(),
        [Lv2Versions::single(V3_55)]
    );
    assert_eq!(a.keyset_count(SelfClass::Lv2), 3);
    assert_eq!(erks(a.lv2_key_candidates(V3_55)), [0xD1, 0xD2, 0xD3]);
    assert!(a.summary().contains("lv2 1+2"), "{}", a.summary());
    assert!(!a
        .missing_for_decrypt()
        .contains(&"lv2 (no keyset)".to_string()));
    assert!(KeyVault::empty()
        .missing_for_decrypt()
        .contains(&"lv2 (no keyset)".to_string()));
    assert!(!crate::test_support::synthetic_vault()
        .missing_for_decrypt()
        .iter()
        .any(|m| m.starts_with("lv2")));
}
