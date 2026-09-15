//! Stale process-exit note appending to fault diagnostics.

use super::*;

#[test]
fn append_orphan_exit_info_is_noop_when_none() {
    let mut s = String::from("FAULT at step 100");
    append_orphan_exit_info(&mut s, None);
    assert_eq!(s, "FAULT at step 100");
}

#[test]
fn append_orphan_exit_info_appends_code_and_pc_when_some() {
    let mut s = String::from("FAULT at step 100");
    append_orphan_exit_info(
        &mut s,
        Some(&ProcessExitInfo {
            code: 0x42,
            call_pc: 0x10ab_cdef,
        }),
    );
    assert!(s.contains("code=66"), "got {s}");
    assert!(s.contains("PC=0x10abcdef"), "got {s}");
    assert!(s.contains("stale exit info"), "got {s}");
}
