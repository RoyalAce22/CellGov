//! The block a completion pass records over the block the entry held.

use super::*;
use crate::firmware_install::core_os::{unpack_image, CoreOsOmission};
use crate::manifest::{sha256_of, Sha256};
use crate::scratch_dir::scratch;
use crate::store::record::CoreOsFileRecord;
use crate::test_support::build_core_os_image;

const KERNEL: &[u8] = b"SCE\0new lv2 kernel bytes";
const LV1: &[u8] = b"SCE\0new lv1 hypervisor bytes";

fn stored(path: &str, bytes: &[u8]) -> KernelRecord {
    KernelRecord {
        path: path.to_string(),
        stored_sha256: Sha256(sha256_of(bytes)),
    }
}

/// A block that holds a kernel, a hypervisor and a two-file table.
fn held() -> CoreOsRecord {
    CoreOsRecord {
        kernel: Some(stored("core_os/lv2_kernel.self", b"SCE\0held kernel")),
        omission: None,
        lv1: Some(stored("core_os/lv1.self", b"SCE\0held lv1")),
        lv1_omission: None,
        files: vec![
            CoreOsFileRecord {
                name: "lv1.self".to_string(),
                size: 12,
            },
            CoreOsFileRecord {
                name: "lv2_kernel.self".to_string(),
                size: 15,
            },
        ],
    }
}

fn no_package() -> CoreOsUnpack {
    CoreOsUnpack {
        files: Vec::new(),
        kernel: Err(CoreOsOmission::NoPackage),
        lv1: None,
    }
}

#[test]
fn a_package_that_yields_no_table_keeps_the_held_kernel_hypervisor_and_table() {
    let held = held();
    let (block, kernel, lv1) = settle_over_held(Some(&held), no_package());
    let reason = CoreOsOmission::NoPackage.to_string();
    assert_eq!(
        kernel,
        SelfCompletion::Kept {
            reason: reason.clone()
        }
    );
    assert_eq!(lv1, SelfCompletion::Kept { reason });
    assert_eq!(block, held, "the record still names every held file");
}

#[test]
fn a_fresh_kernel_and_hypervisor_over_held_ones_are_both_replaced() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv1.self", LV1), ("lv2_kernel.self", KERNEL)]);
    let (block, kernel, lv1) = settle_over_held(Some(&held()), unpack_image(&image, &entry));
    assert_eq!(kernel, SelfCompletion::Replaced);
    assert_eq!(lv1, SelfCompletion::Replaced);
    assert_eq!(
        block.kernel,
        Some(stored("core_os/lv2_kernel.self", KERNEL))
    );
    assert_eq!(block.lv1, Some(stored("core_os/lv1.self", LV1)));
    assert_eq!((block.omission, block.lv1_omission), (None, None));
}

#[test]
fn a_table_naming_the_hypervisor_but_no_kernel_replaces_one_and_keeps_the_other() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv0", b"SCE\0lv0"), ("lv1.self", LV1)]);
    let held = held();
    let (block, kernel, lv1) = settle_over_held(Some(&held), unpack_image(&image, &entry));
    assert_eq!(
        kernel,
        SelfCompletion::Kept {
            reason: CoreOsOmission::NoKernel { files: 2 }.to_string()
        }
    );
    assert_eq!(lv1, SelfCompletion::Replaced);
    assert_eq!(block.kernel, held.kernel);
    assert_eq!(block.omission, None);
    assert_eq!(block.lv1, Some(stored("core_os/lv1.self", LV1)));
    assert_eq!(block.files.len(), 2, "the table this pass read");
    assert_eq!(block.files[0].name, "lv0");
}

#[test]
fn a_table_naming_no_hypervisor_keeps_the_held_one_and_says_why() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv2_kernel.self", KERNEL)]);
    let held = held();
    let (block, kernel, lv1) = settle_over_held(Some(&held), unpack_image(&image, &entry));
    assert_eq!(kernel, SelfCompletion::Replaced);
    assert_eq!(
        lv1,
        SelfCompletion::Kept {
            reason: "CORE_OS_PACKAGE.pkg names no lv1.self".to_string()
        }
    );
    assert_eq!(block.lv1, held.lv1);
    assert_eq!(block.lv1_omission, None);
}

#[test]
fn a_hypervisor_new_to_the_entry_is_written_and_the_held_kernel_replaced() {
    let entry = scratch();
    let image = build_core_os_image(&[("lv1.self", LV1), ("lv2_kernel.self", KERNEL)]);
    let mut held = held();
    held.lv1 = None;
    let (block, kernel, lv1) = settle_over_held(Some(&held), unpack_image(&image, &entry));
    assert_eq!(kernel, SelfCompletion::Replaced);
    assert_eq!(lv1, SelfCompletion::Written);
    assert_eq!(block.lv1, Some(stored("core_os/lv1.self", LV1)));
}

#[test]
fn with_nothing_held_a_package_that_yields_no_table_records_the_omission_alone() {
    let (block, kernel, lv1) = settle_over_held(None, no_package());
    assert_eq!(
        (kernel, lv1),
        (SelfCompletion::Absent, SelfCompletion::Absent)
    );
    assert_eq!(
        block,
        CoreOsRecord {
            kernel: None,
            omission: Some(CoreOsOmission::NoPackage.to_string()),
            lv1: None,
            lv1_omission: None,
            files: Vec::new(),
        }
    );
}
