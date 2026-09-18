//! `verify_stored_kernel` over the kernel as it lies: match, missing,
//! modified.

use super::*;
use crate::scratch_dir::scratch;

fn kernel(bytes: &[u8]) -> KernelRecord {
    KernelRecord {
        path: "core_os/lv2_kernel.self".to_string(),
        stored_sha256: manifest::Sha256(manifest::sha256_of(bytes)),
    }
}

#[test]
fn a_kernel_whose_bytes_match_the_record_is_a_match() {
    let entry = scratch();
    std::fs::create_dir_all(entry.join("core_os")).unwrap();
    std::fs::write(entry.join("core_os/lv2_kernel.self"), b"SCE\0kernel").unwrap();
    assert!(verify_stored_kernel(&entry, &kernel(b"SCE\0kernel"))
        .expect("readable")
        .is_none());
}

#[test]
fn an_absent_kernel_is_missing_at_the_path_the_record_names() {
    let entry = scratch();
    let fault = verify_stored_kernel(&entry, &kernel(b"SCE\0kernel"))
        .expect("absence is an answer")
        .expect("a fault");
    assert_eq!(fault.kind, ModuleDivergence::Missing);
    assert_eq!(fault.path, entry.join("core_os").join("lv2_kernel.self"));
}

#[test]
fn a_kernel_whose_bytes_changed_names_both_digests() {
    let entry = scratch();
    std::fs::create_dir_all(entry.join("core_os")).unwrap();
    std::fs::write(entry.join("core_os/lv2_kernel.self"), b"SCE\0other").unwrap();
    let recorded = kernel(b"SCE\0kernel");
    let fault = verify_stored_kernel(&entry, &recorded)
        .expect("readable")
        .expect("a fault");
    assert_eq!(
        fault.kind,
        ModuleDivergence::Modified {
            expected: recorded.stored_sha256,
            found: manifest::Sha256(manifest::sha256_of(b"SCE\0other")),
        }
    );
}

#[test]
fn a_kernel_that_cannot_be_read_is_a_refusal_not_a_divergence() {
    let entry = scratch();
    // A directory where the file should be reads as neither present
    // bytes nor absence.
    std::fs::create_dir_all(entry.join("core_os/lv2_kernel.self")).unwrap();
    assert!(matches!(
        verify_stored_kernel(&entry, &kernel(b"x")),
        Err(FirmwareVerifyError::ModuleRead { .. })
    ));
}
