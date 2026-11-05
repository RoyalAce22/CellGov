//! PPU word loads where the reserved SPU range maps nothing, checked
//! against the `ppu_spu_reserved_loads` console capture.
//!
//! The console ran an aligned and a misaligned `lwz` at two places:
//!
//! - Raw SPU stride 0, with no Raw SPU created.
//! - SPU thread window slot 0, with no SPU thread group.
//!
//! On the console each load ends its child thread. The reserved SPU range
//! is caching-inhibited, so CellGov takes the alignment interrupt at both
//! misaligned loads. In CellGov the aligned loads complete and read zero
//! from the zero-readable range; a test of its own pins that difference.
//! Each case is its own run here; the console ran each in its own thread.

use cellgov_effects::FaultKind;
use cellgov_mem::{PageSize, Region, RegionAccess};
use cellgov_ppu::PpuFaultClass;
use cellgov_ps3_abi::lv2::errno::CELL_ESRCH;
use cellgov_ps3_abi::lv2::spu::{raw_window, thread_window};

use crate::span_harness::{exit, li, lwz, oris, payload, run, word};

/// The done marker, "SPRL".
const DONE: u32 = 0x5350_524C;
/// The value the probe writes for a word a case did not reach.
const SKIP: u32 = 0xDEAD_D00D;
/// What r6 holds before the load.
const PRESET: u32 = 0x5C5C_5C5C;

/// The four cases: (the base the load names, its displacement).
const CASES: [(u64, i16); 4] = [
    (raw_window::BASE, 0),
    (raw_window::BASE, 2),
    (thread_window::BASE, 0),
    (thread_window::BASE, 2),
];

/// Asserts that the console ended case `n`'s (1-based) child thread at
/// its load.
fn console_ended_the_thread(w: &[u32], n: usize) {
    let c = &w[2 + 8 * (n - 1)..][..8];
    assert_eq!(c[0..3], [0, 0, 1], "case {n}: create, start, reached");
    assert_eq!(c[3], 0, "case {n}: the load did not pass");
    assert_eq!(c[4], SKIP, "case {n}: no word was loaded");
    assert_eq!(c[5], CELL_ESRCH.code, "case {n}: the child is gone");
    assert_eq!(c[6..8], [SKIP, SKIP], "case {n}: no join");
}

/// Memory as the boot gives it at the probed addresses: main memory, and
/// the reserved SPU range at Raw SPU stride 0 and SPU thread window slot 0.
/// The reserved range is zero-readable and caching-inhibited.
fn regions() -> Vec<Region> {
    let reserved = |base: u64, label: &'static str| {
        Region::with_access(
            base,
            raw_window::STRIDE as usize,
            label,
            PageSize::Page64K,
            RegionAccess::ReservedZeroReadable,
        )
        .with_caching_inhibited()
    };
    vec![
        Region::new(0, 0x2_0000, "main", PageSize::Page64K),
        reserved(raw_window::BASE, "spu_reserved"),
        reserved(thread_window::BASE, "spu_reserved_threads"),
    ]
}

/// Runs `lwz r6, d(base)` with r6 preset, and returns the PPU's first
/// fault class and r6 after the run.
fn cellgov(base: u64, d: i16) -> (Option<PpuFaultClass>, u32) {
    let mut p = Vec::new();
    word(&mut p, 6, PRESET);
    p.push(li(5, 0));
    p.push(oris(5, 5, (base >> 16) as u16));
    p.push(lwz(6, d, 5));
    exit(&mut p);
    let ran = run(regions(), &p);
    let class = ran.first_fault.map(|(unit, kind)| {
        assert_eq!(unit, ran.ppu, "the PPU faults");
        match kind {
            FaultKind::Guest(code) => PpuFaultClass::of_code(code).expect("a PPU fault code"),
            other => panic!("{other:?}"),
        }
    });
    (class, ran.gpr()[6] as u32)
}

fn capture() -> Vec<u32> {
    let w = payload("ppu_spu_reserved_loads", 36);
    assert_eq!(w[0..2], [DONE, 0], "the main thread finished");
    assert_eq!(w[34..36], [0, 0], "the payload's tail");
    w
}

#[test]
fn misaligned_loads_where_the_reserved_spu_range_maps_nothing_end_the_thread_as_on_the_console() {
    let w = capture();
    for n in [2, 4] {
        console_ended_the_thread(&w, n);
        let (base, d) = CASES[n - 1];
        assert_eq!(
            cellgov(base, d),
            (Some(PpuFaultClass::Alignment), PRESET),
            "case {n}"
        );
    }
}

#[test]
fn aligned_loads_where_the_reserved_spu_range_maps_nothing_end_the_thread_on_the_console_but_read_zero_in_cellgov(
) {
    let w = capture();
    for n in [1, 3] {
        console_ended_the_thread(&w, n);
        let (base, d) = CASES[n - 1];
        assert_eq!(cellgov(base, d), (None, 0), "case {n}");
    }
}
