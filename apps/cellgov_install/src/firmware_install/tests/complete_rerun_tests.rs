//! A completion pass run again on an entry that already stores the
//! kernel and the hypervisor.

use std::collections::BTreeMap;

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;
use crate::store::layout::ArtifactKind;
use crate::store::record::{
    stored_hypervisor, ArtifactRecord, InstallRecord, SourceRecord, INSTALL_RECORD_FORMAT_VERSION,
};
use crate::test_support::{
    build_core_os_image, build_pup, build_scepkg, build_tar, synthetic_vault,
};
use cellgov_ps3_abi::format::pup::{ENTRY_ID_UPDATE_FILES, ENTRY_ID_VERSION_TXT};

const VERSION: &str = "4.91";
const HELD_KERNEL: &[u8] = b"SCE\0held lv2 kernel bytes";
const HELD_LV1: &[u8] = b"SCE\0held lv1 hypervisor bytes";
const NEW_KERNEL: &[u8] = b"SCE\0new lv2 kernel bytes";
const NEW_LV1: &[u8] = b"SCE\0new lv1 hypervisor bytes";

fn digest_of(bytes: &[u8]) -> manifest::Sha256 {
    manifest::Sha256(sha256_of(bytes))
}

/// A PUP that names `VERSION` and carries `core_os_package` as its
/// CoreOS package.
fn pup_with_package(keys: &KeyVault, core_os_package: &[u8]) -> Vec<u8> {
    let update_files = build_tar(&[
        ("dev_flash_000.tar", b"never opened by a completion"),
        ("CORE_OS_PACKAGE.pkg", core_os_package),
    ]);
    build_pup(
        keys,
        0x0004_9100_0000_0000,
        &[
            (ENTRY_ID_VERSION_TXT, b"4.91\n"),
            (ENTRY_ID_UPDATE_FILES, &update_files),
        ],
    )
}

/// A PUP whose CoreOS package opens and holds `entries`.
fn pup(keys: &KeyVault, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let package = build_scepkg(keys, &build_core_os_image(entries));
    pup_with_package(keys, &package)
}

fn held_block() -> CoreOsRecord {
    CoreOsRecord {
        kernel: Some(KernelRecord {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: digest_of(HELD_KERNEL),
        }),
        omission: None,
        lv1: Some(KernelRecord {
            path: "core_os/lv1.self".to_string(),
            stored_sha256: digest_of(HELD_LV1),
        }),
        lv1_omission: None,
        files: Vec::new(),
    }
}

/// An entry installed from `pup_sha256` whose record and `core_os/`
/// already hold the kernel and the hypervisor of an earlier pass.
fn entry_holding_both(vfs: &Path, pup_sha256: manifest::Sha256) -> (PathBuf, PathBuf) {
    let layout = StoreLayout::new(vfs);
    let artifact = Artifact::Firmware {
        version: VersionKey::new(VERSION).unwrap(),
    };
    let entry_dir = layout.entry_dir(&artifact);
    std::fs::create_dir_all(entry_dir.join("dev_flash/vsh/etc")).unwrap();
    std::fs::write(
        entry_dir.join("dev_flash/vsh/etc/version.txt"),
        b"release:04.9100:\n",
    )
    .unwrap();
    std::fs::create_dir_all(entry_dir.join("core_os")).unwrap();
    std::fs::write(entry_dir.join("core_os/lv2_kernel.self"), HELD_KERNEL).unwrap();
    std::fs::write(entry_dir.join("core_os/lv1.self"), HELD_LV1).unwrap();
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: VERSION.to_string(),
            store_path: layout.store_path_of(&entry_dir).unwrap(),
        },
        source: SourceRecord::local("pup", pup_sha256),
        title: None,
        files: BTreeMap::new(),
        rap: None,
        core_os: Some(held_block()),
    };
    let record_path = layout.record_path(&artifact);
    std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    std::fs::write(&record_path, record.to_toml().unwrap()).unwrap();
    (entry_dir, record_path)
}

fn record_at(path: &Path) -> InstallRecord {
    InstallRecord::parse(&std::fs::read_to_string(path).unwrap()).expect("the record parses")
}

#[test]
fn a_rerun_whose_core_os_package_does_not_open_keeps_both_stored_selves_in_the_record() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup_with_package(&keys, b"not an SCE package");
    let (entry_dir, record_path) = entry_holding_both(&vfs, digest_of(&pup));

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("an omission is not a refusal");
    for (what, completion) in [("kernel", &outcome.kernel), ("hypervisor", &outcome.lv1)] {
        let SelfCompletion::Kept { reason } = completion else {
            panic!("the held {what} is kept: {completion:?}");
        };
        assert!(reason.starts_with("CORE_OS_PACKAGE.pkg: "), "{reason}");
    }
    let record = record_at(&record_path);
    assert_eq!(record.core_os.as_ref(), Some(&held_block()));
    assert_eq!(
        stored_hypervisor(record.core_os.as_ref()),
        Ok(held_block().lv1.as_ref().unwrap()),
        "verify still holds the hypervisor against its digest"
    );
    assert_eq!(
        std::fs::read(entry_dir.join("core_os/lv1.self")).unwrap(),
        HELD_LV1
    );
}

#[test]
fn a_rerun_that_writes_the_hypervisor_over_a_stored_one_reports_it_replaced() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(
        &keys,
        &[("lv1.self", NEW_LV1), ("lv2_kernel.self", NEW_KERNEL)],
    );
    let (entry_dir, record_path) = entry_holding_both(&vfs, digest_of(&pup));

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("the entry completes");
    assert_eq!(outcome.lv1, SelfCompletion::Replaced);
    assert_eq!(outcome.kernel, SelfCompletion::Replaced);
    assert_eq!(
        std::fs::read(entry_dir.join("core_os/lv1.self")).unwrap(),
        NEW_LV1
    );
    let record = record_at(&record_path);
    assert_eq!(
        record.core_os.and_then(|c| c.lv1).map(|l| l.stored_sha256),
        Some(digest_of(NEW_LV1))
    );
}

#[test]
fn a_rerun_that_cannot_write_the_hypervisor_keeps_the_stored_one_and_its_digest() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(
        &keys,
        &[("lv1.self", NEW_LV1), ("lv2_kernel.self", NEW_KERNEL)],
    );
    let (entry_dir, record_path) = entry_holding_both(&vfs, digest_of(&pup));
    // A directory under the staging name: the write fails before the
    // rename, so the pass leaves the stored lv1.self as it is.
    std::fs::create_dir_all(entry_dir.join("core_os/.lv1.self.part")).unwrap();

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("an omission is not a refusal");
    assert_eq!(outcome.kernel, SelfCompletion::Replaced);
    let SelfCompletion::Kept { reason } = &outcome.lv1 else {
        panic!("the held hypervisor is kept: {:?}", outcome.lv1);
    };
    assert!(reason.starts_with("write "), "{reason}");
    let block = record_at(&record_path)
        .core_os
        .expect("the record carries the block");
    assert_eq!(block.lv1, held_block().lv1);
    assert_eq!(block.lv1_omission, None);
    assert_eq!(block.files.len(), 2, "the table this pass read");
    assert_eq!(
        block.kernel.map(|k| k.stored_sha256),
        Some(digest_of(NEW_KERNEL))
    );
    let on_disk = std::fs::read(entry_dir.join("core_os/lv1.self")).unwrap();
    assert_eq!(
        Some(digest_of(&on_disk)),
        block.lv1.map(|l| l.stored_sha256),
        "the recorded digest is the digest of the bytes on disk"
    );
}

#[test]
fn a_rerun_whose_table_names_the_hypervisor_but_no_kernel_keeps_the_stored_kernel() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv0", b"SCE\0lv0"), ("lv1.self", NEW_LV1)]);
    let (entry_dir, record_path) = entry_holding_both(&vfs, digest_of(&pup));

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("an omission is not a refusal");
    assert_eq!(
        outcome.kernel,
        SelfCompletion::Kept {
            reason: "CORE_OS_PACKAGE.pkg names no lv2_kernel.self among its 2 file(s)".to_string()
        }
    );
    assert_eq!(outcome.lv1, SelfCompletion::Replaced);
    let record = record_at(&record_path);
    let block = record.core_os.expect("the record carries the block");
    assert_eq!(block.kernel, held_block().kernel);
    assert_eq!(block.omission, None);
    assert_eq!(
        std::fs::read(entry_dir.join("core_os/lv2_kernel.self")).unwrap(),
        HELD_KERNEL
    );
}
