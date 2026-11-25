use super::*;

#[test]
fn kernel_only_succeeds_only_when_the_pass_wrote_the_kernel() {
    assert_eq!(kernel_only_exit_status(&SelfCompletion::Written), 0);
    assert_eq!(kernel_only_exit_status(&SelfCompletion::Replaced), 0);
    assert_eq!(
        kernel_only_exit_status(&SelfCompletion::Kept {
            reason: "CORE_OS_PACKAGE.pkg: no keyset opened it".to_string()
        }),
        EXIT_KERNEL_OMITTED
    );
    assert_eq!(
        kernel_only_exit_status(&SelfCompletion::Absent),
        EXIT_KERNEL_OMITTED
    );
}
