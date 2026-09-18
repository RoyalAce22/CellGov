//! What the kernel unpack writes, and each way it stops short, by
//! name. The omissions need no key; the package path runs under the
//! synthetic vault.

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;
use crate::test_support::build_core_os_image;

const KERNEL: &[u8] = b"SCE\0lv2 kernel bytes";

fn kernel_path(entry: &Path) -> PathBuf {
    entry.join(CORE_OS_DIR).join(LV2_KERNEL_SELF)
}

#[test]
fn the_kernel_lands_under_core_os_and_the_record_lists_the_whole_table() {
    let entry = scratch();
    let image = build_core_os_image(&[
        ("creserved_0", &[0xff; 4]),
        ("lv1.self", b"SCE\0lv1"),
        ("lv2_kernel.self", KERNEL),
        ("lv0", b"SCE\0lv0"),
    ]);
    let unpack = unpack_image(&image, &entry);
    let kernel = unpack.kernel.as_ref().expect("the kernel was written");
    assert_eq!(kernel.path, "core_os/lv2_kernel.self");
    assert_eq!(kernel.stored_sha256, manifest::Sha256(sha256_of(KERNEL)));
    assert_eq!(std::fs::read(kernel_path(&entry)).unwrap(), KERNEL);

    let names: Vec<(&str, u64)> = unpack
        .files
        .iter()
        .map(|f| (f.name.as_str(), f.size))
        .collect();
    assert_eq!(
        names,
        [
            ("creserved_0", 4),
            ("lv1.self", 7),
            ("lv2_kernel.self", KERNEL.len() as u64),
            ("lv0", 7),
        ]
    );
    // The unpack writes the kernel alone.
    let written: Vec<String> = std::fs::read_dir(entry.join(CORE_OS_DIR))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(written, [LV2_KERNEL_SELF]);

    let record = unpack.into_record();
    assert!(record.kernel.is_some() && record.omission.is_none());
    assert_eq!(record.files.len(), 4);
}

#[test]
fn a_table_naming_no_kernel_is_listed_and_nothing_is_written() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv0", b"SCE\0lv0"), ("lv1.self", b"SCE\0lv1")]);
    let unpack = unpack_image(&image, &entry);
    assert!(
        matches!(unpack.kernel, Err(CoreOsOmission::NoKernel { files: 2 })),
        "{:?}",
        unpack.kernel
    );
    assert_eq!(unpack.files.len(), 2, "the table is still reported");
    assert!(!entry.join(CORE_OS_DIR).exists());
    let record = unpack.into_record();
    assert!(record.kernel.is_none());
    assert!(record
        .omission
        .as_deref()
        .is_some_and(|o| o.contains("names no lv2_kernel.self")));
}

#[test]
fn a_malformed_table_is_an_omission_that_lists_nothing() {
    let entry = scratch();
    let unpack = unpack_image(b"not a coreos image", &entry);
    assert!(
        matches!(unpack.kernel, Err(CoreOsOmission::Table { .. })),
        "{:?}",
        unpack.kernel
    );
    assert!(unpack.files.is_empty());
    assert!(!entry.join(CORE_OS_DIR).exists());
}

#[test]
fn a_kernel_that_cannot_be_written_is_an_omission_naming_the_path() {
    let entry = scratch();
    // A file where the directory should be: every host refuses to
    // create the directory over it.
    std::fs::write(entry.join(CORE_OS_DIR), b"in the way").unwrap();
    let image = build_core_os_image(&[("lv2_kernel.self", KERNEL)]);
    let unpack = unpack_image(&image, &entry);
    let Err(CoreOsOmission::Write { path, .. }) = &unpack.kernel else {
        panic!("expected a write omission, got {:?}", unpack.kernel);
    };
    assert_eq!(*path, kernel_path(&entry));
    assert_eq!(unpack.files.len(), 1, "the table was read before the write");
}

#[test]
fn a_second_unpack_writes_over_the_kernel_it_finds() {
    let entry = scratch();
    unpack_image(
        &build_core_os_image(&[("lv2_kernel.self", b"SCE\0old")]),
        &entry,
    )
    .kernel
    .expect("first write");
    let second = unpack_image(&build_core_os_image(&[("lv2_kernel.self", KERNEL)]), &entry)
        .kernel
        .expect("second write");
    assert_eq!(std::fs::read(kernel_path(&entry)).unwrap(), KERNEL);
    assert_eq!(second.stored_sha256, manifest::Sha256(sha256_of(KERNEL)));
    assert!(
        !entry
            .join(CORE_OS_DIR)
            .join(".lv2_kernel.self.part")
            .exists(),
        "the part file is consumed by the rename"
    );
}

#[test]
fn the_package_is_matched_on_its_bare_name_under_any_prefix() {
    let outer = vec![
        tar::TarEntry {
            name: "dev_flash_000.tar".to_string(),
            data: Vec::new(),
        },
        tar::TarEntry {
            name: "update/CORE_OS_PACKAGE.pkg".to_string(),
            data: b"pkg".to_vec(),
        },
        tar::TarEntry {
            name: "NOT_CORE_OS_PACKAGE.pkg".to_string(),
            data: Vec::new(),
        },
    ];
    assert_eq!(
        find_package(&outer).map(|e| e.data.as_slice()),
        Some(&b"pkg"[..])
    );
    assert!(find_package(&outer[..1]).is_none());
    assert!(find_package(&outer[2..]).is_none());
}

#[cfg(feature = "decrypt")]
mod with_keys {
    use super::*;
    use crate::test_support::{build_scepkg, build_tar, synthetic_vault};

    fn outer(entries: &[(&str, &[u8])]) -> Vec<tar::TarEntry> {
        tar::parse(&build_tar(entries)).expect("a synthetic outer TAR")
    }

    #[test]
    fn an_outer_tar_without_the_package_records_no_package() {
        let entry = scratch();
        let unpack = unpack(
            &outer(&[("dev_flash_000.tar", b"x")]),
            &entry,
            &synthetic_vault(),
        );
        assert!(
            matches!(unpack.kernel, Err(CoreOsOmission::NoPackage)),
            "{:?}",
            unpack.kernel
        );
        assert!(unpack.files.is_empty());
        assert!(!entry.join(CORE_OS_DIR).exists());
    }

    #[test]
    fn a_package_that_does_not_open_records_the_decrypt_refusal() {
        let entry = scratch();
        let unpack = unpack(
            &outer(&[("CORE_OS_PACKAGE.pkg", b"not an SCE container")]),
            &entry,
            &synthetic_vault(),
        );
        assert!(
            matches!(unpack.kernel, Err(CoreOsOmission::Undecryptable { .. })),
            "{:?}",
            unpack.kernel
        );
        let record = unpack.into_record();
        assert!(record
            .omission
            .as_deref()
            .is_some_and(|o| o.starts_with("CORE_OS_PACKAGE.pkg: ")));
    }

    #[test]
    fn a_package_the_vault_opens_yields_its_kernel() {
        let entry = scratch();
        let keys = synthetic_vault();
        let image = build_core_os_image(&[("lv0", b"SCE\0lv0"), ("lv2_kernel.self", KERNEL)]);
        let package = build_scepkg(&keys, &image);
        let unpack = unpack(&outer(&[("CORE_OS_PACKAGE.pkg", &package)]), &entry, &keys);
        let kernel = unpack
            .kernel
            .expect("the package opens under the synthetic keyset");
        assert_eq!(kernel.stored_sha256, manifest::Sha256(sha256_of(KERNEL)));
        assert_eq!(std::fs::read(kernel_path(&entry)).unwrap(), KERNEL);
        assert_eq!(unpack.files.len(), 2);
    }

    #[test]
    fn a_full_install_stores_the_kernel_beside_the_tree_and_records_it() {
        use crate::firmware_install::install_pup;
        use crate::store::layout::{Artifact, StoreLayout, VersionKey};
        use crate::store::record::InstallRecord;
        use crate::test_support::build_pup;
        use cellgov_ps3_abi::format::pup::ENTRY_ID_UPDATE_FILES;

        let out = scratch();
        let vfs = out.join("vfs");
        let keys = synthetic_vault();
        let flash = build_scepkg(
            &keys,
            &build_tar(&[("dev_flash/vsh/etc/version.txt", b"release:04.9100:\n")]),
        );
        let image = build_core_os_image(&[("lv0", b"SCE\0lv0"), ("lv2_kernel.self", KERNEL)]);
        let core = build_scepkg(&keys, &image);
        let update_files = build_tar(&[
            ("dev_flash_000.tar", flash.as_slice()),
            ("CORE_OS_PACKAGE.pkg", core.as_slice()),
        ]);
        let pup = build_pup(
            &keys,
            0x0004_9100_0000_0000,
            &[(ENTRY_ID_UPDATE_FILES, &update_files)],
        );

        let outcome = install_pup(&pup, &keys, &vfs, false, &()).expect("the install commits");
        assert_eq!(outcome.version, "4.91");
        let kernel = outcome.core_os.kernel.as_ref().expect("the kernel landed");
        assert_eq!(kernel.stored_sha256, manifest::Sha256(sha256_of(KERNEL)));
        assert_eq!(
            std::fs::read(kernel_path(&outcome.entry_dir)).unwrap(),
            KERNEL
        );
        assert!(
            outcome
                .entry_dir
                .join("dev_flash/vsh/etc/version.txt")
                .is_file(),
            "the kernel sits beside dev_flash, not inside it"
        );
        assert!(!outcome
            .entry_dir
            .join("dev_flash")
            .join(CORE_OS_DIR)
            .exists());

        let layout = StoreLayout::new(&vfs);
        let record_path = layout.record_path(&Artifact::Firmware {
            version: VersionKey::new("4.91").unwrap(),
        });
        let record = InstallRecord::parse(&std::fs::read_to_string(record_path).unwrap())
            .expect("the written record parses");
        let block = record.core_os.expect("the record carries the block");
        assert_eq!(block.kernel.as_ref(), Some(kernel));
        assert_eq!(
            block
                .files
                .iter()
                .map(|f| f.name.as_str())
                .collect::<Vec<_>>(),
            ["lv0", "lv2_kernel.self"]
        );
    }

    #[test]
    fn a_pup_without_the_package_still_installs() {
        use crate::firmware_install::install_pup;
        use crate::test_support::build_pup;
        use cellgov_ps3_abi::format::pup::ENTRY_ID_UPDATE_FILES;

        let out = scratch();
        let keys = synthetic_vault();
        let flash = build_scepkg(
            &keys,
            &build_tar(&[("dev_flash/vsh/etc/version.txt", b"release:04.9100:\n")]),
        );
        let update_files = build_tar(&[("dev_flash_000.tar", flash.as_slice())]);
        let pup = build_pup(
            &keys,
            0x0004_9100_0000_0000,
            &[(ENTRY_ID_UPDATE_FILES, &update_files)],
        );
        let outcome =
            install_pup(&pup, &keys, &out.join("vfs"), false, &()).expect("the install commits");
        assert!(outcome.core_os.kernel.is_none());
        assert_eq!(
            outcome.core_os.omission.as_deref(),
            Some("update_files carries no CORE_OS_PACKAGE.pkg")
        );
        assert!(!outcome.entry_dir.join(CORE_OS_DIR).exists());
    }
}
