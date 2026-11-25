//! CellGov's PPU timing against the `ppu_issue_touch_set_split` console
//! capture. Each pass of a loop works in the L1 set after the last pass's,
//! and loads an L2-held line, then runs 128 independent `addi`:
//!
//! - loop 0: 2 touches in the set, then an `ld` across a 32-byte sector;
//! - loop 1: 2 touches in the set, then an `ld` in one sector;
//! - loops 2 and 3: the same loads with the 2 touches after them.
//!
//! An integer load across a sector holds the instructions after it for
//! the sector, as [CBE-Handbook p:730 s:A.1.3.2] gives. When it waits for
//! its L1 set first, it holds them for that wait too: loop 0 less loop 1
//! is about the 2 touches' hold more than loop 2 less loop 3.
//!
//! On the console a sector-crossing load that misses the L1 costs more
//! than CellGov charges, and the 128 `addi` take longer than CellGov
//! gives. The tests therefore compare differences of loops, and name the
//! sector cost apart.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use crate::console_ppu_issue_touch_set_forms::{console_cycles, console_words, touches};
use crate::console_ppu_issue_touch_set_pressure::{looped, CONFLICT, IN_BUFFER, IN_GROUP};
use cellgov_ps3_abi::hw::ppu_issue::PPU_SECTOR_CROSS_LOAD_CYCLES;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_touch_set_split/ps3/cech20-cex-493/observation.json";

/// The loops the capture times.
const LOOPS: usize = 4;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5458;

/// `rlwinm r10, r23, 0, 19, 24`: the L1 set bits of r23.
const SET_BITS: u32 = 0x56ea_04f0;
/// `rlwinm r11, r23, 3, 14, 15`: r23's bits 13 and 14 as a 64 KB group.
const GROUP_BITS: u32 = 0x56eb_1b9e;
/// `ld r5, 28(r10)`: bytes 28 to 35 of r10's line.
const SPLIT: u32 = 0xe8aa_001c;
/// `ld r5, 24(r10)`: bytes 24 to 31 of r10's line.
const WHOLE: u32 = 0xe8aa_0018;
/// `ld r5, 16412(r10)`: bytes 28 to 35 of the line two lines on in the set.
const SPLIT_AHEAD: u32 = 0xe8aa_401c;
/// `ld r5, 16408(r10)`: bytes 24 to 31 of that line.
const WHOLE_AHEAD: u32 = 0xe8aa_4018;
/// `addi r24, r24, 1` to `addi r27, r27, 1`.
const ADDS: [u32; 4] = [0x3b18_0001, 0x3b39_0001, 0x3b5a_0001, 0x3b7b_0001];
/// `addi r23, r23, 128`: the next pass's set.
const NEXT_PASS: u32 = 0x3af7_0080;

/// r23 starts at 0. A ring over the buffer only maps it; nothing reads
/// r30.
const SEEDS: &[Seed] = &[
    Seed::Ring(30, CONFLICT, 512 * 1024, false),
    Seed::Gpr(20, CONFLICT),
    Seed::Gpr(23, 0),
];

/// The largest share of the console's figure that the extra cost of the
/// sector after the set's wait may differ by.
const SET_WAIT_GAP: f64 = 0.15;

/// Loop `k`'s body, in the probe's order.
fn body(k: usize) -> Vec<u32> {
    let head = vec![SET_BITS, GROUP_BITS, IN_GROUP, IN_BUFFER];
    let tail = [ADDS.repeat(32), vec![NEXT_PASS]].concat();
    match k {
        0 => [head, touches(2), vec![SPLIT], tail].concat(),
        1 => [head, touches(2), vec![WHOLE], tail].concat(),
        2 => [head, vec![SPLIT_AHEAD], touches(2), tail].concat(),
        _ => [head, vec![WHOLE_AHEAD], touches(2), tail].concat(),
    }
}

/// CellGov's cycles a pass for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&body(k)),
        seeds: SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

#[test]
fn a_sector_crossing_load_that_waits_for_its_set_holds_the_instructions_after_it_for_the_wait() {
    let w = console_words(CONSOLE, LOOPS, DONE);
    let [cellgov, console] = [
        [0, 1, 2, 3].map(cellgov_cycles),
        [0, 1, 2, 3].map(|k| console_cycles(&w, k)),
    ]
    .map(|[held_split, held_whole, free_split, free_whole]| {
        (held_split - held_whole) - (free_split - free_whole)
    });
    assert!(
        (cellgov - console).abs() <= SET_WAIT_GAP * console,
        "the sector costs {cellgov:.2} cycles more after the set's wait in CellGov, \
         {console:.2} on the console, more than {SET_WAIT_GAP} of it apart",
    );
}

#[test]
fn a_sector_crossing_load_that_misses_the_l1_costs_the_console_more_than_cellgov_charges() {
    let w = console_words(CONSOLE, LOOPS, DONE);
    let sector = PPU_SECTOR_CROSS_LOAD_CYCLES as f64;
    let console = console_cycles(&w, 2) - console_cycles(&w, 3);
    assert!(
        console > 2.0 * sector,
        "console: the sector costs a load that misses the L1 {console:.2} cycles, \
         no more than twice the {sector} it costs a load that hits",
    );
    let cellgov = cellgov_cycles(2) - cellgov_cycles(3);
    assert!(
        cellgov < 1.25 * sector,
        "CellGov: the sector costs a load that misses the L1 {cellgov:.2} cycles. \
         CellGov charges a miss what the sector costs a load that hits, and the \
         console charges more; this test pins CellGov's charge until the model \
         charges a miss more",
    );
}
