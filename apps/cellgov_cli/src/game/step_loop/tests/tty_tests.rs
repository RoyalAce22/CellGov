//! TTY write-capture classification -- region-aware resolution and
//! bogus-fd narrowing.

use super::*;
use cellgov_mem::{PageSize, Region};

const MAIN_SIZE: u64 = 0x1_0000;
const STACK_BASE: u64 = 0xD000_0000;
const STACK_SIZE: u64 = 0x1000;

fn tty_args(fd: u64, buf: u64, len: u64) -> [u64; 9] {
    [403, fd, buf, len, 0, 0, 0, 0, 0]
}

/// A main region at 0 and a stack region far above it, with a gap
/// between; `fill` seeds bytes at guest addresses.
fn layout(fill: &[(u64, &[u8])]) -> GuestMemory {
    let mut mem = GuestMemory::from_regions(vec![
        Region::new(0, MAIN_SIZE as usize, "main", PageSize::Page64K),
        Region::new(STACK_BASE, STACK_SIZE as usize, "stack", PageSize::Page4K),
    ])
    .unwrap();
    for (addr, bytes) in fill {
        let range = ByteRange::new(GuestAddr::new(*addr), bytes.len() as u64).unwrap();
        mem.apply_commit(range, bytes).unwrap();
    }
    mem
}

fn in_bounds(fd: u32, fd_was_bogus: bool, bytes: &[u8]) -> TtyCaptureDecision {
    TtyCaptureDecision::InBounds {
        fd,
        fd_was_bogus,
        bytes: bytes.to_vec(),
    }
}

#[test]
fn a_main_region_buffer_is_captured() {
    let mem = layout(&[(0x100, b"hello\0padding")]);
    assert_eq!(
        classify_tty_capture(&tty_args(1, 0x100, 5), &mem),
        in_bounds(1, false, b"hello")
    );
}

#[test]
fn a_stack_region_buffer_is_captured() {
    let addr = STACK_BASE + STACK_SIZE - 0x40;
    let mem = layout(&[(addr, b"CGOV\x00\x00\x00\x04")]);
    assert_eq!(
        classify_tty_capture(&tty_args(1, addr + 4, 4), &mem),
        in_bounds(1, false, b"\x00\x00\x00\x04")
    );
}

#[test]
fn a_buffer_in_the_gap_between_regions_is_oob_and_names_its_neighbours() {
    let mem = layout(&[]);
    let buf = MAIN_SIZE + 0x1000;
    match classify_tty_capture(&tty_args(1, buf, 8), &mem) {
        TtyCaptureDecision::Oob {
            buf: b,
            len,
            reason: MemError::Unmapped(ctx),
        } => {
            assert_eq!((b, len), (buf, 8));
            assert_eq!(ctx.addr, buf);
            assert_eq!(ctx.nearest_below, Some("main"));
            assert_eq!(ctx.nearest_above, Some("stack"));
        }
        other => panic!("expected Unmapped Oob, got {other:?}"),
    }
}

#[test]
fn a_buffer_running_past_its_region_end_is_oob() {
    let mem = layout(&[]);
    let buf = STACK_BASE + STACK_SIZE - 2;
    assert!(matches!(
        classify_tty_capture(&tty_args(1, buf, 8), &mem),
        TtyCaptureDecision::Oob {
            reason: MemError::Unmapped(_),
            ..
        }
    ));
}

#[test]
fn a_range_that_wraps_the_address_space_is_oob() {
    let mem = layout(&[]);
    let decision = classify_tty_capture(&tty_args(1, u64::MAX, 8), &mem);
    assert!(
        matches!(
            decision,
            TtyCaptureDecision::Oob {
                buf: u64::MAX,
                len: 8,
                ..
            }
        ),
        "u64::MAX + 8 must classify as Oob, got {decision:?}"
    );
}

#[test]
fn a_wide_fd_is_narrowed_and_flagged() {
    let mem = layout(&[(0, b"ok")]);
    assert_eq!(
        classify_tty_capture(&tty_args(u64::from(u32::MAX) + 1, 0, 2), &mem),
        in_bounds(u32::MAX, true, b"ok")
    );
}

#[test]
fn the_full_buffer_is_kept_above_4kib() {
    let payload = vec![b'x'; 8000];
    let mem = layout(&[(0x100, &payload)]);
    match classify_tty_capture(&tty_args(1, 0x100, 8000), &mem) {
        TtyCaptureDecision::InBounds { bytes, .. } => assert_eq!(bytes, payload),
        other => panic!("expected InBounds, got {other:?}"),
    }
}

#[test]
fn zero_len_never_dereferences_the_buffer() {
    let mem = layout(&[]);
    for buf in [
        0xDEAD_BEEF_u64,
        MAIN_SIZE,
        STACK_BASE + STACK_SIZE,
        u64::MAX,
    ] {
        assert_eq!(
            classify_tty_capture(&tty_args(1, buf, 0), &mem),
            in_bounds(1, false, b""),
            "buf=0x{buf:x}"
        );
    }
}
