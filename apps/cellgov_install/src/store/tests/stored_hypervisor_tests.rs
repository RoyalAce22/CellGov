use super::*;

use crate::game_install::sha256_of;

fn block(
    lv1: Option<&str>,
    kernel_omission: Option<&str>,
    lv1_omission: Option<&str>,
) -> CoreOsRecord {
    CoreOsRecord {
        kernel: None,
        omission: kernel_omission.map(str::to_string),
        lv1: lv1.map(|path| KernelRecord {
            path: path.to_string(),
            stored_sha256: sha256_of(b"lv1"),
        }),
        lv1_omission: lv1_omission.map(str::to_string),
        files: Vec::new(),
    }
}

#[test]
fn a_stored_hypervisor_is_the_lv1_record() {
    let stored = block(Some("core_os/lv1.self"), None, None);
    assert_eq!(
        stored_hypervisor(Some(&stored)).map(|lv1| lv1.path.as_str()),
        Ok("core_os/lv1.self")
    );
}

#[test]
fn an_unstored_hypervisor_carries_its_own_reason_not_the_kernels() {
    let omitted = block(None, Some("kernel reason"), Some("rename failed"));
    assert_eq!(
        stored_hypervisor(Some(&omitted)),
        Err(KernelAbsence::Omitted(Some("rename failed")))
    );
}

#[test]
fn a_block_with_no_hypervisor_fields_is_an_omission_with_no_reason() {
    let bare = block(None, Some("kernel reason"), None);
    assert_eq!(
        stored_hypervisor(Some(&bare)),
        Err(KernelAbsence::Omitted(None))
    );
}

#[test]
fn a_record_without_the_block_has_not_recorded_the_hypervisor() {
    assert_eq!(stored_hypervisor(None), Err(KernelAbsence::NotRecorded));
}
