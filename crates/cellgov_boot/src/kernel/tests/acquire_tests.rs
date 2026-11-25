//! Each reason a boot has no kernel of record.

use std::path::{Path, PathBuf};

use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_install::manifest::Sha256;
use cellgov_install::sce::SceError;
use cellgov_install::self_image::{into_plaintext_elf, KeyPolicy};
use cellgov_install::store::inventory::FirmwareEntry;
use cellgov_install::store::select::{FirmwareSelectedBy, ManagedFirmware};
use cellgov_install::store::{CoreOsRecord, KernelRecord};
use cellgov_ps3_abi::format::sce::{self_version, SELF_PROGRAM_TYPE_LV2};

use super::{kernel_of_record, KernelMapError, KernelUnavailable};
use crate::compose::FirmwareChoice;
use crate::KeyVaultSource;

const FW: &str = "4.93";
const KERNEL_PATH: &str = "core_os/lv2_kernel.self";

struct Vault(Result<KeyVault, KeyVaultError>);

impl KeyVaultSource for Vault {
    fn vault_for(&self, _bytes: &[u8]) -> Result<&KeyVault, &KeyVaultError> {
        self.0.as_ref()
    }
}

fn empty_vault() -> Vault {
    Vault(Ok(KeyVault::empty()))
}

fn managed(entry_dir: &Path, core_os: Option<CoreOsRecord>) -> FirmwareChoice {
    FirmwareChoice::Managed(ManagedFirmware {
        entry: FirmwareEntry {
            version: FW.to_string(),
            entry_dir: entry_dir.to_path_buf(),
            pup_sha256: "0".repeat(64),
            core_os,
        },
        selected_by: FirmwareSelectedBy::Named,
    })
}

fn stored() -> Option<CoreOsRecord> {
    Some(CoreOsRecord {
        kernel: Some(KernelRecord {
            path: KERNEL_PATH.to_string(),
            stored_sha256: Sha256([0; 32]),
        }),
        omission: None,
        files: Vec::new(),
    })
}

/// A 0x100-byte SCE container that names an LV2 kernel of firmware
/// 4.93, with an envelope of zeros.
fn lv2_container() -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[8..10].copy_from_slice(&0x0002u16.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes());
    data[16..24].copy_from_slice(&0x100u64.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&0xC0u64.to_be_bytes());
    data[0xC0 + 0x0C..0xC0 + 0x10].copy_from_slice(&SELF_PROGRAM_TYPE_LV2.to_be_bytes());
    data[0xC0 + 0x10..0xC0 + 0x18].copy_from_slice(&self_version(4, 0x93).to_be_bytes());
    data
}

fn entry_storing(bytes: &[u8], label: &str) -> cellgov_testkit::scratch::ScratchDir {
    let entry = cellgov_testkit::scratch::scratch_labeled(label);
    std::fs::create_dir_all(entry.join("core_os")).unwrap();
    std::fs::write(entry.join(KERNEL_PATH), bytes).unwrap();
    entry
}

#[test]
fn a_boot_with_no_firmware_or_an_unmanaged_tree_has_no_kernel() {
    let err = kernel_of_record(&FirmwareChoice::None, &empty_vault()).unwrap_err();
    assert!(matches!(err, KernelUnavailable::NoFirmware), "{err:?}");
    let dir = PathBuf::from("raw/firmware");
    let unmanaged = FirmwareChoice::Unmanaged { dir: dir.clone() };
    let err = kernel_of_record(&unmanaged, &empty_vault()).unwrap_err();
    assert!(
        matches!(&err, KernelUnavailable::UnmanagedFirmware { dir: d } if *d == dir),
        "{err:?}"
    );
}

#[test]
fn an_entry_that_stores_no_kernel_says_whether_its_record_predates_kernels() {
    let dir = PathBuf::from("store/firmware/4.93");
    let err = kernel_of_record(&managed(&dir, None), &empty_vault()).unwrap_err();
    assert!(
        matches!(&err, KernelUnavailable::NotRecorded { firmware } if firmware == FW),
        "{err:?}"
    );
    let omitted = CoreOsRecord {
        kernel: None,
        omission: Some("the package held no kernel".to_string()),
        files: Vec::new(),
    };
    let err = kernel_of_record(&managed(&dir, Some(omitted)), &empty_vault()).unwrap_err();
    assert!(
        matches!(
            &err,
            KernelUnavailable::Omitted { firmware, reason: Some(r) }
                if firmware == FW && r == "the package held no kernel"
        ),
        "{err:?}"
    );
}

#[test]
fn a_recorded_kernel_missing_from_the_entry_is_a_read_refusal_naming_the_file() {
    let entry = cellgov_testkit::scratch::scratch_labeled("kernel_missing");
    let err = kernel_of_record(&managed(&entry, stored()), &empty_vault()).unwrap_err();
    assert!(
        matches!(&err, KernelUnavailable::Read { firmware, path, .. }
            if firmware == FW && *path == entry.join("core_os").join("lv2_kernel.self")),
        "{err:?}"
    );
}

#[test]
fn a_vault_that_did_not_load_is_named() {
    let entry = entry_storing(&lv2_container(), "kernel_no_vault");
    let vault = Vault(Err(KeyVaultError::MissingScepkg));
    let err = kernel_of_record(&managed(&entry, stored()), &vault).unwrap_err();
    assert!(
        matches!(&err, KernelUnavailable::Vault { firmware, reason }
            if firmware == FW && *reason == KeyVaultError::MissingScepkg.to_string()),
        "{err:?}"
    );
}

/// Whether this test binary's `cellgov_install` lacks the `decrypt`
/// feature, as the library itself answers.
fn decrypt_disabled() -> bool {
    matches!(
        into_plaintext_elf(lv2_container(), &KeyVault::empty(), KeyPolicy::AppOnly),
        Err(SceError::DecryptFeatureDisabled)
    )
}

#[test]
fn an_empty_vault_is_a_missing_lv2_key_or_a_build_that_cannot_decrypt() {
    let entry = entry_storing(&lv2_container(), "kernel_no_key");
    let err = kernel_of_record(&managed(&entry, stored()), &empty_vault()).unwrap_err();
    if decrypt_disabled() {
        assert!(
            matches!(&err, KernelUnavailable::DecryptDisabled { firmware } if firmware == FW),
            "{err:?}"
        );
    } else {
        assert!(
            matches!(&err, KernelUnavailable::NoKey { firmware, source: SceError::NoLv2Key { .. } }
                if firmware == FW),
            "{err:?}"
        );
    }
}

#[test]
fn an_lv2_keyset_that_does_not_open_the_envelope_is_a_missing_key_not_a_failed_decrypt() {
    let entry = entry_storing(&lv2_container(), "kernel_wrong_key");
    let toml = format!(
        "[[lv2]]\nversion = \"4.20-4.93\"\nerk = \"{}\"\nriv = \"{}\"\n",
        "73".repeat(32),
        "73".repeat(16)
    );
    let vault = KeyVault::parse(Path::new("lv2.toml"), toml.as_bytes()).unwrap();
    let err = kernel_of_record(&managed(&entry, stored()), &Vault(Ok(vault))).unwrap_err();
    if decrypt_disabled() {
        assert!(
            matches!(&err, KernelUnavailable::DecryptDisabled { firmware } if firmware == FW),
            "{err:?}"
        );
    } else {
        assert!(
            matches!(&err, KernelUnavailable::NoKey { firmware, source: SceError::KeyEnvelopePadding }
                if firmware == FW),
            "{err:?}"
        );
    }
}

#[test]
fn a_truncated_container_fails_the_decrypt_and_plaintext_that_is_no_elf_fails_the_map() {
    let mut truncated = lv2_container();
    truncated.truncate(0x40);
    let entry = entry_storing(&truncated, "kernel_bad_container");
    let err = kernel_of_record(&managed(&entry, stored()), &empty_vault()).unwrap_err();
    if decrypt_disabled() {
        assert!(
            matches!(&err, KernelUnavailable::DecryptDisabled { firmware } if firmware == FW),
            "{err:?}"
        );
    } else {
        assert!(
            matches!(&err, KernelUnavailable::Decrypt { firmware, source: SceError::TooSmall { .. } }
                if firmware == FW),
            "{err:?}"
        );
    }
    let entry = entry_storing(b"not an SCE container", "kernel_not_elf");
    let err = kernel_of_record(&managed(&entry, stored()), &empty_vault()).unwrap_err();
    assert!(
        matches!(&err, KernelUnavailable::Map { firmware, source: KernelMapError::Elf { .. } }
            if firmware == FW),
        "{err:?}"
    );
}
