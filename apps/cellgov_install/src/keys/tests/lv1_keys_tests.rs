//! The LV1 keyset class: the preserved material rows that file one,
//! the keyfile shapes that file one, and the version-range labels.

use std::path::Path;

use cellgov_ps3_abi::format::sce::self_version;

use super::hex::hex;
use super::*;

const V3_15: u64 = self_version(3, 0x15);
const V4_93: u64 = self_version(4, 0x93);

fn h(byte: u8, len: usize) -> String {
    hex(&vec![byte; len])
}

fn parse(label: &str, text: &str) -> KeyVault {
    KeyVault::parse(Path::new(label), text.as_bytes()).unwrap()
}

fn erks<'a>(keys: impl Iterator<Item = &'a SelfKey>) -> Vec<u8> {
    keys.map(|k| k.erk[0]).collect()
}

/// A `[[material]]` row as a catalog import preserved it: `erk`, `riv`
/// and `pub` components under a label of two version words.
fn material(kind: &str, label: &str, byte: u8) -> String {
    format!(
        "[[material]]\nkind = \"{kind}\"\nlabel = \"{label}\"\nerk = \"{}\"\nriv = \"{}\"\npub = \"{}\"\n\n",
        h(byte, 32),
        h(byte, 16),
        h(byte, 40)
    )
}

#[test]
fn a_preserved_lv1_material_row_files_an_lv1_keyset_under_its_range() {
    let v = parse(
        "keys.toml",
        &format!(
            "{}{}",
            material("lv1", "0003000000000000-0003003100000000", 0xA1),
            material("lv1", "0004002000000000-ffffffffffffffff", 0xA2),
        ),
    );
    assert_eq!(v.keyset_count(SelfClass::Lv1), 2);
    assert_eq!(
        v.labels(SelfClass::Lv1),
        ["3.00-3.31", "4.20-ffffffffffffffff"]
    );
    assert_eq!(v.unlabeled_count(SelfClass::Lv1), 0);
    assert_eq!(erks(v.lv1_key_candidates(V3_15)), [0xA1, 0xA2]);
    assert_eq!(erks(v.lv1_key_candidates(V4_93)), [0xA2, 0xA1]);
    // The rows stay preserved: the keyset is a reading of the material.
    assert_eq!(v.material().count(), 2);
    assert_eq!(v.keyset_count(SelfClass::Lv2), 0);
}

#[test]
fn a_material_row_of_another_kind_or_without_a_riv_files_no_lv1_keyset() {
    let no_riv = format!(
        "[[material]]\nkind = \"lv1\"\nlabel = \"0003005500000000-0003005500000000\"\nerk = \"{}\"\npub = \"{}\"\n\n",
        h(0xB1, 32),
        h(0xB1, 40)
    );
    let v = parse(
        "keys.toml",
        &format!(
            "{}{}{}",
            material("lv2", "0003006000000000-0003006100000000", 0xB2),
            material("iso", "0003006000000000-0003006100000000", 0xB3),
            no_riv,
        ),
    );
    assert_eq!(v.keyset_count(SelfClass::Lv1), 0);
    assert_eq!(v.material().count(), 3);
}

#[test]
fn an_lv1_keyset_round_trips_through_its_own_toml_table() {
    let v = parse(
        "keys.toml",
        &material("lv1", "0003005500000000-0003005500000000", 0xC1),
    );
    let text = v.to_toml();
    assert!(text.contains("[[lv1]]\nversion = \"3.55\"\n"), "{text}");
    let again = parse("again.toml", &text);
    assert_eq!(again.labels(SelfClass::Lv1), ["3.55"]);
    assert_eq!(erks(again.lv1_key_candidates(V3_15)), [0xC1]);
}

#[test]
fn the_lv1_keyfile_shapes_each_file_an_lv1_keyset() {
    let scetool = format!(
        "[lv1-3.60]\ntype=SELF\nrevision=0000\nversion=0003006000000000\nself_type=LV1\nerk={}\nriv={}\n",
        h(0xD1, 32),
        h(0xD1, 16)
    );
    let table_row = format!("lv1 3.65-3.66 {} {}\n", h(0xD2, 32), h(0xD2, 16));
    let catalog = format!(
        "void KeyVault::LoadSelfLv1Keys() {{\n  sk_LV1_arr.emplace_back(0x0003007000000000, 0x0003007400000000, 0x0, KEY, \"{}\", \"{}\", \"{}\", 0x01);\n}}\n",
        h(0xD3, 32),
        h(0xD3, 16),
        h(0xD3, 40)
    );
    assert_eq!(parse("keys", &scetool).labels(SelfClass::Lv1), ["3.60"]);
    assert_eq!(
        parse("table.txt", &table_row).labels(SelfClass::Lv1),
        ["3.65-3.66"]
    );
    let from_catalog = parse("key_vault.cpp", &catalog);
    assert_eq!(from_catalog.labels(SelfClass::Lv1), ["3.70-3.74"]);
    assert_eq!(from_catalog.material().count(), 1);
    let by_block_name = format!(
        "[lv1-3.75]\ntype=SELF\nerk={}\nriv={}\n",
        h(0xD4, 32),
        h(0xD4, 16)
    );
    assert_eq!(
        parse("keys", &by_block_name).labels(SelfClass::Lv1),
        ["3.75"]
    );
    let dir = crate::scratch_dir::scratch();
    std::fs::write(dir.join("lv1-key-3.80-3.81"), h(0xD5, 32)).unwrap();
    std::fs::write(dir.join("lv1-iv-3.80-3.81"), h(0xD5, 16)).unwrap();
    assert_eq!(
        KeyVault::load_from_path(&dir)
            .unwrap()
            .labels(SelfClass::Lv1),
        ["3.80-3.81"]
    );
}

#[test]
fn an_lv1_loader_row_files_no_lv1_keyset() {
    let v = parse(
        "keys",
        &format!(
            "[lv1ldr]\ntype=SELF\nself_type=LDR\nerk={}\nriv={}\n\n\
             [lv1ldr-3.60]\ntype=SELF\nerk={}\nriv={}\n",
            h(0xE1, 32),
            h(0xE1, 16),
            h(0xE2, 32),
            h(0xE2, 16)
        ),
    );
    assert_eq!(v.keyset_count(SelfClass::Lv1), 0);
    let reasons: Vec<&IgnoreReason> = v.ignored().iter().map(|i| &i.reason).collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(
        reasons
            .iter()
            .all(|r| matches!(r, IgnoreReason::UnusedKeyset { .. })),
        "{reasons:?}"
    );
}

#[test]
fn a_range_of_version_words_parses_with_either_end_dotted() {
    let words = Lv2Versions::parse("0003000000000000-0003003100000000").unwrap();
    assert_eq!(
        (words.lo, words.hi),
        (self_version(3, 0x00), self_version(3, 0x31))
    );
    let mixed = Lv2Versions::parse("4.20-ffffffffffffffff").unwrap();
    assert_eq!((mixed.lo, mixed.hi), (self_version(4, 0x20), u64::MAX));
    assert_eq!(mixed.to_string(), "4.20-ffffffffffffffff");
    let hi_dotted = Lv2Versions::parse("0003000000000000-3.31").unwrap();
    assert_eq!(
        (hi_dotted.lo, hi_dotted.hi),
        (self_version(3, 0x00), self_version(3, 0x31))
    );
    assert_eq!(
        Lv2Versions::parse("0003003100000000-0003000000000000"),
        None,
        "an out-of-order range"
    );
}

#[test]
fn the_inventory_counts_lv1_keysets_and_names_their_absence() {
    let v = parse(
        "keys.toml",
        &material("lv1", "0003005600000000-0003005600000000", 0xF1),
    );
    assert!(v.summary().contains("lv1 1+0"), "{}", v.summary());
    assert!(!v
        .missing_for_decrypt()
        .contains(&"lv1 (no keyset)".to_string()));
    assert!(KeyVault::empty()
        .missing_for_decrypt()
        .contains(&"lv1 (no keyset)".to_string()));
    assert_eq!(SelfClass::Lv1.label_kind(), "versions");
}

#[test]
fn a_merge_files_lv1_keysets_in_the_lv1_table_and_refuses_a_disagreeing_range() {
    let one = |label: &str, byte: u8| parse("keys.toml", &material("lv1", label, byte));
    let mut a = one("0003005500000000-0003005500000000", 0xA5);
    a.merge(one("0003006000000000-0003006000000000", 0xB5))
        .unwrap();
    assert_eq!(a.labels(SelfClass::Lv1), ["3.55", "3.60"]);
    assert_eq!(a.keyset_count(SelfClass::Lv2), 0);
    let err = a
        .merge(one("0003005500000000-0003005500000000", 0xC5))
        .unwrap_err();
    assert!(
        matches!(&err, KeyVaultError::Conflict { what, .. } if what == "lv1 versions 3.55"),
        "{err}"
    );
}

#[test]
fn an_lv1_catalog_row_with_no_version_range_reads_back_with_no_lv1_keyset() {
    let catalog = format!(
        "void KeyVault::LoadSelfLv1Keys() {{\n  sk_LV1_arr.emplace_back(KEY, \"{}\", \"{}\", \"{}\", 0x01);\n}}\n",
        h(0xA7, 32),
        h(0xA7, 16),
        h(0xA7, 40)
    );
    let imported = parse("key_vault.cpp", &catalog);
    assert_eq!(imported.keyset_count(SelfClass::Lv1), 0);
    assert_eq!(imported.material().count(), 1);
    let again = parse("keys.toml", &imported.to_toml());
    assert_eq!(again.keyset_count(SelfClass::Lv1), 0);
    assert_eq!(again.material().count(), 1);
}
