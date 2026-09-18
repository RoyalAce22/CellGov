//! Key vault loading: every accepted keyfile shape, name aliasing,
//! conflict refusal, and location resolution.

use super::hex::{hex, is_hex_token, value_bytes};
use super::names::{classify_name, parse_revision, Kind, NameClass, Part};
use super::*;
use crate::scratch_dir::scratch;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn h(byte: u8, len: usize) -> String {
    hex(&vec![byte; len])
}

const SCALARS: [(Slot, u8); 8] = [
    (Slot::PupHmac, 0x11),
    (Slot::PkgAes, 0x22),
    (Slot::NpKlicKey, 0x33),
    (Slot::NpKlicFree, 0x44),
    (Slot::RapKey, 0x55),
    (Slot::RapPbox, 0x66),
    (Slot::RapE1, 0x77),
    (Slot::RapE2, 0x88),
];

fn full_toml() -> String {
    let mut t = String::new();
    for (slot, b) in SCALARS {
        t.push_str(&format!(
            "{} = \"{}\"\n",
            slot.name(),
            h(b, slot.byte_len())
        ));
    }
    t.push_str(&format!(
        "[[scepkg]]\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xA1, 32),
        h(0xA2, 16)
    ));
    t.push_str(&format!(
        "[[app]]\nrevision = 0x0a\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xB1, 32),
        h(0xB2, 16)
    ));
    t.push_str(&format!(
        "[[app]]\nlabel = \"from-a-friend\"\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xB3, 32),
        h(0xB4, 16)
    ));
    t.push_str(&format!(
        "[[npdrm]]\nrevision = \"0A\"\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xC1, 32),
        h(0xC2, 16)
    ));
    t.push_str(&format!(
        "[[lv2]]\nversion = \"3.60-3.61\"\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xD1, 32),
        h(0xD2, 16)
    ));
    t
}

fn parse_toml(text: &str) -> Result<KeyVault, KeyVaultError> {
    KeyVault::parse(Path::new("keys.toml"), text.as_bytes())
}

fn parse_text(text: &str) -> Result<KeyVault, KeyVaultError> {
    KeyVault::parse(Path::new("keys.txt"), text.as_bytes())
}

fn assert_full(v: &KeyVault) {
    assert_eq!(v.pup_hmac().unwrap(), &[0x11u8; 0x40]);
    assert_eq!(v.pkg_aes().unwrap(), &[0x22u8; 16]);
    assert_eq!(v.np_klic_key().unwrap(), &[0x33u8; 16]);
    assert_eq!(v.np_klic_free().unwrap(), &[0x44u8; 16]);
    assert_eq!(v.rap_key().unwrap(), &[0x55u8; 16]);
    assert_eq!(v.rap_pbox().unwrap(), &[0x66u8; 16]);
    assert_eq!(v.rap_e1().unwrap(), &[0x77u8; 16]);
    assert_eq!(v.rap_e2().unwrap(), &[0x88u8; 16]);
    let scepkg: Vec<&SelfKey> = v.scepkg_keys().unwrap().collect();
    assert_eq!(scepkg.len(), 1);
    assert_eq!(scepkg[0].erk, [0xA1u8; 32]);
    assert_eq!(scepkg[0].riv, [0xA2u8; 16]);
    let app = v.self_key(SelfClass::App, 0x0A).unwrap();
    assert_eq!(app.erk, [0xB1u8; 32]);
    assert_eq!(app.riv, [0xB2u8; 16]);
    assert_eq!(v.unlabeled_count(SelfClass::App), 1);
    let npdrm = v.self_key(SelfClass::Npdrm, 0x0A).unwrap();
    assert_eq!(npdrm.erk, [0xC1u8; 32]);
    let lv2: Vec<&SelfKey> = v.lv2_key_candidates(0x0003_0060_0000_0000).collect();
    assert_eq!(lv2.len(), 1);
    assert_eq!(lv2[0].erk, [0xD1u8; 32]);
    assert_eq!(v.labels(SelfClass::Lv2), ["3.60-3.61"]);
    assert!(
        v.missing_for_decrypt().is_empty(),
        "{:?}",
        v.missing_for_decrypt()
    );
}

#[test]
fn the_toml_schema_round_trips_through_to_toml() {
    let v = parse_toml(&full_toml()).unwrap();
    assert_full(&v);
    let again = parse_toml(&v.to_toml()).unwrap();
    assert_full(&again);
    assert_eq!(again.to_toml(), v.to_toml());
}

#[test]
fn an_empty_vault_refuses_every_slot_by_name() {
    let v = KeyVault::empty();
    for (slot, _) in SCALARS {
        let err = v.scalar(slot).unwrap_err();
        assert!(
            matches!(err, KeyVaultError::MissingSlot { slot: s } if s == slot),
            "{slot}: {err}"
        );
        assert!(err.to_string().contains(slot.name()));
    }
    assert!(matches!(
        v.scepkg_keys().map(|_| ()).unwrap_err(),
        KeyVaultError::MissingScepkg
    ));
    assert_eq!(v.missing_for_decrypt().len(), SCALARS.len() + 4);
    assert!(v.self_key_candidates(SelfClass::App, 0).next().is_none());
}

#[test]
fn candidates_yield_the_labeled_key_first_then_every_unlabeled_one() {
    let v = parse_toml(&full_toml()).unwrap();
    let c: Vec<&SelfKey> = v.self_key_candidates(SelfClass::App, 0x0A).collect();
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].erk, [0xB1u8; 32]);
    assert_eq!(c[1].erk, [0xB3u8; 32]);
    let unknown: Vec<&SelfKey> = v.self_key_candidates(SelfClass::App, 0x1D).collect();
    assert_eq!(unknown.len(), 1, "only the unlabeled candidate remains");
    assert_eq!(unknown[0].erk, [0xB3u8; 32]);
}

#[test]
fn toml_scalar_aliases_and_byte_arrays_are_accepted_and_unknown_fields_refused() {
    let v = parse_toml(&format!(
        "PUP_KEY = \"{}\"\nklic_ps3_free = [0x44, {}]\n",
        h(0x11, 64),
        ["0x44"; 15].join(", ")
    ))
    .unwrap();
    assert_eq!(v.pup_hmac().unwrap(), &[0x11u8; 64]);
    assert_eq!(v.np_klic_free().unwrap(), &[0x44u8; 16]);

    let err = parse_toml(&format!("frobnicate = \"{}\"\n", h(0x11, 16))).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::UnknownName { ref name, .. } if name == "frobnicate"),
        "{err}"
    );
    assert!(
        err.to_string().contains("pup_hmac"),
        "names the slots: {err}"
    );
}

#[test]
fn toml_revisions_outside_the_self_range_are_refused() {
    for bad in ["revision = 0x8000", "revision = -1", "revision = \"zz\""] {
        let err = parse_toml(&format!(
            "[[app]]\n{bad}\nerk = \"{}\"\nriv = \"{}\"\n",
            h(1, 32),
            h(2, 16)
        ))
        .unwrap_err();
        assert!(
            matches!(err, KeyVaultError::BadRevision { .. }),
            "{bad}: {err}"
        );
    }
}

#[test]
fn a_scetool_keyfile_files_app_npdrm_lv2_pkg_and_the_np_scalar_keysets() {
    let text = format!(
        "# scetool data/keys\n\
         [app-3.55]\ntype=SELF\nrevision=0A\nversion=0003005500000000\nself_type=APP\n\
         erk={}\nriv={}\npub={}\npriv={}\nctype=33\n\n\
         [npdrm-3.55]\ntype=SELF\nrevision=0A\nself_type=NPDRM\nkey={}\niv={}\nctype=33\n\n\
         [lv2-3.55]\ntype=SELF\nrevision=0A\nself_type=LV2\nerk={}\nriv={}\n\n\
         [pkg-key-retail]\ntype=PKG\nrevision=00\nerk={}\nriv={}\npub={}\nctype=33\n\n\
         [NP_klic_free]\ntype=OTHER\nkey={}\n\n\
         [NP_klic_key]\ntype=OTHER\nerk={}\n\n\
         [NP_tid]\ntype=OTHER\nkey={}\n",
        h(0xB1, 32),
        h(0xB2, 16),
        h(0xEE, 40),
        h(0xEF, 21),
        h(0xC1, 32),
        h(0xC2, 16),
        h(0x99, 32),
        h(0x98, 16),
        h(0xA1, 32),
        h(0xA2, 16),
        h(0xEE, 40),
        h(0x44, 16),
        h(0x33, 16),
        h(0x45, 16),
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.self_key(SelfClass::App, 0x0A).unwrap().erk, [0xB1u8; 32]);
    assert_eq!(
        v.self_key(SelfClass::Npdrm, 0x0A).unwrap().riv,
        [0xC2u8; 16]
    );
    assert_eq!(v.scepkg_keys().unwrap().next().unwrap().erk, [0xA1u8; 32]);
    assert_eq!(v.np_klic_free().unwrap(), &[0x44u8; 16]);
    assert_eq!(v.np_klic_key().unwrap(), &[0x33u8; 16]);
    // The LV2 block carries no `version=`, so its name labels it; its
    // `revision=0A` names no key.
    let lv2: Vec<String> = v.lv2_versions().map(|r| r.to_string()).collect();
    assert_eq!(lv2, ["3.55"]);
    assert_eq!(
        v.lv2_key_candidates(0x0003_0055_0000_0000)
            .next()
            .unwrap()
            .erk,
        [0x99u8; 32]
    );
    assert_eq!(v.self_key(SelfClass::Lv2, 0x0A), None);
    let ignored: Vec<String> = v.ignored().iter().map(|i| i.reason.to_string()).collect();
    assert_eq!(ignored.len(), 1, "{ignored:?}");
    assert!(ignored[0].contains("NP_tid"), "{ignored:?}");
}

#[test]
fn a_keyset_block_without_a_revision_is_an_unlabeled_candidate() {
    let text = format!(
        "[appldr]\nself_type=APP\nerk={}\nriv={}\n",
        h(0xB3, 32),
        h(0xB4, 16)
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.unlabeled_count(SelfClass::App), 1);
    assert!(v.labeled_revisions(SelfClass::App).next().is_none());
    assert!(v.to_toml().contains("label = \"appldr\""));
}

#[test]
fn wiki_named_lines_and_a_heading_over_a_bare_value_are_filed_by_alias() {
    let text = format!(
        "PS3 PUP HMAC\n{}\n\n\
         ps3_klic_dec_key: {}\n\
         klic_ps3_free: {}\n\
         npdrm_pkg_ps3_aes_key: {}\n\
         npdrm_pkg_ps3_idu_aes_key: {}\n\
         npdrm_idps_seed: {}\n\
         sc_key::key_for_master : {}\n\
         Location: nas_plugin.sprx (PS3 FW 0.93-4.88 CEX/DEX/TOOL)\n\
         rap_init_key = {}\nrap_pbox = {}\nrap_e1 = {}\nrap_e2 = {}\n\
         {}\n",
        h(0x11, 64),
        h(0x33, 16),
        h(0x44, 16),
        h(0x22, 16),
        h(0x23, 16),
        h(0x24, 16),
        h(0x25, 16),
        h(0x55, 16),
        h(0x66, 16),
        h(0x77, 16),
        h(0x88, 16),
        h(0x26, 16),
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.pup_hmac().unwrap(), &[0x11u8; 64]);
    assert_eq!(v.np_klic_key().unwrap(), &[0x33u8; 16]);
    assert_eq!(v.np_klic_free().unwrap(), &[0x44u8; 16]);
    assert_eq!(v.pkg_aes().unwrap(), &[0x22u8; 16]);
    assert_eq!(v.rap_key().unwrap(), &[0x55u8; 16]);
    assert_eq!(v.rap_pbox().unwrap(), &[0x66u8; 16]);
    assert_eq!(v.rap_e1().unwrap(), &[0x77u8; 16]);
    assert_eq!(v.rap_e2().unwrap(), &[0x88u8; 16]);
    let ignored: Vec<(Option<usize>, String)> = v
        .ignored()
        .iter()
        .map(|i| (i.at.line_number(), i.reason.to_string()))
        .collect();
    // The IDU key reads as an NPDRM half of the wrong length, the IDPS
    // seed and `key_for_master` as unrecognized names, and the
    // trailing bare value has no heading over it.
    assert_eq!(ignored.len(), 4, "{ignored:?}");
    assert!(
        ignored
            .iter()
            .any(|(l, r)| *l == Some(8) && r.contains("npdrm_idps_seed")),
        "{ignored:?}"
    );
    assert!(
        ignored
            .iter()
            .any(|(l, r)| *l == Some(9) && r.contains("key_for_master")),
        "{ignored:?}"
    );
    assert!(
        ignored
            .iter()
            .any(|(l, r)| *l == Some(15) && r.contains("no name")),
        "{ignored:?}"
    );
    assert!(
        ignored.iter().any(|(l, r)| *l == Some(7)
            && r.contains("npdrm_pkg_ps3_idu_aes_key")
            && r.contains("32")),
        "{ignored:?}"
    );
}

#[test]
fn pasted_key_table_rows_file_app_npdrm_and_package_rows_and_skip_the_rest() {
    let text = format!(
        "selftype version revision fw ERK RIV PUBLIC PRIVATE CURVE_TYPE\n\
         app 3.55 0x0A 3.55++ {} {} {} {} 0x33\n\
         npdrm 3.55 0x0A np 3.55++ {} {} {} {} 0x33\n\
         seven 3.55 0x00 =>3.55 {} {} {} {} 0x33\n\
         0.60~3.55 (pkg) {} {} {} {} 0x17\n\
         3.56~4.93 (spkg) | {} | {} | {} | {} | 0x17\n",
        h(0xB1, 32),
        h(0xB2, 16),
        h(0xEE, 40),
        h(0xEF, 21),
        h(0xC1, 32),
        h(0xC2, 16),
        h(0xEE, 40),
        h(0xEF, 21),
        h(0x99, 32),
        h(0x98, 16),
        h(0xEE, 40),
        h(0xEF, 21),
        h(0xA1, 32),
        h(0xA2, 16),
        h(0xEE, 40),
        h(0xEF, 21),
        h(0xA3, 32),
        h(0xA4, 16),
        h(0xEE, 40),
        h(0xEF, 21),
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.self_key(SelfClass::App, 0x0A).unwrap().erk, [0xB1u8; 32]);
    assert_eq!(
        v.self_key(SelfClass::Npdrm, 0x0A).unwrap().erk,
        [0xC1u8; 32]
    );
    assert_eq!(v.unlabeled_count(SelfClass::App), 0);
    let scepkg: Vec<&SelfKey> = v.scepkg_keys().unwrap().collect();
    assert_eq!(scepkg.len(), 2);
    assert_eq!(scepkg[0].erk, [0xA1u8; 32]);
    assert_eq!(scepkg[1].erk, [0xA3u8; 32]);
    assert_eq!(v.ignored().len(), 1, "{:?}", v.ignored());
    assert!(matches!(
        &v.ignored()[0].reason,
        IgnoreReason::UnusedKeyset { name } if name == "seven"
    ));
}

#[test]
fn a_reported_name_never_carries_the_hex_it_swallowed() {
    let text = format!("ss::Kf1_u0: {} ss::Kf2_u0: {}\n", h(0x21, 16), h(0x22, 16));
    let v = parse_text(&text).unwrap();
    assert_eq!(v.ignored().len(), 1);
    let reason = v.ignored()[0].reason.to_string();
    assert!(reason.contains("<hex>"), "{reason}");
    assert!(!reason.contains("2121"), "{reason}");
}

#[test]
fn a_directory_of_per_key_files_pairs_halves_by_label_and_sets_disc_keys_aside() {
    let dir = scratch();
    let w = |name: &str, bytes: &[u8]| std::fs::write(dir.join(name), bytes).unwrap();
    w("app-key-0a", h(0xB1, 32).as_bytes());
    w("app-iv-0a", &[0xB2u8; 16]);
    w(
        "npdrm-key-355.txt",
        format!("0x{}\n", h(0xC1, 32)).as_bytes(),
    );
    w("npdrm-iv-355.txt", h(0xC2, 16).as_bytes());
    w("app-key-0b", &[0xB5u8; 32]);
    w("pkg-key", &[0xA1u8; 32]);
    w("pkg-iv", &[0xA2u8; 16]);
    w("pup-hmac", h(0x11, 64).as_bytes());
    w("np-klic-free", &[0x44u8; 16]);
    w(
        "Some Game (Europe).dkey",
        h(0xD1, 16).to_ascii_uppercase().as_bytes(),
    );
    w("Other Game (Japan).key", &[0xD2u8; 16]);
    w("notes.pdf", b"%PDF-1.4 not a key");
    w("license.rap", &[0u8; 16]);
    w(".hidden", &[0xFFu8; 16]);
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub").join("rap-e1"), [0x77u8; 16]).unwrap();

    let v = KeyVault::load_from_path(&dir).unwrap();
    assert_eq!(v.self_key(SelfClass::App, 0x0A).unwrap().erk, [0xB1u8; 32]);
    assert_eq!(v.self_key(SelfClass::App, 0x0A).unwrap().riv, [0xB2u8; 16]);
    assert_eq!(
        v.unlabeled_count(SelfClass::Npdrm),
        1,
        "355 is a firmware label, not a revision"
    );
    assert_eq!(
        v.self_key_candidates(SelfClass::Npdrm, 0x0A)
            .next()
            .unwrap()
            .erk,
        [0xC1u8; 32]
    );
    assert_eq!(v.scepkg_keys().unwrap().next().unwrap().riv, [0xA2u8; 16]);
    assert_eq!(v.pup_hmac().unwrap(), &[0x11u8; 64]);
    assert_eq!(v.np_klic_free().unwrap(), &[0x44u8; 16]);
    assert_eq!(v.rap_e1().unwrap(), &[0x77u8; 16]);
    let reasons: Vec<String> = v
        .ignored()
        .iter()
        .map(|i| format!("{}: {}", i.at, i.reason))
        .collect();
    assert_eq!(reasons.len(), 6, "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains("hidden")), "{reasons:?}");
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("app-key-0b") && r.contains("riv")),
        "{reasons:?}"
    );
    assert!(reasons.iter().any(|r| r.contains(".pdf")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains("RAP")), "{reasons:?}");
    assert!(
        reasons
            .iter()
            .any(|r| r.contains(".dkey") && r.contains("16-byte value with no name")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|r| r.contains("Other Game (Japan)") && r.contains("binary content")),
        "{reasons:?}"
    );
}

#[test]
fn a_directory_that_is_not_there_is_named() {
    let dir = scratch();
    let err = KeyVault::load_from_path(&dir.join("absent")).unwrap_err();
    assert!(matches!(err, KeyVaultError::Missing { .. }), "{err}");
}

#[test]
fn a_slot_given_twice_with_different_values_is_a_conflict_and_the_same_value_is_not() {
    let same = format!(
        "pkg_aes = \"{}\"\nPKG_AES_KEY = \"{}\"\n",
        h(0x22, 16),
        h(0x22, 16)
    );
    assert!(parse_toml(&same).is_ok());
    let differ = format!(
        "pkg_aes = \"{}\"\nPKG_AES_KEY = \"{}\"\n",
        h(0x22, 16),
        h(0x23, 16)
    );
    let err = parse_toml(&differ).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::Conflict { ref what, .. } if what == "pkg_aes"),
        "{err}"
    );

    let mut a = parse_toml(&format!(
        "[[app]]\nrevision = 1\nerk = \"{}\"\nriv = \"{}\"\n",
        h(1, 32),
        h(2, 16)
    ))
    .unwrap();
    let b = parse_toml(&format!(
        "[[app]]\nrevision = 1\nerk = \"{}\"\nriv = \"{}\"\n",
        h(9, 32),
        h(2, 16)
    ))
    .unwrap();
    let err = a.merge(b).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::Conflict { ref what, .. } if what == "app revision 0x0001"),
        "{err}"
    );
}

#[test]
fn a_scalar_of_the_wrong_length_is_refused_naming_the_slot_and_line() {
    let err = parse_text(&format!("np_klic_key: {}\n", h(0x33, 32))).unwrap_err();
    match err {
        KeyVaultError::WrongLength {
            at,
            what,
            got,
            want,
        } => {
            assert_eq!(at.line_number(), Some(1));
            assert_eq!(what, "np_klic_key");
            assert_eq!((got, want), (32, 16));
        }
        other => panic!("expected WrongLength, got {other}"),
    }
}

#[test]
fn hex_decoding_tolerates_prefixes_and_separators_and_names_what_it_refuses() {
    assert_eq!(decode_hex("0A0b"), Ok(vec![0x0A, 0x0B]));
    assert_eq!(decode_hex("0x0A 0x0B"), Ok(vec![0x0A, 0x0B]));
    assert_eq!(decode_hex("{ 0x0A, 0x0B, }"), Ok(vec![0x0A, 0x0B]));
    assert_eq!(decode_hex("0a:0b-0c_0d"), Ok(vec![0x0A, 0x0B, 0x0C, 0x0D]));
    assert_eq!(decode_hex("\"0a0b\""), Ok(vec![0x0A, 0x0B]));
    assert_eq!(decode_hex("0x000x12"), Ok(vec![0x00, 0x12]));
    assert_eq!(decode_hex("abc"), Err(HexError::OddLength { digits: 3 }));
    assert_eq!(decode_hex("0g"), Err(HexError::NonHex { ch: 'g' }));
}

#[test]
fn revision_labels_parse_only_as_short_hex() {
    assert_eq!(parse_revision("0A"), Some(0x0A));
    assert_eq!(parse_revision("0a"), Some(0x0A));
    assert_eq!(parse_revision("a"), Some(0x0A));
    assert_eq!(parse_revision("0x1d"), Some(0x1D));
    assert_eq!(parse_revision("rev0A"), Some(0x0A));
    assert_eq!(
        parse_revision("r10"),
        None,
        "a bare r is a letter, not a prefix"
    );
    assert_eq!(parse_revision("red"), None);
    assert_eq!(parse_revision("rff"), None);
    assert_eq!(parse_revision("0x0100"), Some(0x0100));
    assert_eq!(parse_revision("355"), None, "a firmware label");
    assert_eq!(parse_revision("0x8000"), None, "debug flag, not a revision");
    assert_eq!(parse_revision(""), None);
    assert_eq!(parse_revision("appldr"), None);
}

#[test]
fn names_classify_by_alias_and_by_class_part_suffix_words() {
    assert_eq!(
        classify_name("PS3 PUP HMAC", Some(64)),
        NameClass::Scalar(Slot::PupHmac)
    );
    assert_eq!(
        classify_name("PUP_KEY", None),
        NameClass::Scalar(Slot::PupHmac)
    );
    assert_eq!(
        classify_name("pkg-key", Some(16)),
        NameClass::Scalar(Slot::PkgAes)
    );
    assert_eq!(
        classify_name("pkg-key", Some(32)),
        NameClass::Half {
            kind: Kind::Scepkg,
            part: Part::Erk,
            label: String::new()
        }
    );
    assert_eq!(
        classify_name("NPDRM_iv_0x1C", None),
        NameClass::Half {
            kind: Kind::Class(SelfClass::Npdrm),
            part: Part::Riv,
            label: "0x1c".to_string()
        }
    );
    assert_eq!(
        classify_name("erk-app-3-55", None),
        NameClass::Half {
            kind: Kind::Class(SelfClass::App),
            part: Part::Erk,
            label: "3-55".to_string()
        }
    );
    assert_eq!(classify_name("app-pub-0A", None), NameClass::Unknown);
    assert_eq!(classify_name("edat-key-0", None), NameClass::Unknown);
    assert_eq!(classify_name("keys", None), NameClass::Unknown);
}

#[test]
fn location_prefers_the_environment_and_falls_back_to_the_imported_vault() {
    let vfs = scratch();
    let installed = installed_keys_dir(&vfs).join(INSTALLED_KEYS_FILE);

    let err = KeyVault::locate_from(Some(OsString::new()), &vfs).unwrap_err();
    assert!(matches!(err, KeyVaultError::EnvEmpty), "{err}");

    let err = KeyVault::locate_from(None, &vfs).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::NotConfigured { installed: ref p } if *p == installed),
        "{err}"
    );
    assert!(err.to_string().contains(ENV_KEYS));
    assert!(err.to_string().contains("keys import"));

    assert_eq!(
        KeyVault::locate_from(Some(OsString::from("D:/elsewhere/keys")), &vfs).unwrap(),
        PathBuf::from("D:/elsewhere/keys")
    );

    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, full_toml()).unwrap();
    assert_eq!(KeyVault::locate_from(None, &vfs).unwrap(), installed);
    let v = KeyVault::load_from_path(&installed).unwrap();
    assert_full(&v);
    assert_eq!(v.sources(), [installed]);
}

#[test]
fn a_toml_file_that_does_not_parse_is_named_as_such() {
    let err = parse_toml("this = is = not toml\n").unwrap_err();
    assert!(matches!(err, KeyVaultError::Toml { .. }), "{err}");
}

#[test]
fn debug_output_and_the_summary_carry_counts_not_bytes() {
    let v = parse_toml(&full_toml()).unwrap();
    let dbg = format!("{v:?}");
    assert!(dbg.contains("8 of 8 scalar slots"), "{dbg}");
    assert!(!dbg.contains("1111"), "no key bytes in Debug: {dbg}");
    let key = v.self_key(SelfClass::App, 0x0A).unwrap();
    assert_eq!(format!("{key:?}"), "SelfKey { .. }");
}

#[test]
fn a_text_file_that_yields_nothing_is_set_aside_as_such_and_a_deep_directory_is_named() {
    let v = parse_text("just some prose\nand another line\n").unwrap();
    assert_eq!(v.ignored().len(), 1, "{:?}", v.ignored());
    assert!(matches!(v.ignored()[0].reason, IgnoreReason::NothingFound));

    let dir = scratch();
    let mut deep = dir.to_path_buf();
    for level in 0..5 {
        deep = deep.join(format!("level{level}"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("pup-hmac"), [0x11u8; 64]).unwrap();
    let v = KeyVault::load_from_path(&dir).unwrap();
    assert!(v.pup_hmac().is_err(), "five levels down is past the walk");
    let reasons: Vec<String> = v.ignored().iter().map(|i| i.reason.to_string()).collect();
    assert_eq!(reasons.len(), 1, "{reasons:?}");
    assert!(reasons[0].contains("levels down"), "{reasons:?}");
}

#[test]
fn a_table_row_badged_sd_beside_a_plain_row_at_the_same_revision_is_a_candidate() {
    let text = format!(
        "app SD 0.60~0.84 0x00 0.60++ {} {}\napp 0.60~0.84 0x00 0.60++ {} {}\n",
        h(0xB9, 32),
        h(0xB8, 16),
        h(0xB1, 32),
        h(0xB2, 16),
    );
    let v = parse_text(&text).unwrap();
    assert_eq!(v.self_key(SelfClass::App, 0x00).unwrap().erk, [0xB1u8; 32]);
    let candidates: Vec<&SelfKey> = v.self_key_candidates(SelfClass::App, 0x00).collect();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[1].erk, [0xB9u8; 32]);
    assert!(v.to_toml().contains("label = \"sd-0x0000\""));
    assert!(v.ignored().is_empty());
}

#[test]
fn hex_decoding_of_empty_prefix_only_and_lone_characters_is_named() {
    assert_eq!(decode_hex(""), Ok(vec![]));
    assert_eq!(decode_hex("0x"), Ok(vec![]));
    assert_eq!(decode_hex("  , : "), Ok(vec![]));
    assert_eq!(decode_hex("0"), Err(HexError::OddLength { digits: 1 }));
    assert_eq!(decode_hex("x"), Err(HexError::NonHex { ch: 'x' }));
    assert_eq!(decode_hex("0x0"), Err(HexError::OddLength { digits: 1 }));
    assert_eq!(decode_hex("0X0a'0b'"), Ok(vec![0x0A, 0x0B]));
    assert_eq!(
        decode_hex("0a\u{e9}"),
        Err(HexError::NonHex { ch: '\u{e9}' })
    );
}

#[test]
fn a_hex_token_is_exactly_the_asked_length_after_one_prefix() {
    let hex32 = h(0xAB, 32);
    assert!(is_hex_token(&hex32, 32));
    assert!(is_hex_token(&format!("0x{hex32}"), 32));
    assert!(is_hex_token(&format!("0X{hex32}"), 32));
    assert!(!is_hex_token(&hex32, 16));
    assert!(!is_hex_token(&hex32[1..], 32));
    assert!(!is_hex_token(&format!("0x0x{hex32}"), 32));
    assert!(!is_hex_token(&format!("{hex32} "), 32));
    assert!(is_hex_token("", 0));
}

#[test]
fn a_raw_key_made_of_hex_digit_bytes_is_read_as_the_bytes_it_is() {
    let raw: [u8; 16] = *b"0123456789abcdef";
    assert_eq!(value_bytes(&raw), raw.to_vec());
    assert_eq!(value_bytes(b""), Vec::<u8>::new());
    assert_eq!(value_bytes(h(0xD1, 16).as_bytes()), vec![0xD1u8; 16]);
    assert_eq!(
        value_bytes(format!("0x{}\r\n", h(0xD1, 16)).as_bytes()),
        vec![0xD1u8; 16]
    );
    assert_eq!(value_bytes(&[0xD2u8; 16]), vec![0xD2u8; 16]);
}

#[test]
fn a_disc_table_in_a_keys_toml_is_refused_by_name() {
    let text = format!("[disc]\n\"Some Game (USA)\" = \"{}\"\n", h(0xD1, 16));
    let err = parse_toml(&text).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::UnknownName { ref name, .. } if name == "disc"),
        "{err}"
    );
}

#[test]
fn an_empty_key_file_is_set_aside_as_holding_no_key() {
    let v = KeyVault::parse(Path::new("Empty (USA).dkey"), b"").expect("empty file loads");
    assert_eq!(v.ignored().len(), 1, "{:?}", v.ignored());
    assert!(matches!(v.ignored()[0].reason, IgnoreReason::NothingFound));
}

#[test]
fn pkg_names_are_the_package_aes_key_unless_the_value_is_an_erk() {
    for name in ["pkg", "pkgkey", "PKG_KEY", "pkg-key"] {
        assert_eq!(
            classify_name(name, None),
            NameClass::Scalar(Slot::PkgAes),
            "{name}"
        );
        assert_eq!(
            classify_name(name, Some(16)),
            NameClass::Scalar(Slot::PkgAes),
            "{name}"
        );
        assert_eq!(
            classify_name(name, Some(32)),
            NameClass::Half {
                kind: Kind::Scepkg,
                part: Part::Erk,
                label: String::new()
            },
            "{name}"
        );
    }
    assert_eq!(
        classify_name("pkg-iv", Some(16)),
        NameClass::Half {
            kind: Kind::Scepkg,
            part: Part::Riv,
            label: String::new()
        }
    );
    assert_eq!(
        classify_name("pkg-key-0", Some(32)),
        NameClass::Half {
            kind: Kind::Scepkg,
            part: Part::Erk,
            label: "0".to_string()
        }
    );
    assert_eq!(
        classify_name("npdrm_idps_seed", Some(16)),
        NameClass::Unknown
    );
    assert_eq!(classify_name("key", Some(32)), NameClass::Unknown);
    assert_eq!(classify_name("", None), NameClass::Unknown);
}

#[test]
fn revision_prefixes_need_digits_behind_them_and_bare_words_are_not_revisions() {
    assert_eq!(parse_revision("revision"), None);
    assert_eq!(parse_revision("rev"), None);
    assert_eq!(parse_revision("r"), None);
    assert_eq!(parse_revision("0x"), None);
    assert_eq!(parse_revision("1000"), None);
    assert_eq!(parse_revision("0x7fff"), Some(0x7FFF));
    assert_eq!(parse_revision("0xffff"), None);
    assert_eq!(parse_revision("0x00000a"), None);
    assert_eq!(parse_revision(" 0A "), Some(0x0A));
    assert_eq!(parse_revision("0X1D"), Some(0x1D));
    assert_eq!(parse_revision("rev0x0100"), Some(0x0100));
}

#[test]
fn provenance_lines_are_one_based_and_a_zero_is_the_first_line() {
    let p = Path::new("keys.txt");
    assert_eq!(Provenance::at(p, 0).line_number(), Some(1));
    assert_eq!(Provenance::at(p, 1).line_number(), Some(1));
    assert_eq!(Provenance::at(p, 7).line_number(), Some(7));
    assert_eq!(
        Provenance::at(p, usize::MAX).line_number(),
        Some(usize::MAX)
    );
    assert_eq!(Provenance::file(p).line_number(), None);
    assert_eq!(Provenance::at(p, 3).to_string(), "keys.txt:3");
    assert_eq!(Provenance::file(p).to_string(), "keys.txt");
}

#[test]
fn a_candidate_that_is_later_labeled_stops_being_a_candidate() {
    let unlabeled = format!(
        "[[app]]\nlabel = \"appldr\"\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xB1, 32),
        h(0xB2, 16)
    );
    let labeled = format!(
        "[[app]]\nrevision = 0x0a\nerk = \"{}\"\nriv = \"{}\"\n",
        h(0xB1, 32),
        h(0xB2, 16)
    );
    let v = parse_toml(&format!("{unlabeled}{labeled}")).unwrap();
    assert_eq!(v.unlabeled_count(SelfClass::App), 0);
    assert_eq!(v.self_key_candidates(SelfClass::App, 0x0A).count(), 1);
    assert_eq!(v.self_key_candidates(SelfClass::App, 0x0B).count(), 0);
    assert_eq!(v.self_key_candidates(SelfClass::Npdrm, 0x0A).count(), 0);

    let w = parse_toml(&format!("{labeled}{unlabeled}")).unwrap();
    assert_eq!(w.unlabeled_count(SelfClass::App), 0);
    assert_eq!(
        w.to_toml(),
        v.to_toml(),
        "file order does not change the vault"
    );
}

#[test]
fn keyset_labels_with_quotes_backslashes_controls_and_non_ascii_round_trip() {
    let labels = [
        r#""Keyset \"Quoted\"""#,
        r#""Back\\slash""#,
        r#""Tab\u0009here""#,
        r#""Line\u000Abreak""#,
        r#""Caf\u00E9""#,
        r#""A.B [C] = D""#,
        r#""Del\u007Fete""#,
    ];
    let mut toml = String::new();
    for (i, label) in labels.iter().enumerate() {
        let fill = u8::try_from(i).unwrap() + 1;
        toml.push_str(&format!(
            "\n[[app]]\nlabel = {label}\nerk = \"{}\"\nriv = \"{}\"\n",
            h(fill, 32),
            h(fill, 16)
        ));
    }
    let v = parse_toml(&toml).unwrap();
    assert_eq!(v.unlabeled_count(SelfClass::App), labels.len());
    let written = v.to_toml();
    for escape in [r#"\""#, r#"\\"#, r#"\u0009"#, r#"\u000A"#, r#"\u007F"#] {
        assert!(written.contains(escape), "{escape}: {written}");
    }
    let again = parse_toml(&v.to_toml()).unwrap();
    assert_eq!(again.to_toml(), v.to_toml());
}

#[test]
fn an_environment_vault_that_is_not_there_is_named_when_loaded_not_when_located() {
    let vfs = scratch();
    let absent = vfs.join("no-such-keys");
    let location = KeyVault::locate_from(Some(absent.clone().into_os_string()), &vfs).unwrap();
    assert_eq!(location, absent);
    let err = KeyVault::load_from_path(&location).unwrap_err();
    assert!(
        matches!(err, KeyVaultError::Missing { ref path } if *path == absent),
        "{err}"
    );
    assert!(err.to_string().contains("does not exist"), "{err}");
}

#[test]
fn merging_keeps_both_sources_and_a_conflict_names_the_file_on_each_side() {
    let a_text = format!("pkg_aes = \"{}\"\n", h(0x22, 16));
    let b_text = format!("pkg_aes = \"{}\"\n", h(0x23, 16));
    let mut a = KeyVault::parse(Path::new("a.toml"), a_text.as_bytes()).unwrap();
    let b = KeyVault::parse(Path::new("b.toml"), b_text.as_bytes()).unwrap();
    match a.merge(b).unwrap_err() {
        KeyVaultError::Conflict {
            what,
            first,
            second,
        } => {
            assert_eq!(what, "pkg_aes");
            assert_eq!(first.path, Path::new("a.toml"));
            assert_eq!(second.path, Path::new("b.toml"));
        }
        other => panic!("expected Conflict, got {other}"),
    }

    let mut a = KeyVault::parse(Path::new("a.toml"), a_text.as_bytes()).unwrap();
    let same = KeyVault::parse(Path::new("c.toml"), a_text.as_bytes()).unwrap();
    a.merge(same).unwrap();
    assert_eq!(
        a.sources(),
        [PathBuf::from("a.toml"), PathBuf::from("c.toml")]
    );
    assert_eq!(
        a.slot_provenance(Slot::PkgAes).unwrap().path,
        Path::new("a.toml"),
        "the first definition keeps its provenance"
    );
}

#[test]
fn refusals_read_lowercase_first_and_carry_no_key_bytes() {
    let p = Path::new("keys.txt");
    let messages = [
        KeyVaultError::NotConfigured {
            installed: p.to_path_buf(),
        }
        .to_string(),
        KeyVaultError::EnvEmpty.to_string(),
        KeyVaultError::Missing {
            path: p.to_path_buf(),
        }
        .to_string(),
        KeyVaultError::MissingSlot { slot: Slot::RapE1 }.to_string(),
        KeyVaultError::MissingScepkg.to_string(),
        KeyVaultError::Conflict {
            what: "pkg_aes".to_string(),
            first: Provenance::at(p, 1),
            second: Provenance::at(p, 2),
        }
        .to_string(),
        KeyVaultError::UnknownName {
            at: Provenance::file(p),
            name: "x".to_string(),
        }
        .to_string(),
        KeyVaultError::BadRevision {
            at: Provenance::at(p, 4),
            value: "zz".to_string(),
        }
        .to_string(),
        KeyVaultError::WrongLength {
            at: Provenance::at(p, 5),
            what: "pkg_aes".to_string(),
            got: 32,
            want: 16,
        }
        .to_string(),
        KeyVaultError::BadHex {
            at: Provenance::at(p, 6),
            what: "pkg_aes".to_string(),
            source: HexError::NonHex { ch: 'g' },
        }
        .to_string(),
    ];
    for m in &messages {
        let first = m.chars().next().unwrap();
        assert!(
            first.is_ascii_lowercase() || m.starts_with(ENV_KEYS) || m.starts_with("keys.txt"),
            "{m}"
        );
        assert!(!m.ends_with('.'), "{m}");
    }

    let err = parse_text(&format!("np_klic_key: {}\n", h(0x33, 32))).unwrap_err();
    assert!(!err.to_string().contains("3333"), "{err}");
    let err = parse_toml(&format!("pkg_aes = \"{}zz\"\n", h(0x22, 16))).unwrap_err();
    assert!(matches!(err, KeyVaultError::BadHex { .. }), "{err}");
    assert!(!err.to_string().contains("2222"), "{err}");
}

#[test]
fn what_is_missing_is_named_slot_by_slot_and_table_by_table() {
    let missing = KeyVault::empty().missing_for_decrypt();
    for slot in Slot::ALL {
        assert!(missing.contains(&slot.name().to_string()), "{slot}");
    }
    assert!(missing.contains(&"scepkg".to_string()));
    assert!(missing.contains(&"app (no keyset)".to_string()));
    assert!(missing.contains(&"npdrm (no keyset)".to_string()));
    assert!(missing.contains(&"lv2 (no keyset)".to_string()));
    assert_eq!(missing.len(), Slot::ALL.len() + 4);
}
