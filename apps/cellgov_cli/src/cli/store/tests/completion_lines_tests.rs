use super::*;

fn block_line() -> Result<String, String> {
    Ok("  hypervisor core_os/lv1.self (as stored, sha256 00)".to_string())
}

#[test]
fn a_replaced_hypervisor_is_reported_as_written_over() {
    assert_eq!(
        completion_lines("hypervisor", &SelfCompletion::Replaced, block_line()),
        vec![
            block_line(),
            Ok("  the hypervisor already stored there was written over".to_string()),
        ]
    );
}

#[test]
fn a_kept_hypervisor_is_reported_with_the_reason_and_not_as_stored_by_this_pass() {
    let kept = SelfCompletion::Kept {
        reason: "CORE_OS_PACKAGE.pkg: no keyset opened it".to_string(),
    };
    assert_eq!(
        completion_lines("hypervisor", &kept, block_line()),
        vec![Err(
            "  hypervisor not unpacked: CORE_OS_PACKAGE.pkg: no keyset opened it; the hypervisor \
             already stored there is kept, and the record still names it"
                .to_string()
        )]
    );
}

#[test]
fn a_written_or_absent_self_gets_the_block_line_alone() {
    for completion in [SelfCompletion::Written, SelfCompletion::Absent] {
        assert_eq!(
            completion_lines("kernel", &completion, block_line()),
            vec![block_line()]
        );
    }
}
