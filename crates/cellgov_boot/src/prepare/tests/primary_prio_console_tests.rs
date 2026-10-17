//! The primary thread's priority held to the `lv2_process_prio_*`
//! captures from the reference console. Each probe declares one
//! `primary_prio` and records the priority
//! `sys_ppu_thread_get_priority` reads for its own thread.

use super::resolve_primary_prio;
use crate::NullSink;

const PROBES: [&str; 2] = ["lv2_process_prio_3071", "lv2_process_prio_3072"];

/// The probe's four words: the declared priority, the thread-id call's
/// and the priority call's return codes, and the priority read.
fn words(probe: &str) -> [u32; 4] {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    assert_eq!(frame.len(), 8 + 16, "{probe}: one 16-byte frame");
    std::array::from_fn(|i| {
        u32::from_be_bytes(frame[8 + 4 * i..12 + 4 * i].try_into().expect("4 bytes"))
    })
}

#[test]
fn the_primary_priority_matches_the_console_for_each_declaration() {
    for probe in PROBES {
        let [declared, id_rc, prio_rc, prio] = words(probe);
        assert_eq!((id_rc, prio_rc), (0, 0), "{probe}: both reads succeed");
        assert_eq!(
            resolve_primary_prio(Some(declared as i32), &NullSink),
            prio,
            "{probe}: declared {declared}"
        );
    }
}
