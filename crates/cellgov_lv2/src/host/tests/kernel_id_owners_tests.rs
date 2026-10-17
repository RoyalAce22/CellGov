//! Each kernel id records the process that holds it: the limit counts
//! one process's ids, and a process's ids can be listed for its exit.

use cellgov_ps3_abi::lv2::kernel_id::{compose, MAX_LIVE_IDS};
use cellgov_ps3_abi::lv2::process::{
    BOOT_PROCESS_PID, SYS_MUTEX_OBJECT, SYS_RWLOCK_OBJECT, SYS_SEMAPHORE_OBJECT,
};

use super::*;

const CHILD: u32 = BOOT_PROCESS_PID + 0x100;

#[test]
fn the_limit_counts_only_the_creating_process_ids() {
    let mut t = KernelIdTable::default();
    for n in 0..MAX_LIVE_IDS {
        assert!(t.alloc(BOOT_PROCESS_PID, 0x85).is_some(), "create {n}");
    }
    assert_eq!(t.alloc(BOOT_PROCESS_PID, 0x85), None);
    let child = t.alloc(CHILD, 0x85).expect("the child holds no id yet");
    assert!(t.is_live(child));
    assert_eq!(t.alloc(BOOT_PROCESS_PID, 0x85), None, "still full");
}

#[test]
fn held_by_owner_names_each_process_ids_and_a_release_drops_them() {
    let mut t = KernelIdTable::default();
    let boot = t.alloc(BOOT_PROCESS_PID, SYS_MUTEX_OBJECT).unwrap();
    let child = t.alloc(CHILD, SYS_SEMAPHORE_OBJECT).unwrap();
    let boot_again = t.alloc(BOOT_PROCESS_PID, SYS_RWLOCK_OBJECT).unwrap();
    assert_eq!(t.held_by_owner(BOOT_PROCESS_PID), vec![boot, boot_again]);
    assert_eq!(t.held_by_owner(CHILD), vec![child]);
    assert!(t.release(child));
    assert!(t.held_by_owner(CHILD).is_empty());
    let reused = t.alloc(BOOT_PROCESS_PID, SYS_MUTEX_OBJECT).unwrap();
    assert_eq!(reused, compose(SYS_MUTEX_OBJECT, 1, 1));
    assert_eq!(
        t.held_by_owner(BOOT_PROCESS_PID),
        vec![boot, boot_again, reused]
    );
}
