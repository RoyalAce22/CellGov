use super::*;
use cellgov_install::manifest::Sha256;
use cellgov_install::store::KernelRecord;

fn record(kernel: Option<KernelRecord>, omission: Option<&str>) -> CoreOsRecord {
    CoreOsRecord {
        kernel,
        omission: omission.map(str::to_string),
        files: Vec::new(),
    }
}

#[test]
fn kernel_only_succeeds_only_when_the_record_holds_a_kernel() {
    let kernel = KernelRecord {
        path: "core_os/lv2_kernel.self".to_string(),
        stored_sha256: Sha256([0; 32]),
    };
    assert_eq!(kernel_only_exit_status(&record(Some(kernel), None)), 0);
    assert_eq!(
        kernel_only_exit_status(&record(None, Some("no CORE_OS_PACKAGE.pkg"))),
        EXIT_KERNEL_OMITTED
    );
    assert_eq!(
        kernel_only_exit_status(&record(None, None)),
        EXIT_KERNEL_OMITTED
    );
}
