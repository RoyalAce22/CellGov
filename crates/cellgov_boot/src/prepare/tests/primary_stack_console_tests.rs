//! The primary stack size held to the `lv2_process_queries` and
//! `lv2_process_stack_*` captures from the reference console. Each probe
//! declares one `primary_stacksize` (or, in `lv2_process_stack_undeclared`,
//! no process param) and records the size `sys_ppu_thread_get_stack_information`
//! reports.

use super::resolve_primary_stacksize;
use crate::NullSink;

/// The probes, and whether each declares a process param.
const PROBES: [(&str, bool); 13] = [
    ("lv2_process_queries", true),
    ("lv2_process_stack_small", true),
    ("lv2_process_stack_sub_page", true),
    ("lv2_process_stack_6k", true),
    ("lv2_process_stack_16k", true),
    ("lv2_process_stack_32k", true),
    ("lv2_process_stack_raw", true),
    ("lv2_process_stack_unaligned", true),
    ("lv2_process_stack_max", true),
    ("lv2_process_stack_over_max", true),
    ("lv2_process_stack_sentinel", true),
    ("lv2_process_stack_sentinel_128k", true),
    ("lv2_process_stack_undeclared", false),
];

/// Payload offsets of the stack call's return code, the 8 bytes it
/// wrote (u32 address, u32 size), and the declared size.
const OFF_STACK_RC: usize = 48;
const OFF_STACK_INFO: usize = 56;
const OFF_DECLARED: usize = 72;

fn payload(probe: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(frame.len(), 8 + len, "{probe}: one whole frame");
    frame[8..].to_vec()
}

fn word(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

#[test]
fn the_primary_stack_size_matches_the_console_for_every_declaration() {
    let mut wrong = Vec::new();
    for (probe, declares) in PROBES {
        let p = payload(probe);
        assert_eq!(word(&p, OFF_STACK_RC), 0, "{probe}: stack information rc");
        let console = word(&p, OFF_STACK_INFO) as u32;
        let declared = word(&p, OFF_DECLARED) as u32;
        let cellgov = resolve_primary_stacksize(declares.then_some(declared), &NullSink);
        if cellgov != console {
            wrong.push(format!(
                "{probe}: declared {declared:#x}, CellGov {cellgov:#x}, console {console:#x}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
