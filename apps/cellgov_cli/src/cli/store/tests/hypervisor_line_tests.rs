use super::*;
use cellgov_install::manifest::Sha256;
use cellgov_install::store::KernelRecord;

fn block(lv1: Option<KernelRecord>, lv1_omission: Option<&str>) -> CoreOsRecord {
    CoreOsRecord {
        kernel: None,
        omission: None,
        lv1,
        lv1_omission: lv1_omission.map(str::to_string),
        files: Vec::new(),
    }
}

#[test]
fn a_block_with_no_hypervisor_and_no_reason_still_reports_one_not_unpacked() {
    assert_eq!(
        hypervisor_line(&block(None, None)),
        Err("  hypervisor not unpacked".to_string())
    );
}

#[test]
fn a_block_with_no_hypervisor_reports_its_reason() {
    assert_eq!(
        hypervisor_line(&block(None, Some("the write failed"))),
        Err("  hypervisor not unpacked: the write failed".to_string())
    );
}

#[test]
fn a_stored_hypervisor_reports_its_path_and_stored_sha256() {
    let stored = KernelRecord {
        path: "core_os/lv1.self".to_string(),
        stored_sha256: Sha256([0xab; 32]),
    };
    assert_eq!(
        hypervisor_line(&block(Some(stored), None)),
        Ok(format!(
            "  hypervisor core_os/lv1.self (as stored, sha256 {})",
            "ab".repeat(32)
        ))
    );
}
