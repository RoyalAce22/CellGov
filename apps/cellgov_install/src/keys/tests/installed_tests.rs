use super::*;
use crate::scratch_dir::scratch;

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

    let outcome = import_into(&source, &store, false).expect("import");
    assert!(!outcome.merged, "nothing was installed to merge into");
    assert_eq!(outcome.vault.pkg_aes().unwrap(), &[0x11u8; 16]);

    let file = installed_keys_dir(&store).join(INSTALLED_KEYS_FILE);
    assert_eq!(outcome.file, file);
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

    import_into(&first, &store, false).expect("first import");
    let merged = import_into(&second, &store, false).expect("second import merges");
    assert!(merged.merged);
    assert_eq!(merged.vault.pkg_aes().unwrap(), &[0x11u8; 16]);
    assert_eq!(merged.vault.np_klic_key().unwrap(), &[0x33u8; 16]);

    let replaced = import_into(&second, &store, true).expect("replace");
    assert!(!replaced.merged, "--replace writes a fresh vault");
    assert!(
        replaced.vault.pkg_aes().is_err(),
        "--replace drops the earlier slot"
    );
    assert_eq!(replaced.vault.np_klic_key().unwrap(), &[0x33u8; 16]);
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

    import_into(&first, &store, false).expect("first import");
    let err = import_into(&other, &store, false).expect_err("a conflict is refused");
    let KeyImportError::Vault(KeyVaultError::Conflict {
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
    let err = import_into(&source, &store, false).expect_err("nothing usable");
    assert!(
        matches!(&err, KeyImportError::NothingUsable { path } if *path == source),
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

    let err = import_into(&source, &store, false).expect_err("no slot takes a disc key");
    assert!(
        matches!(err, KeyImportError::NothingUsable { .. }),
        "got {err:?}"
    );
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
    let err = import_into(&dir.join("absent"), &dir.join("vfs"), false).expect_err("absent");
    assert!(
        matches!(err, KeyImportError::Vault(KeyVaultError::Missing { .. })),
        "got {err:?}"
    );
}

#[test]
fn a_lone_lv2_keyset_is_a_usable_key() {
    let vault = KeyVault::parse(
        Path::new("keys.toml"),
        format!(
            "[[lv2]]\nversion = \"3.60-3.61\"\nerk = \"{}\"\nriv = \"{}\"\n",
            hex_of(0x44, 32),
            hex_of(0x55, 16)
        )
        .as_bytes(),
    )
    .expect("parse");
    assert_eq!(vault.keyset_count(crate::keys::SelfClass::Lv2), 1);
    assert!(vault.holds_any_key());
    assert!(!KeyVault::empty().holds_any_key());
}
