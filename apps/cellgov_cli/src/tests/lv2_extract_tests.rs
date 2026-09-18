use super::*;

use cellgov_install::store::CoreOsRecord;
use cellgov_ps3_abi::format::sce::self_version;
use cellgov_testkit::scratch::scratch_labeled;

use crate::composition::test_support::SyntheticStore;

fn entry(core_os: Option<CoreOsRecord>) -> FirmwareEntry {
    FirmwareEntry {
        version: "3.60".to_string(),
        entry_dir: PathBuf::from("firmware/3.60"),
        pup_sha256: "00".repeat(32),
        core_os,
    }
}

#[test]
fn an_uninstalled_named_firmware_is_refused_by_name() {
    let root = scratch_labeled("lv2_extract_uninstalled");
    let args = Lv2ExtractArgs {
        fw: Some("3.60".to_string()),
        output_dir: root.join("out"),
    };
    let error = extract(&args, &root.join("dev_hdd0")).expect_err("3.60 is not installed");
    let text = error.to_string();
    assert!(text.contains("--fw \"3.60\" is not installed"), "{text}");
}

#[test]
fn an_empty_store_names_the_install_command_without_boot_only_advice() {
    let store = SyntheticStore::new("lv2_extract_empty");
    let args = Lv2ExtractArgs {
        fw: None,
        output_dir: store.root().join("out"),
    };
    let error = extract(&args, &store.root().join("dev_hdd0"))
        .expect_err("an empty store has no kernel to extract");
    let text = error.to_string();
    assert!(text.contains("no firmware is installed"), "{text}");
    assert!(text.contains("cellgov firmware install"), "{text}");
    assert!(!text.contains("--firmware-dir"), "{text}");
    assert!(!text.contains(DISABLE_DEFAULT_ENV), "{text}");
}

#[test]
fn an_ambiguous_store_names_extraction_and_the_candidates() {
    let store = SyntheticStore::new("lv2_extract_ambiguous");
    store.add_firmware("3.55", true);
    store.add_firmware("4.93", true);
    let args = Lv2ExtractArgs {
        fw: None,
        output_dir: store.root().join("out"),
    };
    let error = extract(&args, &store.root().join("dev_hdd0"))
        .expect_err("several installed kernels require --fw");
    let text = error.to_string();
    assert!(text.contains("3.55, 4.93"), "{text}");
    assert!(text.contains("name the one to extract with --fw"), "{text}");
    assert!(!text.contains("boot against"), "{text}");
}

#[test]
fn an_entry_without_a_stored_kernel_carries_its_reason() {
    let error = kernel_record(&entry(Some(CoreOsRecord {
        kernel: None,
        omission: Some("update_files carries no CoreOS package".to_string()),
        files: Vec::new(),
    })))
    .expect_err("the entry stores no kernel");
    let text = error.to_string();
    assert!(
        text.contains("firmware 3.60: kernel not unpacked"),
        "{text}"
    );
    assert!(
        text.contains("update_files carries no CoreOS package"),
        "{text}"
    );
}

#[test]
fn an_entry_from_before_stored_kernels_is_refused_by_version() {
    let error = kernel_record(&entry(None)).expect_err("the old entry stores no kernel");
    let text = error.to_string();
    assert!(
        text.contains("firmware 3.60: kernel not unpacked"),
        "{text}"
    );
    assert!(text.contains("predates stored kernels"), "{text}");
}

#[test]
fn a_decrypted_kernel_is_written_and_described_as_json() {
    let root = scratch_labeled("lv2_extract_output");
    let output_dir = root.join("outside-repository");
    let source = root.join("firmware/3.60/core_os/lv2_kernel.self");
    let elf = b"\x7fELFsynthetic".to_vec();
    let doc = write_output(
        &output_dir,
        "3.60",
        &source,
        DecryptedKernel {
            elf: elf.clone(),
            version: self_version(4, 0x93),
        },
    )
    .expect("write plaintext kernel");

    let output = output_dir.join("lv2_kernel-3.60.elf");
    assert_eq!(std::fs::read(&output).expect("read output"), elf);
    assert_eq!(doc.kernel_version, "4.93");
    assert_eq!(doc.output, output.display().to_string());
    let json = serde_json::to_value(&doc).expect("serialize report");
    assert_eq!(json["firmware"], "3.60");
    assert_eq!(json["kernel_version"], "4.93");
    assert_eq!(json["elf_bytes"], 13);
    assert_eq!(
        json["elf_sha256"],
        "ff608d6fd9470beb265da3f143342e761421194b25b170e595df51691eb298d8"
    );
}
