use cellgov_install::keys::KeyVaultError;

use super::*;
use crate::cli::store::scratch::scratch;

fn hex_of(byte: u8, len: usize) -> String {
    format!("{byte:02x}").repeat(len)
}

#[test]
fn import_writes_a_toml_that_reloads_with_the_same_slots_and_remove_deletes_it() {
    let dir = scratch();
    let store = dir.join("vfs");
    let source = dir.join("keys.txt");
    std::fs::write(
        &source,
        format!(
            "pkg_aes = {}\npup_hmac = {}\n",
            hex_of(0x11, 16),
            hex_of(0x22, 64)
        ),
    )
    .unwrap();

    let imported = merge_into_installed(&source, &store, false).expect("import");
    assert_eq!(imported.pkg_aes().unwrap(), &[0x11u8; 16]);

    let file = installed_keys_dir(&store).join(INSTALLED_KEYS_FILE);
    assert!(file.is_file(), "{} written", file.display());
    let reloaded = KeyVault::load_from_path(&file).expect("reload");
    assert_eq!(reloaded.pkg_aes().unwrap(), &[0x11u8; 16]);
    assert_eq!(reloaded.pup_hmac().unwrap(), &[0x22u8; 64]);
    assert!(reloaded.np_klic_key().is_err(), "only the imported slots");
    assert_eq!(
        KeyVault::locate_from(None, &store).expect("located"),
        file,
        "the decrypting commands find what import wrote"
    );

    assert!(
        remove_installed(&store).expect("remove"),
        "something was removed"
    );
    assert!(!installed_keys_dir(&store).exists());
    assert!(
        !remove_installed(&store).expect("remove again"),
        "nothing left"
    );
}

#[test]
fn import_merges_into_the_installed_vault_unless_replace() {
    let dir = scratch();
    let store = dir.join("vfs");
    let first = dir.join("first.txt");
    let second = dir.join("second.txt");
    std::fs::write(&first, format!("pkg_aes = {}\n", hex_of(0x11, 16))).unwrap();
    std::fs::write(&second, format!("np_klic_key = {}\n", hex_of(0x33, 16))).unwrap();

    merge_into_installed(&first, &store, false).expect("first import");
    let merged = merge_into_installed(&second, &store, false).expect("second import merges");
    assert_eq!(merged.pkg_aes().unwrap(), &[0x11u8; 16]);
    assert_eq!(merged.np_klic_key().unwrap(), &[0x33u8; 16]);

    let replaced = merge_into_installed(&second, &store, true).expect("replace");
    assert!(
        replaced.pkg_aes().is_err(),
        "--replace drops the earlier slot"
    );
    assert_eq!(replaced.np_klic_key().unwrap(), &[0x33u8; 16]);
    let file = installed_keys_dir(&store).join(INSTALLED_KEYS_FILE);
    let on_disk = KeyVault::load_from_path(&file).expect("reload");
    assert!(on_disk.pkg_aes().is_err());
}

#[test]
fn import_refuses_a_disagreeing_slot_naming_both_definitions() {
    let dir = scratch();
    let store = dir.join("vfs");
    let first = dir.join("first.txt");
    let other = dir.join("other.txt");
    std::fs::write(&first, format!("pkg_aes = {}\n", hex_of(0x11, 16))).unwrap();
    std::fs::write(&other, format!("pkg_aes = {}\n", hex_of(0x12, 16))).unwrap();

    merge_into_installed(&first, &store, false).expect("first import");
    let err = merge_into_installed(&other, &store, false).expect_err("a conflict is refused");
    let StoreCliError::Keys(KeyVaultError::Conflict {
        what,
        first,
        second,
    }) = &err
    else {
        panic!("expected a Conflict, got {err:?}");
    };
    assert_eq!(what, "pkg_aes");
    assert!(first.path.ends_with(INSTALLED_KEYS_FILE), "{first}");
    assert!(second.path.ends_with("other.txt"), "{second}");
    let on_disk = KeyVault::load_from_path(&installed_keys_dir(&store).join(INSTALLED_KEYS_FILE))
        .expect("reload");
    assert_eq!(
        on_disk.pkg_aes().unwrap(),
        &[0x11u8; 16],
        "the installed vault is untouched by a refused import"
    );
}

#[test]
fn import_of_a_file_holding_no_key_is_refused_and_writes_nothing() {
    let dir = scratch();
    let store = dir.join("vfs");
    let source = dir.join("notes.txt");
    std::fs::write(&source, format!("frobnicate = {}\n", hex_of(0x11, 16))).unwrap();
    let err = merge_into_installed(&source, &store, false).expect_err("nothing usable");
    assert!(
        matches!(err, StoreCliError::KeysNothingUsable { .. }),
        "got {err:?}"
    );
    assert!(!installed_keys_dir(&store).exists());
}

#[test]
fn import_of_a_directory_of_disc_keys_is_refused_and_names_them_set_aside() {
    let dir = scratch();
    let source = dir.join("dkeys");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("Some Game (USA).dkey"), hex_of(0xD1, 16)).unwrap();
    std::fs::write(source.join("Other Game (Japan).key"), [0xD2u8; 16]).unwrap();
    let store = dir.join("vfs");

    let err = merge_into_installed(&source, &store, false).expect_err("no slot takes a disc key");
    assert!(
        matches!(err, StoreCliError::KeysNothingUsable { .. }),
        "got {err:?}"
    );
    assert!(!err.to_string().contains("disc key"), "{err}");
    assert!(!installed_keys_dir(&store).exists());

    let reasons: Vec<String> = KeyVault::load_from_path(&source)
        .expect("load")
        .ignored()
        .iter()
        .map(|i| format!("{}: {}", i.at, i.reason))
        .collect();
    assert_eq!(reasons.len(), 2, "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains(".dkey")), "{reasons:?}");
    assert!(reasons.iter().any(|r| r.contains(".key")), "{reasons:?}");
}

#[test]
fn import_of_an_absent_path_is_refused_by_name() {
    let dir = scratch();
    let err =
        merge_into_installed(&dir.join("absent"), &dir.join("vfs"), false).expect_err("absent");
    assert!(
        matches!(err, StoreCliError::Keys(KeyVaultError::Missing { .. })),
        "got {err:?}"
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
