use super::*;

use crate::cli::store::read::model::STORE_FORMAT_VERSION;

fn entry_doc(kernel: Option<&str>, hypervisor: Option<&str>) -> VerifiedEntryDoc {
    VerifiedEntryDoc {
        entry: "4.93".to_string(),
        matched: 371,
        divergences: Vec::new(),
        kernel_omission: kernel.map(str::to_string),
        hypervisor_omission: hypervisor.map(str::to_string),
    }
}

fn doc(entry: VerifiedEntryDoc) -> VerifyDoc {
    VerifyDoc {
        format_version: STORE_FORMAT_VERSION,
        store: "vfs".to_string(),
        subject: "4.93".to_string(),
        clean: entry.divergences.is_empty(),
        entries: vec![entry],
    }
}

#[test]
fn a_hypervisor_the_entry_does_not_store_gets_an_unchecked_line_and_no_divergence() {
    let d = doc(entry_doc(
        None,
        Some("rename core_os/lv1.self: access denied"),
    ));
    assert!(d.clean);
    let rendered = render(&d, "firmware 4.93");
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(
        lines,
        [
            "4.93: hypervisor not checked: rename core_os/lv1.self: access denied",
            "firmware 4.93: 371 artefact(s) match their record",
        ]
    );
}

#[test]
fn an_entry_missing_both_selves_names_the_kernel_then_the_hypervisor() {
    let rendered = render_entry(&entry_doc(
        Some(KERNEL_NOT_RECORDED),
        Some(HYPERVISOR_NOT_RECORDED),
    ));
    let lines: Vec<&str> = rendered.lines().collect();
    assert_eq!(
        lines,
        [
            format!("4.93: kernel not checked: {KERNEL_NOT_RECORDED}"),
            format!("4.93: hypervisor not checked: {HYPERVISOR_NOT_RECORDED}"),
        ]
    );
}

#[test]
fn the_json_entry_carries_the_hypervisor_omission_only_when_there_is_one() {
    let omitted = serde_json::to_value(entry_doc(None, Some("no space"))).expect("serialize");
    assert_eq!(omitted["hypervisor_omission"], "no space");
    let checked = serde_json::to_value(entry_doc(None, None)).expect("serialize");
    assert!(checked.get("hypervisor_omission").is_none(), "{checked}");
}

#[test]
fn a_record_without_the_block_names_the_hypervisor_and_the_kernel_only_install() {
    assert_eq!(
        HYPERVISOR_NOT_RECORDED,
        "not unpacked (installed before the hypervisor was kept; add it with \
         `cellgov firmware install <PUP> --kernel-only`, giving the PUP this entry was \
         installed from, whose sha256 `cellgov firmware show <VERSION>` prints)"
    );
}

#[test]
fn an_omission_that_names_no_reason_prints_not_unpacked() {
    assert_eq!(
        omission_text(KernelAbsence::Omitted(None), HYPERVISOR_NOT_RECORDED),
        "not unpacked"
    );
}

#[cfg(feature = "decrypt")]
mod with_decrypt {
    use super::*;

    use std::path::PathBuf;

    use cellgov_install::manifest::{sha256_of, Sha256, MANIFEST_FILE};
    use cellgov_install::store::{CoreOsRecord, KernelRecord};
    use cellgov_ps3_abi::format::dev_flash::FLASH_MOUNT;
    use cellgov_testkit::scratch::{scratch_labeled, ScratchDir};

    const KERNEL: &[u8] = b"stored kernel";

    fn view() -> StoreView {
        StoreView {
            root: PathBuf::from("vfs"),
            inventory: cellgov_install::store::inventory::StoreInventory::read(Path::new(
                "no-such-store-root",
            ))
            .expect("an absent root reads as an empty store"),
            registry: cellgov_boot::manifest::TitleRegistry::default(),
            fixtures: PathBuf::from("no-such-fixtures"),
        }
    }

    /// A firmware entry with one plaintext module under `dev_flash/`,
    /// its `firmware.toml`, and the kernel stored under `core_os/`.
    fn entry_dir() -> ScratchDir {
        let entry = scratch_labeled("verify-hypervisor-omission");
        let dev_flash = entry.join(FLASH_MOUNT);
        let mut module = cellgov_ps3_abi::format::elf::ELF_MAGIC.to_vec();
        module.extend_from_slice(b"lv2");
        std::fs::create_dir_all(dev_flash.join("sys/external")).expect("create dev_flash");
        std::fs::write(dev_flash.join("sys/external/liblv2.prx"), &module).expect("write");
        let manifest_text = format!(
            "format_version = 2\n\
             [firmware]\n\
             image_version = \"0x0004008200000000\"\n\
             version = \"4.93\"\n\
             pup_sha256 = \"{}\"\n\
             [[files]]\n\
             path = \"sys/external/liblv2.prx\"\n\
             sha256 = \"{}\"\n\
             revision = 1\n",
            Sha256(sha256_of(b"pup")).to_hex(),
            Sha256(sha256_of(&module)).to_hex(),
        );
        std::fs::write(dev_flash.join(MANIFEST_FILE), manifest_text).expect("write firmware.toml");
        std::fs::create_dir_all(entry.join("core_os")).expect("create core_os");
        std::fs::write(entry.join("core_os/lv2_kernel.self"), KERNEL).expect("write");
        entry
    }

    fn firmware_entry(dir: &Path, core_os: Option<CoreOsRecord>) -> FirmwareEntry {
        FirmwareEntry {
            version: "4.93".to_string(),
            entry_dir: dir.to_path_buf(),
            pup_sha256: Sha256(sha256_of(b"pup")).to_hex(),
            core_os,
        }
    }

    #[test]
    fn an_unwritten_hypervisor_reaches_the_verify_document_with_its_reason() {
        let dir = entry_dir();
        let block = CoreOsRecord {
            kernel: Some(KernelRecord {
                path: "core_os/lv2_kernel.self".to_string(),
                stored_sha256: Sha256(sha256_of(KERNEL)),
            }),
            omission: None,
            lv1: None,
            lv1_omission: Some("rename core_os/lv1.self: access denied".to_string()),
            files: Vec::new(),
        };
        let entry = firmware_entry(&dir, Some(block));
        let checked = firmware_entry_doc(&view(), &entry, &KeyVault::empty()).expect("verify");
        assert_eq!(
            checked.hypervisor_omission.as_deref(),
            Some("rename core_os/lv1.self: access denied")
        );
        assert_eq!(checked.kernel_omission, None);
        assert_eq!(checked.matched, 2);
        assert!(checked.divergences.is_empty());
    }

    #[test]
    fn a_record_without_the_block_names_the_hypervisor_as_not_recorded() {
        let dir = entry_dir();
        let entry = firmware_entry(&dir, None);
        let checked = firmware_entry_doc(&view(), &entry, &KeyVault::empty()).expect("verify");
        assert_eq!(
            checked.hypervisor_omission.as_deref(),
            Some(HYPERVISOR_NOT_RECORDED)
        );
        assert_eq!(
            checked.kernel_omission.as_deref(),
            Some(KERNEL_NOT_RECORDED)
        );
    }
}
