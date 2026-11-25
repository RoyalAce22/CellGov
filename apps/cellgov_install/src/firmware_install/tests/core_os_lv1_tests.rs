//! The hypervisor SELF the CoreOS unpack keeps beside the kernel.

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;
use crate::test_support::build_core_os_image;

const KERNEL: &[u8] = b"SCE\0lv2 kernel bytes";
const LV1: &[u8] = b"SCE\0lv1 hypervisor bytes";

fn lv1_path(entry: &Path) -> PathBuf {
    entry.join(CORE_OS_DIR).join(LV1_SELF)
}

#[test]
fn the_hypervisor_lands_beside_the_kernel_as_shipped() {
    let entry = scratch();
    let image = build_core_os_image(&[
        ("lv0", b"SCE\0lv0"),
        ("lv1.self", LV1),
        ("lv2_kernel.self", KERNEL),
    ]);
    let unpack = unpack_image(&image, &entry);
    let lv1 = unpack
        .lv1
        .as_ref()
        .expect("the table names the hypervisor")
        .as_ref()
        .expect("the hypervisor was written");
    assert_eq!(lv1.path, "core_os/lv1.self");
    assert_eq!(lv1.stored_sha256, manifest::Sha256(sha256_of(LV1)));
    assert_eq!(std::fs::read(lv1_path(&entry)).unwrap(), LV1);
    assert_eq!(
        std::fs::read(entry.join(CORE_OS_DIR).join(LV2_KERNEL_SELF)).unwrap(),
        KERNEL
    );

    let record = unpack.into_record();
    assert_eq!(
        record.lv1,
        Some(KernelRecord {
            path: "core_os/lv1.self".to_string(),
            stored_sha256: manifest::Sha256(sha256_of(LV1)),
        })
    );
    assert_eq!(record.lv1_omission, None);
}

#[test]
fn a_table_naming_no_kernel_still_stores_its_hypervisor() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv0", b"SCE\0lv0"), ("lv1.self", LV1)]);
    let unpack = unpack_image(&image, &entry);
    assert!(
        matches!(unpack.kernel, Err(CoreOsOmission::NoKernel { files: 2 })),
        "{:?}",
        unpack.kernel
    );
    assert_eq!(std::fs::read(lv1_path(&entry)).unwrap(), LV1);
    let record = unpack.into_record();
    assert!(record.kernel.is_none() && record.omission.is_some());
    assert_eq!(
        record.lv1.map(|lv1| lv1.stored_sha256),
        Some(manifest::Sha256(sha256_of(LV1)))
    );
}

#[test]
fn a_hypervisor_that_cannot_be_written_is_an_omission_beside_the_stored_kernel() {
    let entry = scratch();
    // A directory under the hypervisor's name: no host renames a file
    // over it.
    std::fs::create_dir_all(lv1_path(&entry)).unwrap();
    let image = build_core_os_image(&[("lv1.self", LV1), ("lv2_kernel.self", KERNEL)]);
    let unpack = unpack_image(&image, &entry);
    let Some(Err(CoreOsOmission::Write { path, .. })) = &unpack.lv1 else {
        panic!("expected a write omission, got {:?}", unpack.lv1);
    };
    assert_eq!(*path, lv1_path(&entry));
    assert!(unpack.kernel.is_ok(), "{:?}", unpack.kernel);
    let record = unpack.into_record();
    assert!(record.lv1.is_none());
    let expected_prefix = format!("write {}: ", lv1_path(&entry).display());
    assert!(
        record
            .lv1_omission
            .as_deref()
            .is_some_and(|o| o.starts_with(&expected_prefix)),
        "{:?}",
        record.lv1_omission
    );
}

#[test]
fn a_table_naming_no_hypervisor_records_neither_field() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv2_kernel.self", KERNEL)]);
    let unpack = unpack_image(&image, &entry);
    assert!(unpack.lv1.is_none());
    assert!(!lv1_path(&entry).exists());
    let record = unpack.into_record();
    assert_eq!((record.lv1, record.lv1_omission), (None, None));
}

#[cfg(feature = "decrypt")]
#[test]
fn a_full_install_records_the_hypervisor_it_stores() {
    use crate::firmware_install::install_pup;
    use crate::store::layout::{Artifact, StoreLayout, VersionKey};
    use crate::store::record::InstallRecord;
    use crate::test_support::{build_pup, build_scepkg, build_tar, synthetic_vault};
    use cellgov_ps3_abi::format::pup::ENTRY_ID_UPDATE_FILES;

    let out = scratch();
    let vfs = out.join("vfs");
    let keys = synthetic_vault();
    let flash = build_scepkg(
        &keys,
        &build_tar(&[("dev_flash/vsh/etc/version.txt", b"release:04.9100:\n")]),
    );
    let image = build_core_os_image(&[("lv1.self", LV1), ("lv2_kernel.self", KERNEL)]);
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
    assert_eq!(std::fs::read(lv1_path(&outcome.entry_dir)).unwrap(), LV1);
    let record_path = StoreLayout::new(&vfs).record_path(&Artifact::Firmware {
        version: VersionKey::new("4.91").unwrap(),
    });
    let record = InstallRecord::parse(&std::fs::read_to_string(record_path).unwrap())
        .expect("the written record parses");
    let block = record.core_os.expect("the record carries the block");
    assert_eq!(block.lv1, outcome.core_os.lv1);
    assert_eq!(
        block.lv1.map(|lv1| lv1.stored_sha256),
        Some(manifest::Sha256(sha256_of(LV1)))
    );
}
