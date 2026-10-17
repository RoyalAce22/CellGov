//! The primary stack's placement held to the `lv2_process_queries` and
//! `lv2_process_stack_*` captures from the reference console: the base
//! `sys_ppu_thread_get_stack_information` reports, and r1 in each
//! probe's `main`.

use super::{primary_entry_sp, resolve_primary_stacksize};
use crate::NullSink;
use cellgov_ps3_abi::hw::address_space::PS3_PRIMARY_STACK_START;

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

/// Payload offsets of the 8 bytes the stack call wrote (u32 address,
/// u32 size), r1 in `main`, and the declared size.
const OFF_STACK_INFO: usize = 56;
const OFF_R1_AT_MAIN: usize = 64;
const OFF_DECLARED: usize = 72;

/// What the probes push between the entry r1 and the r1 `main` reads:
/// the common crt0's 0x80-byte frame (`tests/micro/common/crt0.S`) and
/// the probe `main`'s 0x100-byte one (its `stdu r1,-256(r1)`).
const PROBE_FRAMES: u64 = 0x180;

fn payload(probe: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    frame[8..].to_vec()
}

fn word(payload: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(payload[at..at + 8].try_into().expect("8 bytes"))
}

#[test]
fn the_primary_stack_sits_at_the_console_base_with_the_console_entry_r1() {
    let mut wrong = Vec::new();
    for (probe, declares) in PROBES {
        let p = payload(probe);
        let info = word(&p, OFF_STACK_INFO);
        let console_base = info >> 32;
        let console_r1 = word(&p, OFF_R1_AT_MAIN);
        let declared = word(&p, OFF_DECLARED) as u32;
        let size = resolve_primary_stacksize(declares.then_some(declared), &NullSink);
        let cellgov_r1 = primary_entry_sp(size) - PROBE_FRAMES;
        if (PS3_PRIMARY_STACK_START, cellgov_r1) != (console_base, console_r1) {
            wrong.push(format!(
                "{probe}: size {size:#x}, CellGov base {PS3_PRIMARY_STACK_START:#x} r1 at main \
                 {cellgov_r1:#x}, console base {console_base:#x} r1 at main {console_r1:#x}"
            ));
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
