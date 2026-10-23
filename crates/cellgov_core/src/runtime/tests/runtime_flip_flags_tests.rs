//! The flip-flags words libgcm_sys.sprx reads a head's flip status
//! from: seeded by `sys_rsx_context_allocate`, rewritten by the
//! FLIP_STATUS package, and marked done when a flip completes.

use super::rsx::build_with_sys_rsx_reservation;
use super::*;
use crate::rsx::reports::RSX_CONTEXT_ID;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv2::rsx::{driver_info, package};
use cellgov_ps3_abi::lv2::syscall::{SYS_RSX_CONTEXT_ALLOCATE, SYS_RSX_CONTEXT_ATTRIBUTE};

/// The masks libgcm_sys.sprx's `cellGcmResetFlipStatus` and
/// `cellGcmSetFlipStatus` send.
const RESET_MASK: u64 = !driver_info::FLIP_DONE as u64;
const SET_VALUE: u64 = driver_info::FLIP_DONE as u64;

fn syscall(rt: &mut Runtime, syscall_args: [u64; 9]) {
    rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args,
    });
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
}

fn allocated() -> Runtime {
    let mut rt = build_with_sys_rsx_reservation();
    let mut args = [0u64; 9];
    args[0] = SYS_RSX_CONTEXT_ALLOCATE;
    args[1] = 0x100;
    args[2] = 0x108;
    args[3] = 0x110;
    args[4] = 0x118;
    syscall(&mut rt, args);
    assert!(rt.lv2_host().sys_rsx_context().allocated);
    rt
}

fn flip_status(rt: &mut Runtime, head: u64, mask: u64, value: u64) {
    let mut args = [0u64; 9];
    args[0] = SYS_RSX_CONTEXT_ATTRIBUTE;
    args[1] = u64::from(RSX_CONTEXT_ID);
    args[2] = u64::from(package::FLIP_STATUS);
    args[3] = head;
    args[4] = mask;
    args[5] = value;
    syscall(rt, args);
}

fn head_word(rt: &Runtime, head: usize) -> u32 {
    let addr = rt.lv2_host().sys_rsx_context().driver_info_addr
        + (driver_info::HEAD_FLIP_FLAGS_OFFSET + head * driver_info::HEAD_STRIDE) as u32;
    let bytes = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(u64::from(addr)), 4).unwrap())
        .expect("driver info is backed");
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

#[test]
fn allocation_leaves_both_heads_reading_done() {
    let rt = allocated();
    for head in 0..driver_info::HEAD_COUNT {
        assert_eq!(head_word(&rt, head), driver_info::FLIP_DONE, "head {head}");
    }
}

#[test]
fn the_reset_mask_clears_and_the_set_value_restores_one_head() {
    let mut rt = allocated();
    flip_status(&mut rt, 1, RESET_MASK, 0);
    assert_eq!(head_word(&rt, 1), 0, "reset head 1");
    assert_eq!(
        head_word(&rt, 0),
        driver_info::FLIP_DONE,
        "head 0 untouched"
    );
    flip_status(&mut rt, 1, 0, SET_VALUE);
    assert_eq!(head_word(&rt, 1), driver_info::FLIP_DONE, "set head 1");
}

#[test]
fn a_head_past_the_block_writes_nothing_and_is_named() {
    let mut rt = allocated();
    let breaks = rt.lv2_host().observability().invariant_break_count;
    flip_status(&mut rt, driver_info::HEAD_COUNT as u64, 0, 0);
    assert_eq!(
        rt.lv2_host().observability().invariant_break_count,
        breaks + 1
    );
    for head in 0..driver_info::HEAD_COUNT {
        assert_eq!(head_word(&rt, head), driver_info::FLIP_DONE, "head {head}");
    }
}

#[test]
fn a_completed_flip_marks_the_flip_head_done() {
    let mut rt = allocated();
    flip_status(&mut rt, driver_info::FLIP_HEAD as u64, RESET_MASK, 0);
    rt.registry_mut()
        .register_with(|id| RsxFlipRequestEmitterUnit {
            id,
            steps: Cell::new(0),
            buffer_index: 0,
        });
    rt.registry_mut()
        .register_with(|id| CountingUnit::new(id, 5));
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
    assert!(rt.rsx_flip().pending());
    assert_eq!(head_word(&rt, driver_info::FLIP_HEAD), 0, "still WAITING");
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
    assert!(!rt.rsx_flip().pending());
    assert_eq!(
        head_word(&rt, driver_info::FLIP_HEAD),
        driver_info::FLIP_DONE,
        "the completion published DONE where libgcm reads it"
    );
}

/// The words the `rsx_flip_status_set` probe read on the reference
/// console: rsxInit's code, then the flip status after reset, set,
/// reset and set.
fn console_flip_status_set() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/rsx_flip_status_set/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

/// The status libgcm_sys.sprx's getter derives from a head word: DONE
/// (0) when the done bit is set, WAITING (1) when it is clear.
fn libgcm_flip_status(word: u32) -> u32 {
    u32::from(word & driver_info::FLIP_DONE == 0)
}

#[test]
fn the_reset_and_set_calls_read_back_as_the_console_s() {
    let console = console_flip_status_set();
    assert_eq!(console.len(), 5);
    assert_eq!(console[0], 0, "rsxInit succeeded on the console");
    let mut rt = allocated();
    let head = driver_info::FLIP_HEAD as u64;
    let calls = [
        (RESET_MASK, 0),
        (0, SET_VALUE),
        (RESET_MASK, 0),
        (0, SET_VALUE),
    ];
    for (k, (mask, value)) in calls.into_iter().enumerate() {
        flip_status(&mut rt, head, mask, value);
        assert_eq!(
            libgcm_flip_status(head_word(&rt, driver_info::FLIP_HEAD)),
            console[k + 1],
            "read {k}"
        );
    }
}
