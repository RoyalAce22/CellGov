use cellgov_install::keys::KeyVaultError;

use super::*;
use crate::cli::store::scratch::scratch;

fn hex_of(byte: u8, len: usize) -> String {
    format!("{byte:02x}").repeat(len)
}

#[test]
fn an_import_of_a_directory_of_disc_keys_is_refused_in_the_command_wording() {
    let dir = scratch();
    let source = dir.join("dkeys");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("Some Game (USA).dkey"), hex_of(0xD1, 16)).unwrap();
    let store = dir.join("store");

    let err = StoreCliError::from(
        import_into(&source, &store, false).expect_err("no slot takes a disc key"),
    );
    let text = err.to_string();
    assert!(
        text.starts_with(&format!(
            "keys import: {} holds no scalar key",
            source.display()
        )),
        "{text}"
    );
    assert!(
        text.contains(&format!("run `keys show {}`", source.display())),
        "{text}"
    );
    assert!(!text.contains("disc key"), "{text}");
}

#[test]
fn a_refused_merge_keeps_the_vault_wording() {
    let dir = scratch();
    let store = dir.join("store");
    let first = dir.join("first.txt");
    let other = dir.join("other.txt");
    std::fs::write(&first, format!("pkg_aes = {}\n", hex_of(0x11, 16))).unwrap();
    std::fs::write(&other, format!("pkg_aes = {}\n", hex_of(0x12, 16))).unwrap();
    import_into(&first, &store, false).expect("first import");

    let err = StoreCliError::from(import_into(&other, &store, false).expect_err("conflict"));
    assert!(
        matches!(err, StoreCliError::Keys(KeyVaultError::Conflict { .. })),
        "got {err:?}"
    );
    assert!(
        err.to_string()
            .starts_with("pkg_aes is given twice with different values: "),
        "{err}"
    );
}

#[test]
fn the_key_inventory_names_every_slot_and_what_the_decrypt_paths_still_lack() {
    let vault = KeyVault::parse(
        Path::new("k.txt"),
        format!(
            "pkg_aes = {}\nfrobnicate = {}\n",
            hex_of(0x11, 16),
            hex_of(0x12, 16)
        )
        .as_bytes(),
    )
    .expect("parse");
    let report = render_inventory(Path::new("k.txt"), &vault);
    assert!(report.starts_with("key vault: k.txt\n"), "{report}");
    assert!(
        report.contains("pkg_aes: 16 bytes, from k.txt:1"),
        "{report}"
    );
    for slot in Slot::ALL.iter().filter(|s| **s != Slot::PkgAes) {
        assert!(
            report.contains(&format!("{}: missing", slot.name())),
            "{slot}: {report}"
        );
    }
    assert!(report.contains("scepkg: 0 keyset(s)"), "{report}");
    assert!(
        report.contains("app: revisions (none), 0 unlabeled"),
        "{report}"
    );
    assert!(
        report.contains("lv2: versions (none), 0 unlabeled"),
        "{report}"
    );
    assert!(
        report.contains("app (no keyset), npdrm (no keyset), lv2 (no keyset)"),
        "{report}"
    );
    assert!(!report.contains("disc"), "{report}");
    assert!(report.contains("k.txt:2: name \"frobnicate\""), "{report}");
    assert!(
        report.contains("missing for decrypt: pup_hmac, "),
        "{report}"
    );
    assert!(!report.contains("decrypt paths: ready"), "{report}");
    assert!(
        !report.contains(&hex_of(0x11, 16)),
        "no key bytes: {report}"
    );
}
