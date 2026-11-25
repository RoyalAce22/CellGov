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

fn block(lv1: Option<&[u8]>, lv1_omission: Option<&str>) -> CoreOsRecord {
    CoreOsRecord {
        kernel: Some(KernelRecord {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: manifest::Sha256(manifest::sha256_of(KERNEL)),
        }),
        omission: None,
        lv1: lv1.map(|bytes| KernelRecord {
            path: "core_os/lv1.self".to_string(),
            stored_sha256: manifest::Sha256(manifest::sha256_of(bytes)),
        }),
        lv1_omission: lv1_omission.map(str::to_string),
        files: Vec::new(),
    }
}

#[test]
fn a_hypervisor_the_install_could_not_write_is_named_with_its_reason() {
    let entry = entry_with_module_and_kernel();
    let omitted = block(None, Some("rename core_os/lv1.self: access denied"));
    let checked =
        verify_firmware_entry(&entry, Some(&omitted), &KeyVault::empty()).expect("verify");
    assert_eq!(
        checked.hypervisor_absence,
        Some(KernelAbsence::Omitted(Some(
            "rename core_os/lv1.self: access denied"
        )))
    );
    assert_eq!(checked.kernel_absence, None);
    assert_eq!(checked.report.matched, 2);
    assert!(
        checked.report.is_clean(),
        "{:?}",
        checked.report.divergences
    );
}

#[test]
fn a_block_without_hypervisor_fields_names_an_unchecked_hypervisor() {
    let entry = entry_with_module_and_kernel();
    let bare = block(None, None);
    let checked = verify_firmware_entry(&entry, Some(&bare), &KeyVault::empty()).expect("verify");
    assert_eq!(
        checked.hypervisor_absence,
        Some(KernelAbsence::Omitted(None))
    );
    assert_eq!(checked.report.matched, 2);
}

#[test]
fn a_record_without_the_block_names_both_selves_as_not_recorded() {
    let entry = entry_with_module_and_kernel();
    let checked = verify_firmware_entry(&entry, None, &KeyVault::empty()).expect("verify");
    assert_eq!(checked.kernel_absence, Some(KernelAbsence::NotRecorded));
    assert_eq!(checked.hypervisor_absence, Some(KernelAbsence::NotRecorded));
    assert_eq!(checked.report.matched, 1);
}

#[test]
fn a_stored_hypervisor_is_checked_when_the_entry_stores_no_kernel() {
    let entry = entry_with_module_and_kernel();
    std::fs::write(entry.join("core_os/lv1.self"), LV1).expect("write");
    let no_kernel = CoreOsRecord {
        kernel: None,
        omission: Some("names no lv2_kernel.self".to_string()),
        ..block(Some(LV1), None)
    };
    let checked =
        verify_firmware_entry(&entry, Some(&no_kernel), &KeyVault::empty()).expect("verify");
    assert_eq!(
        checked.kernel_absence,
        Some(KernelAbsence::Omitted(Some("names no lv2_kernel.self")))
    );
    assert_eq!(checked.hypervisor_absence, None);
    assert_eq!(checked.report.matched, 2);
    assert!(
        checked.report.is_clean(),
        "{:?}",
        checked.report.divergences
    );
}

#[test]
fn a_checked_hypervisor_carries_no_absence() {
    let entry = entry_with_module_and_kernel();
    std::fs::write(entry.join("core_os/lv1.self"), LV1).expect("write");
    let stored = block(Some(LV1), None);
    let checked = verify_firmware_entry(&entry, Some(&stored), &KeyVault::empty()).expect("verify");
    assert_eq!(checked.hypervisor_absence, None);
    assert_eq!(checked.report.matched, 3);
}
