//! The completion pass over an entry the store already holds; the fresh
//! install's unpack is under `core_os_tests`.

use std::collections::BTreeMap;

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;
use crate::store::layout::ArtifactKind;
use crate::store::record::{
    ArtifactRecord, InstallRecord, KernelRecord, SourceRecord, INSTALL_RECORD_FORMAT_VERSION,
};
use crate::test_support::{
    build_core_os_image, build_pup, build_scepkg, build_tar, synthetic_vault,
};
use cellgov_ps3_abi::format::pup::{ENTRY_ID_UPDATE_FILES, ENTRY_ID_VERSION_TXT};

const KERNEL: &[u8] = b"SCE\0lv2 kernel bytes";
const VERSION: &str = "4.91";

/// A PUP that names `VERSION` and carries `core_os_entries` in its
/// CoreOS package, plus one dev_flash package the completion never
/// opens.
fn pup(keys: &KeyVault, core_os_entries: &[(&str, &[u8])]) -> Vec<u8> {
    let image = build_core_os_image(core_os_entries);
    let core = build_scepkg(keys, &image);
    let update_files = build_tar(&[
        ("dev_flash_000.tar", b"never opened by a completion"),
        ("CORE_OS_PACKAGE.pkg", core.as_slice()),
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

/// An installed entry: a tree with a `dev_flash/`, and a record that
/// names `pup_sha256` as its source.
fn install_entry(vfs: &Path, pup_sha256: manifest::Sha256) -> (PathBuf, PathBuf) {
    let layout = StoreLayout::new(vfs);
    let artifact = Artifact::Firmware {
        version: VersionKey::new(VERSION).unwrap(),
    };
    install_entry_at(vfs, &layout.entry_dir(&artifact), pup_sha256, None)
}

/// [`install_entry`] with the tree at `entry_dir`, which the record
/// under `VERSION` names, and the `[core_os]` block the record carries.
fn install_entry_at(
    vfs: &Path,
    entry_dir: &Path,
    pup_sha256: manifest::Sha256,
    core_os: Option<CoreOsRecord>,
) -> (PathBuf, PathBuf) {
    let layout = StoreLayout::new(vfs);
    let artifact = Artifact::Firmware {
        version: VersionKey::new(VERSION).unwrap(),
    };
    let entry_dir = entry_dir.to_path_buf();
    std::fs::create_dir_all(entry_dir.join("dev_flash/vsh/etc")).unwrap();
    std::fs::write(
        entry_dir.join("dev_flash/vsh/etc/version.txt"),
        b"release:04.9100:\n",
    )
    .unwrap();
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
        core_os,
    };
    let record_path = layout.record_path(&artifact);
    std::fs::create_dir_all(record_path.parent().unwrap()).unwrap();
    std::fs::write(&record_path, record.to_toml().unwrap()).unwrap();
    (entry_dir, record_path)
}

fn digest_of(bytes: &[u8]) -> manifest::Sha256 {
    manifest::Sha256(sha256_of(bytes))
}

#[test]
fn a_matching_pup_completes_the_entry_in_place_and_leaves_dev_flash_alone() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv0", b"SCE\0lv0"), ("lv2_kernel.self", KERNEL)]);
    let (entry_dir, record_path) = install_entry(&vfs, digest_of(&pup));
    let dev_flash_before = std::fs::read(entry_dir.join("dev_flash/vsh/etc/version.txt")).unwrap();

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("the entry completes");
    assert_eq!(outcome.version, VERSION);
    assert_eq!(outcome.entry_dir, entry_dir);
    assert_eq!(outcome.record_path, record_path);
    assert!(!outcome.replaced);
    let kernel = outcome.core_os.kernel.as_ref().expect("the kernel landed");
    assert_eq!(kernel.stored_sha256, digest_of(KERNEL));
    assert_eq!(
        std::fs::read(entry_dir.join("core_os/lv2_kernel.self")).unwrap(),
        KERNEL
    );
    assert_eq!(
        std::fs::read(entry_dir.join("dev_flash/vsh/etc/version.txt")).unwrap(),
        dev_flash_before
    );
    assert_eq!(
        std::fs::read_dir(entry_dir.join("dev_flash"))
            .unwrap()
            .count(),
        1,
        "nothing new under dev_flash"
    );

    let record = InstallRecord::parse(&std::fs::read_to_string(&record_path).unwrap())
        .expect("the rewritten record parses");
    assert_eq!(record.source.sha256, digest_of(&pup), "the source is kept");
    assert_eq!(record.core_os.and_then(|c| c.kernel), Some(kernel.clone()));

    // A second pass writes over the kernel and says so.
    let again = complete_kernel(&pup, &keys, &vfs, &()).expect("idempotent");
    assert!(again.replaced);
}

#[test]
fn a_version_that_is_not_installed_is_refused_and_nothing_is_written() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv2_kernel.self", KERNEL)]);
    let err = complete_kernel(&pup, &keys, &vfs, &()).unwrap_err();
    assert!(
        matches!(&err, FirmwareInstallError::NotInstalled { version } if version == VERSION),
        "{err}"
    );
    assert!(!vfs.join("firmware").exists());
}

#[test]
fn a_pup_other_than_the_one_that_installed_the_entry_is_refused_by_both_hashes() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv2_kernel.self", KERNEL)]);
    let (entry_dir, record_path) = install_entry(&vfs, digest_of(b"another PUP"));
    let record_before = std::fs::read_to_string(&record_path).unwrap();

    let err = complete_kernel(&pup, &keys, &vfs, &()).unwrap_err();
    let FirmwareInstallError::CompletionPupMismatch {
        installed,
        incoming,
        ..
    } = &err
    else {
        panic!("expected CompletionPupMismatch, got {err}");
    };
    assert_eq!(*installed, digest_of(b"another PUP"));
    assert_eq!(*incoming, digest_of(&pup));
    assert!(!entry_dir.join("core_os").exists(), "nothing was written");
    assert_eq!(
        std::fs::read_to_string(&record_path).unwrap(),
        record_before,
        "the record is untouched"
    );
}

#[test]
fn a_record_naming_another_versions_tree_is_refused_and_that_tree_is_not_written() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv2_kernel.self", KERNEL)]);
    // The record under 4.91 names 4.90's entry, which holds a tree.
    let foreign = StoreLayout::new(&vfs).firmware_root().join("4.90");
    let (entry_dir, record_path) = install_entry_at(&vfs, &foreign, digest_of(&pup), None);
    assert_eq!(entry_dir, foreign);
    let record_before = std::fs::read_to_string(&record_path).unwrap();

    let err = complete_kernel(&pup, &keys, &vfs, &()).unwrap_err();
    assert!(
        matches!(
            &err,
            FirmwareInstallError::RecordTreeForeign { version, store_path }
                if version == VERSION && store_path == "firmware/4.90"
        ),
        "{err}"
    );
    assert!(!foreign.join("core_os").exists(), "nothing was written");
    assert_eq!(
        std::fs::read_to_string(&record_path).unwrap(),
        record_before,
        "the record is untouched"
    );
}

#[test]
fn a_kernel_the_pass_could_not_write_over_is_not_reported_as_replaced() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv2_kernel.self", KERNEL)]);
    let entry_dir = StoreLayout::new(&vfs).entry_dir(&Artifact::Firmware {
        version: VersionKey::new(VERSION).unwrap(),
    });
    let held = CoreOsRecord {
        kernel: Some(KernelRecord {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: digest_of(b"SCE\0old"),
        }),
        omission: None,
        files: Vec::new(),
    };
    install_entry_at(&vfs, &entry_dir, digest_of(&pup), Some(held));
    // A directory under the kernel's name: no host renames a file over
    // it, so the rename fails and whatever was there stays.
    std::fs::create_dir_all(entry_dir.join("core_os/lv2_kernel.self")).unwrap();

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("an omission is not a refusal");
    assert!(
        outcome
            .core_os
            .omission
            .as_deref()
            .is_some_and(|o| o.starts_with("write ")),
        "{:?}",
        outcome.core_os
    );
    assert!(
        !outcome.replaced,
        "nothing was written over, so nothing was replaced"
    );
    assert!(entry_dir.join("core_os/lv2_kernel.self").is_dir());
}

#[test]
fn a_record_whose_tree_is_gone_is_refused_rather_than_completed_into_nothing() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv2_kernel.self", KERNEL)]);
    let (entry_dir, _) = install_entry(&vfs, digest_of(&pup));
    std::fs::remove_dir_all(&entry_dir).unwrap();

    let err = complete_kernel(&pup, &keys, &vfs, &()).unwrap_err();
    assert!(
        matches!(&err, FirmwareInstallError::EntryTreeAbsent { path, .. } if *path == entry_dir),
        "{err}"
    );
    assert!(!entry_dir.exists());
}

#[test]
fn a_pup_whose_package_names_no_kernel_completes_with_the_omission_recorded() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv0", b"SCE\0lv0")]);
    let (entry_dir, record_path) = install_entry(&vfs, digest_of(&pup));

    let outcome = complete_kernel(&pup, &keys, &vfs, &()).expect("an omission is not a refusal");
    assert!(outcome.core_os.kernel.is_none());
    assert_eq!(outcome.core_os.files.len(), 1);
    assert!(!entry_dir.join("core_os").exists());
    let record = InstallRecord::parse(&std::fs::read_to_string(record_path).unwrap()).unwrap();
    assert!(record
        .core_os
        .and_then(|c| c.omission)
        .is_some_and(|o| o.contains("names no lv2_kernel.self")));
}

#[test]
fn a_pup_that_names_no_version_is_refused_before_the_store_is_read() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let update_files = build_tar(&[("dev_flash_000.tar", b"x")]);
    let pup = build_pup(
        &keys,
        0x0004_9100_0000_0000,
        &[(ENTRY_ID_UPDATE_FILES, &update_files)],
    );
    let err = complete_kernel(&pup, &keys, &vfs, &()).unwrap_err();
    assert!(
        matches!(
            err,
            FirmwareInstallError::Pup(pup::PupError::NoEntry {
                entry_id: ENTRY_ID_VERSION_TXT
            })
        ),
        "{err}"
    );
    assert!(
        !vfs.exists(),
        "a refusal before the version is known touches nothing under the store"
    );
}

#[test]
fn a_held_version_refuses_the_completion() {
    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let pup = pup(&keys, &[("lv2_kernel.self", KERNEL)]);
    let (entry_dir, _) = install_entry(&vfs, digest_of(&pup));
    let _held = lock_artifact(
        &StoreLayout::new(&vfs),
        &Artifact::Firmware {
            version: VersionKey::new(VERSION).unwrap(),
        },
    )
    .expect("claim the version");
    let err = complete_kernel(&pup, &keys, &vfs, &()).unwrap_err();
    assert!(matches!(err, FirmwareInstallError::Locked(_)), "{err}");
    assert!(!entry_dir.join("core_os").exists());
}
