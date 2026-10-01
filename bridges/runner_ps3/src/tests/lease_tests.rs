//! The lease: one holder at a time, the refusal that names the holder
//! and the clearing command, release, drop and unlock.

use super::*;

#[test]
fn a_host_folds_to_a_file_name() {
    let dir = Path::new("leases");
    assert_eq!(
        lease_path(dir, "10.77.0.2"),
        dir.join("cellgov_runner_ps3_10.77.0.2.lease")
    );
    assert_eq!(
        lease_path(dir, "ps3:80/../x"),
        dir.join("cellgov_runner_ps3_ps3_80_.._x.lease")
    );
}

#[test]
fn a_second_runner_is_refused_with_the_holder_and_the_command_that_clears_it() {
    let dir = cellgov_testkit::scratch::scratch();
    let first = Lease::acquire(&dir, "10.77.0.2", "spu_fixed_value").expect("free");
    let err = Lease::acquire(&dir, "10.77.0.2", "dma_completion").expect_err("held");
    match &err {
        LeaseError::Held { host, holder, .. } => {
            assert_eq!(host, "10.77.0.2");
            assert!(holder.ends_with("holder=spu_fixed_value"), "{holder}");
            assert!(
                holder.starts_with(&format!("pid={}", std::process::id())),
                "{holder}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        err.to_string()
            .ends_with("clear it with `runner_ps3 unlock --host 10.77.0.2`"),
        "{err}"
    );
    first.release().expect("release");
    Lease::acquire(&dir, "10.77.0.2", "dma_completion").expect("free again");
}

#[test]
fn another_console_has_its_own_lease() {
    let dir = cellgov_testkit::scratch::scratch();
    let _one = Lease::acquire(&dir, "10.77.0.2", "a").expect("free");
    let _two = Lease::acquire(&dir, "10.77.0.3", "b").expect("another host is free");
}

#[test]
fn a_dropped_lease_frees_the_console() {
    let dir = cellgov_testkit::scratch::scratch();
    let path = {
        let lease = Lease::acquire(&dir, "10.77.0.2", "a").expect("free");
        lease.path().to_path_buf()
    };
    assert!(!path.exists());
    Lease::acquire(&dir, "10.77.0.2", "b").expect("free after drop");
}

#[test]
fn unlock_clears_a_stale_lease_and_reports_whether_one_was_there() {
    let dir = cellgov_testkit::scratch::scratch();
    let lease = Lease::acquire(&dir, "10.77.0.2", "a").expect("free");
    std::mem::forget(lease);
    assert!(unlock(&dir, "10.77.0.2").expect("unlock"));
    assert!(!unlock(&dir, "10.77.0.2").expect("nothing to unlock"));
    Lease::acquire(&dir, "10.77.0.2", "b").expect("free after unlock");
}
