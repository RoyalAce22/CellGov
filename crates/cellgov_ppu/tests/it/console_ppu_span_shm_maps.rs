//! A doubleword load and store across the edge between two 64 KiB
//! shared-memory segments mapped end to end, checked against the
//! `ppu_span_shm_maps` console capture.
//!
//! Each map is a region of its own in CellGov. On the console both
//! accesses complete: the `ld` reads four bytes from each segment and the
//! `std` writes four bytes into each.

use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv2::syscall as sc;

use crate::span_harness::{
    addi, addis, call, doubleword, exit, ld, li, lwz, oris, payload, run, sldi32, std, stw, word,
    OUT,
};

/// The done marker, "SPAN".
const DONE: u32 = 0x5350_414E;
/// The value the probe's child passes to its thread exit.
const CHILD_EXIT: u32 = 0xE5;

/// The probe's steps as one thread.
///
/// 1. Allocate the area and the two segments, then map the segments.
/// 2. Store the four words around the edge.
/// 3. `ld` into r24, then `std` 0x1122334455667788.
/// 4. Load the four words again into r25..r28.
/// 5. Call `sys_process_exit`.
///
/// r14..r18 hold the call return codes, and r20 holds the area base.
fn program() -> Vec<u32> {
    let mut p = Vec::new();
    let mut args = Vec::new();
    word(&mut args, 3, 0x1000_0000);
    args.push(li(4, 0x200));
    word(&mut args, 5, 0x1000_0000);
    args.push(li(6, OUT as i16));
    call(&mut p, sc::MMAPPER_ALLOCATE_ADDRESS, &args, 14);
    for (slot, rc) in [(4i16, 15u32), (8, 16)] {
        // A keyless 64 KiB segment of 64 KiB granularity.
        let args = [
            addis(3, 0, 0xFFFF),
            sldi32(3, 3),
            li(4, 0),
            oris(4, 4, 1),
            li(5, 0x200),
            li(6, OUT as i16 + slot),
        ];
        call(&mut p, sc::MMAPPER_ALLOCATE_SHARED_MEMORY, &args, rc);
    }
    p.push(lwz(20, OUT as i16, 0));
    p.push(lwz(21, OUT as i16 + 4, 0));
    p.push(lwz(22, OUT as i16 + 8, 0));
    let map = |at: u32, seg: u32| [at, addi(4, seg, 0), addis(5, 0, 4), li(6, 0)];
    call(
        &mut p,
        sc::MMAPPER_MAP_SHARED_MEMORY,
        &map(addi(3, 20, 0), 21),
        17,
    );
    call(
        &mut p,
        sc::MMAPPER_MAP_SHARED_MEMORY,
        &map(addis(3, 20, 1), 22),
        18,
    );
    // r23 is the edge between the two maps: the area base + 64 KiB.
    p.push(addis(23, 20, 1));
    for (d, value) in [
        (-8i16, 0x1819_1A1B),
        (-4, 0x1C1D_1E1F),
        (0, 0x2021_2223),
        (4, 0x2425_2627),
    ] {
        word(&mut p, 6, value);
        p.push(stw(6, d, 23));
    }
    p.push(ld(24, -4, 23));
    doubleword(&mut p, 7);
    p.push(std(7, -4, 23));
    for (rt, d) in [(25, -8i16), (26, -4), (27, 0), (28, 4)] {
        p.push(lwz(rt, d, 23));
    }
    exit(&mut p);
    p
}

#[test]
fn a_load_and_a_store_across_two_maps_end_to_end_complete_as_on_the_console() {
    let w = payload("ppu_span_shm_maps", 32);
    assert_eq!(w[0..2], [DONE, 0], "the main thread finished");
    assert_eq!(
        w[2..8],
        [0, 0x5000_0000, 0, 0, 0, 0],
        "the area, segments and maps"
    );
    assert_eq!(w[8..12], [0, 0, 1, 1], "the child passed both accesses");
    assert_eq!(w[14..17], [0, 0, CHILD_EXIT], "the child exited and joined");
    assert_eq!(w[21..26], [0; 5], "every unmap and free");

    let ran = run(
        vec![Region::new(0, 0x2_0000, "main", PageSize::Page64K)],
        &program(),
    );
    assert_eq!(ran.first_fault, None);
    let gpr = ran.gpr();
    let setup: Vec<u64> = [w[2], w[4], w[5], w[6], w[7]]
        .iter()
        .map(|&v| u64::from(v))
        .collect();
    assert_eq!(gpr[14..19], setup[..], "the area, segments and maps");
    assert_eq!(gpr[20], u64::from(w[3]), "the area base");
    assert_eq!(gpr[24], u64::from(w[12]) << 32 | u64::from(w[13]), "ld");
    let edge: Vec<u64> = w[17..21].iter().map(|&v| u64::from(v)).collect();
    assert_eq!(
        gpr[25..29],
        edge[..],
        "the words around the edge after the std"
    );
}
