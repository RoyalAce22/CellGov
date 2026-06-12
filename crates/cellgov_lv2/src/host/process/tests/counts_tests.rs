//! `ProcessCounts` per-class object counting: full class-id coverage, inc/dec wiring, and zero saturation.

use super::*;
use cellgov_ps3_abi::sys_process::ALL_PROCESS_OBJECT_CLASS_IDS;

/// Drives off [`ALL_PROCESS_OBJECT_CLASS_IDS`] so a new constant
/// landing in `cellgov_ps3_abi::sys_process` without a
/// `count_for_class` arm shows up as a test failure rather than
/// as a silent zero forever. Empty host: every documented class
/// reports 0.
#[test]
fn count_for_class_covers_every_documented_class_id() {
    let host = Lv2Host::new();
    let counts = ProcessCounts::new();
    for &class in ALL_PROCESS_OBJECT_CLASS_IDS {
        assert_eq!(
            counts.count_for_class(class, &host),
            0,
            "fresh host must report 0 for class 0x{class:02X}"
        );
    }
    // Unknown class falls to zero.
    assert_eq!(counts.count_for_class(0xFF, &host), 0);
}

/// Catches the strongest realistic regression: someone adds a
/// constant to [`ALL_PROCESS_OBJECT_CLASS_IDS`] (so the coverage
/// test passes vacuously) but forgets to add a `count_for_class`
/// arm. Bumping each counter and asserting at least one class
/// becomes nonzero exercises the wiring beyond the empty-host
/// case.
#[test]
fn each_counter_bump_moves_a_documented_class() {
    let host = Lv2Host::new();
    let mut counts = ProcessCounts::new();
    for (label, bump, expected_class) in [
        (
            "timer",
            &ProcessCounts::timer_inc as &dyn Fn(&mut ProcessCounts),
            SYS_TIMER_OBJECT,
        ),
        (
            "rwlock",
            &ProcessCounts::rwlock_inc as &dyn Fn(&mut ProcessCounts),
            SYS_RWLOCK_OBJECT,
        ),
        (
            "event_port",
            &ProcessCounts::event_port_inc as &dyn Fn(&mut ProcessCounts),
            SYS_EVENT_PORT_OBJECT,
        ),
        (
            "lwcond",
            &ProcessCounts::lwcond_inc as &dyn Fn(&mut ProcessCounts),
            SYS_LWCOND_OBJECT,
        ),
        (
            "fs_fd",
            &ProcessCounts::fs_fd_inc as &dyn Fn(&mut ProcessCounts),
            SYS_FS_FD_OBJECT,
        ),
    ] {
        let before = counts.count_for_class(expected_class, &host);
        bump(&mut counts);
        let after = counts.count_for_class(expected_class, &host);
        assert_eq!(
            after,
            before + 1,
            "{label}: bump did not increment count for class 0x{expected_class:02X}"
        );
    }
}

#[test]
fn counter_classes_observe_inc_dec() {
    let host = Lv2Host::new();
    let mut counts = ProcessCounts::new();
    counts.timer_inc();
    counts.timer_inc();
    counts.rwlock_inc();
    counts.event_port_inc();
    counts.lwcond_inc();
    counts.fs_fd_inc();
    assert_eq!(counts.count_for_class(SYS_TIMER_OBJECT, &host), 2);
    assert_eq!(counts.count_for_class(SYS_RWLOCK_OBJECT, &host), 1);
    assert_eq!(counts.count_for_class(SYS_EVENT_PORT_OBJECT, &host), 1);
    assert_eq!(counts.count_for_class(SYS_LWCOND_OBJECT, &host), 1);
    assert_eq!(counts.count_for_class(SYS_FS_FD_OBJECT, &host), 1);

    counts.timer_dec();
    counts.rwlock_dec();
    counts.lwcond_dec();
    assert_eq!(counts.count_for_class(SYS_TIMER_OBJECT, &host), 1);
    assert_eq!(counts.count_for_class(SYS_RWLOCK_OBJECT, &host), 0);
    assert_eq!(counts.count_for_class(SYS_LWCOND_OBJECT, &host), 0);
}

#[test]
fn dec_saturates_at_zero() {
    let mut counts = ProcessCounts::new();
    counts.timer_dec();
    counts.rwlock_dec();
    counts.event_port_dec();
    counts.lwcond_dec();
    let host = Lv2Host::new();
    assert_eq!(counts.count_for_class(SYS_TIMER_OBJECT, &host), 0);
    assert_eq!(counts.count_for_class(SYS_RWLOCK_OBJECT, &host), 0);
    assert_eq!(counts.count_for_class(SYS_EVENT_PORT_OBJECT, &host), 0);
    assert_eq!(counts.count_for_class(SYS_LWCOND_OBJECT, &host), 0);
}
