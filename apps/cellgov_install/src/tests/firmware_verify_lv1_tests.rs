//! The entry pass holds the stored hypervisor to the digest its record
//! carries, beside the kernel and the modules.

use super::*;

use crate::scratch_dir::{scratch, ScratchDir};

const KERNEL: &[u8] = b"stored kernel";
const LV1: &[u8] = b"stored hypervisor";

/// A firmware entry with one plaintext module under `dev_flash/` and
/// its `firmware.toml`, and the kernel stored under `core_os/`.
fn entry_with_module_and_kernel() -> ScratchDir {
    let entry = scratch();
    let dev_flash = entry.join(FLASH_MOUNT);
    let mut module = cellgov_ps3_abi::format::elf::ELF_MAGIC.to_vec();
    module.extend_from_slice(b"lv2");
    std::fs::create_dir_all(dev_flash.join("sys/external")).expect("create dev_flash");
    std::fs::write(dev_flash.join("sys/external/liblv2.prx"), &module).expect("write");
    let manifest_text = format!(
        "format_version = 2\n\
         [firmware]\n\
         image_version = \"0x0004008200000000\"\n\
         version = \"4.91\"\n\
         pup_sha256 = \"{}\"\n\
         [[files]]\n\
         path = \"sys/external/liblv2.prx\"\n\
         sha256 = \"{}\"\n\
         revision = 1\n",
        manifest::Sha256(manifest::sha256_of(b"pup")).to_hex(),
        manifest::Sha256(manifest::sha256_of(&module)).to_hex(),
    );
    std::fs::write(dev_flash.join(MANIFEST_FILE), manifest_text).expect("write firmware.toml");
    std::fs::create_dir_all(entry.join("core_os")).expect("create core_os");
    std::fs::write(entry.join("core_os/lv2_kernel.self"), KERNEL).expect("write");
    entry
}

fn block_recording_lv1(recorded: &[u8]) -> CoreOsRecord {
    CoreOsRecord {
        kernel: Some(KernelRecord {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: manifest::Sha256(manifest::sha256_of(KERNEL)),
        }),
        omission: None,
        lv1: Some(KernelRecord {
            path: "core_os/lv1.self".to_string(),
            stored_sha256: manifest::Sha256(manifest::sha256_of(recorded)),
        }),
        lv1_omission: None,
        files: Vec::new(),
    }
}

#[test]
fn an_intact_stored_hypervisor_counts_with_the_kernel_and_the_modules() {
    let entry = entry_with_module_and_kernel();
    std::fs::write(entry.join("core_os/lv1.self"), LV1).expect("write");
    let block = block_recording_lv1(LV1);
    let checked = verify_firmware_entry(&entry, Some(&block), &KeyVault::empty()).expect("verify");
    assert!(
        checked.report.is_clean(),
        "{:?}",
        checked.report.divergences
    );
    assert_eq!(checked.report.matched, 3);
}

#[test]
fn a_rewritten_stored_hypervisor_diverges_by_its_own_path() {
    let entry = entry_with_module_and_kernel();
    std::fs::write(entry.join("core_os/lv1.self"), b"rewritten").expect("write");
    let block = block_recording_lv1(LV1);
    let checked = verify_firmware_entry(&entry, Some(&block), &KeyVault::empty()).expect("verify");
    assert_eq!(checked.report.matched, 2);
    let [only] = checked.report.divergences.as_slice() else {
        panic!(
            "expected one divergence, got {:?}",
            checked.report.divergences
        );
    };
    assert_eq!(only.path, entry.join("core_os").join("lv1.self"));
    assert!(
        matches!(only.kind, ModuleDivergence::Modified { .. }),
        "{:?}",
        only.kind
    );
}

#[test]
fn a_recorded_hypervisor_the_entry_no_longer_holds_is_missing() {
    let entry = entry_with_module_and_kernel();
    let block = block_recording_lv1(LV1);
    let checked = verify_firmware_entry(&entry, Some(&block), &KeyVault::empty()).expect("verify");
    assert_eq!(checked.report.matched, 2);
    let [only] = checked.report.divergences.as_slice() else {
        panic!(
            "expected one divergence, got {:?}",
            checked.report.divergences
        );
    };
    assert_eq!(only.path, entry.join("core_os").join("lv1.self"));
    assert!(
        matches!(only.kind, ModuleDivergence::Missing),
        "{:?}",
        only.kind
    );
}

#[test]
fn a_vault_with_no_lv1_keyset_is_a_vault_gap() {
    let gap = crate::sce::SceError::NoLv1Key {
        version: 0x0004_0093_0000_0000,
    };
    assert!(is_vault_gap(&gap), "{gap}");
}
