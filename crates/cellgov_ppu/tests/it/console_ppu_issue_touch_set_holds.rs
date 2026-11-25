//! CellGov's PPU timing against the `ppu_issue_touch_set_holds` console
//! capture. The capture times a chase through memory in line order with
//! a load in front of each load:
//!
//! - loops 0 and 1: 0 or 4 touches in the chase's L1 set;
//! - loops 2 and 3: a load of an L2-held line or of a line from memory
//!   in another set, after 4 touches of that set;
//! - loop 4: a load of a line from memory in the chase's set;
//! - loop 5: a load of a line from memory in another set;
//! - loops 6 and 7: an `ldarx` or an `ld` of an L2-held line in the
//!   chase's set.
//!
//! A load in the chase's set holds the set for its own trip, from the L2
//! or from memory, and the chase waits for it.
//!
//! On the console the chase also waits for a load from memory in another
//! set (loop 5), about as long as for one in its own set (loop 4). So
//! loop 4 does not tell the set's hold from that wait. CellGov does not
//! model the wait in another set, and the tests do not assert loops 2 and
//! 3, which rest on it.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use crate::console_ppu_issue_touch_set_forms::{
    console_cycles, console_words, prologue, touches, LD_R10,
};
use crate::console_ppu_issue_touch_set_pressure::{
    looped, CONFLICT, IN_ORDER_CHASE_GAP, LD, MEM_RING,
};

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_touch_set_holds/ps3/cech20-cex-493/observation.json";

/// The loops the capture times.
const LOOPS: usize = 8;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5448;

/// The ring's bytes.
const RING_BYTES: u64 = 8 * 1024 * 1024;

/// `addis r12, r4, 64`: r4 + 4 MB, a line in r4's L1 set.
const MEM: u32 = 0x3d84_0040;
/// `addi r12, r12, 4096`: a line 32 L1 sets from r12's.
const MEM_FAR: u32 = 0x398c_1000;
/// `ld r5, 0(r12)`.
const LD_R12: u32 = 0xe8ac_0000;
/// `ldarx r5, 0, r10`.
const LDARX_R10: u32 = 0x7ca0_50a8;

/// The ring, and a second ring after it that only maps the 4 MB and
/// 64 KB that r4 + 4 MB + 4 KB reaches past the first. Nothing reads r30.
const SEEDS: &[Seed] = &[
    Seed::Ring(4, MEM_RING, RING_BYTES, false),
    Seed::Ring(
        30,
        MEM_RING + RING_BYTES,
        4 * 1024 * 1024 + 64 * 1024,
        false,
    ),
    Seed::Gpr(20, CONFLICT),
];

/// Loop `k`'s body, in the probe's order, for the loops the tests replay.
fn body(k: usize) -> Vec<u32> {
    match k {
        0 => [prologue(), vec![LD]].concat(),
        1 => [prologue(), touches(4), vec![LD]].concat(),
        4 => vec![MEM, LD_R12, LD],
        5 => vec![MEM, MEM_FAR, LD_R12, LD],
        6 => [prologue(), vec![LDARX_R10, LD]].concat(),
        7 => [prologue(), vec![LD_R10, LD]].concat(),
        _ => unreachable!("loop {k} is not replayed"),
    }
}

/// CellGov's cycles a load for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&body(k)),
        seeds: SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

#[test]
fn a_load_or_an_ldarx_in_the_chases_set_delays_the_chase_by_its_own_trip() {
    let w = console_words(CONSOLE, LOOPS, DONE);
    let loops = [0, 1, 4, 6, 7];
    let cellgov = loops.map(cellgov_cycles);
    let console = loops.map(|k| console_cycles(&w, k));
    for ((k, g), c) in loops.iter().zip(cellgov).zip(console) {
        assert!(
            (c - g).abs() / c <= IN_ORDER_CHASE_GAP,
            "loop {k}: CellGov {g:.2} cycles a load, the console's fastest {c:.2}, \
             more than {IN_ORDER_CHASE_GAP} of it apart",
        );
    }
    for (who, [none, _, memory, ldarx, l2]) in [("CellGov", cellgov), ("console", console)] {
        let (memory, ldarx, l2) = (memory - none, ldarx - none, l2 - none);
        assert!(
            memory > 6.0 * l2,
            "{who}: a load from memory in the chase's set adds {memory:.2} cycles, \
             no more than 6 times the {l2:.2} that a load from the L2 adds",
        );
        assert!(
            ldarx >= 0.75 * l2,
            "{who}: an ldarx in the chase's set adds {ldarx:.2} cycles, \
             under 3/4 of the {l2:.2} that an ld adds",
        );
    }
}

#[test]
fn a_load_from_memory_in_a_free_set_delays_the_chase_on_the_console_and_not_in_cellgov() {
    let w = console_words(CONSOLE, LOOPS, DONE);
    let [none, same, other] = [0, 4, 5].map(|k| console_cycles(&w, k));
    assert!(
        other - none > 0.75 * (same - none),
        "console: a load from memory in another set adds {:.2} cycles to the chase, \
         under 3/4 of the {:.2} one in its own set adds",
        other - none,
        same - none,
    );
    let [none, same, other] = [0, 4, 5].map(cellgov_cycles);
    assert!(
        other - none < 0.1 * (same - none),
        "CellGov: a load from memory in another set adds {:.2} cycles to the chase. \
         CellGov lets the two misses overlap, and the console does not; this test \
         pins CellGov's overlap until the model makes the chase wait",
        other - none,
    );
}
