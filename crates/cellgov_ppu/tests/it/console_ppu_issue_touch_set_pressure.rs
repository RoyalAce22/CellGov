//! CellGov's PPU timing against the `ppu_issue_touch_set_pressure`
//! console capture. The capture times a chase through memory in line
//! order after touches of L2-held lines:
//!
//! - loops 0 to 5: 0, 1, 2, 4, 6 or 8 touches in the load's L1 set;
//! - loop 6: 8 touches spread over 8 L1 sets, the first the load's;
//! - loops 7 to 10: a touch of the load's line, then 3, 4, 7 or 8
//!   touches in its set;
//! - loop 11: a touch of the load's line, then the 8 spread touches;
//! - loop 12: the 8 touches in the set, then a touch of the load's line.
//!
//! Each touch reload holds its L1 set, and a later miss to the set waits
//! for it. Touches spread over the sets wait for no reload.
//!
//! The `console_ppu_issue_touch_queue` tests assert loops 7 and 8, beside
//! the capture that settles how many later touches a load waits behind.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_touch_set_pressure/ps3/cech20-cex-493/observation.json";

/// The loops the capture times.
const LOOPS: usize = 13;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5453;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 12;

/// `ld r4, 0(r4)`.
pub(crate) const LD: u32 = 0xe884_0000;
/// `dcbt 0, r4`: the line the next load reads.
const DCBT_R4: u32 = 0x7c00_222c;
/// `rlwinm r10, r4, 0, 19, 24`: the L1 set bits of r4's line.
pub(crate) const SET_BITS: u32 = 0x548a_04f0;
/// `rlwinm r11, r4, 3, 14, 15`: r4's address bits 13 and 14 as a 64 KB
/// group.
pub(crate) const GROUP_BITS: u32 = 0x548b_1b9e;
/// `add r10, r10, r11`.
pub(crate) const IN_GROUP: u32 = 0x7d4a_5a14;
/// `add r10, r10, r20`: the line of r20's buffer in the same L1 set.
pub(crate) const IN_BUFFER: u32 = 0x7d4a_a214;
/// `dcbt 0, r10`.
pub(crate) const DCBT_R10: u32 = 0x7c00_522c;
/// `addi r10, r10, 8192`: the next line in the same L1 set.
pub(crate) const NEXT_IN_SET: u32 = 0x394a_2000;
/// `addi r10, r10, 8320`: a line in the next L1 set.
const NEXT_SET: u32 = 0x394a_2080;
/// `ori 0, 0, 0`.
const NOP: u32 = 0x6000_0000;

/// Where the replay places the 8 MB ring and the 64 KB-aligned conflict
/// buffer.
pub(crate) const MEM_RING: u64 = 0x100_0000;
pub(crate) const CONFLICT: u64 = 0x80_0000;

const SEEDS: &[Seed] = &[
    Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, false),
    Seed::Gpr(20, CONFLICT),
];

/// The console's payload as big-endian words.
pub(crate) fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 8 * (2 * LOOPS + 4));
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[2 * LOOPS + 3], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS + 1], words[2 * LOOPS + 2]),
        (5, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest run, in core cycles a load.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `body` closed by its `bdnz`.
pub(crate) fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// The prologue and `n` touches of buffer lines `step` bytes apart.
fn touches(n: usize, step: u32) -> Vec<u32> {
    let mut w = vec![SET_BITS, GROUP_BITS, IN_GROUP, IN_BUFFER];
    for _ in 0..n {
        w.extend([DCBT_R10, step]);
    }
    w
}

/// Loop `k`'s body, in the probe's order.
fn body(k: usize) -> Vec<u32> {
    let same = |n| touches(n, NEXT_IN_SET);
    let spread = touches(8, NEXT_SET);
    match k {
        0..=5 => [same([0, 1, 2, 4, 6, 8][k]), vec![LD]].concat(),
        6 => [spread, vec![LD]].concat(),
        7..=10 => [vec![DCBT_R4], same([3, 4, 7, 8][k - 7]), vec![NOP, LD]].concat(),
        11 => [vec![DCBT_R4], spread, vec![NOP, LD]].concat(),
        _ => [same(8), vec![DCBT_R4, LD]].concat(),
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

/// The largest share of the console's fastest run a loop may differ by.
/// A chase in line order runs faster on the console than the model's
/// memory latency gives: the chase with no touch differs by about 9%.
pub(crate) const IN_ORDER_CHASE_GAP: f64 = 0.10;

/// Asserts that loop `k` replays within `IN_ORDER_CHASE_GAP` of the
/// console's fastest run, and returns both, CellGov first.
pub(crate) fn assert_within(words: &[u64], k: usize) -> (f64, f64) {
    let (cellgov, console) = (cellgov_cycles(k), console_cycles(words, k));
    assert!(
        (console - cellgov).abs() / console <= IN_ORDER_CHASE_GAP,
        "loop {k}: CellGov {cellgov:.2} cycles a load, the console's fastest {console:.2}, \
         more than {IN_ORDER_CHASE_GAP} of it apart",
    );
    (cellgov, console)
}

#[test]
fn touches_in_the_loads_l1_set_delay_the_load_within_the_chase_spread() {
    let w = console_words();
    for k in 0..=5 {
        assert_within(&w, k);
    }
}

#[test]
fn touches_spread_over_the_sets_delay_the_load_far_less_than_touches_in_its_set() {
    let w = console_words();
    let (cellgov_none, console_none) = assert_within(&w, 0);
    let (cellgov_same, console_same) = assert_within(&w, 5);
    let (cellgov_spread, console_spread) = assert_within(&w, 6);
    for (who, none, same, spread) in [
        ("CellGov", cellgov_none, cellgov_same, cellgov_spread),
        ("console", console_none, console_same, console_spread),
    ] {
        assert!(
            same - none > 4.0 * (spread - none),
            "{who}: 8 touches in the load's set add {:.2} cycles, \
             no more than 4 times the {:.2} that 8 spread touches add",
            same - none,
            spread - none,
        );
    }
}

#[test]
fn a_load_after_a_touch_of_its_line_and_seven_or_eight_later_touches_waits_for_the_line() {
    let w = console_words();
    for k in 9..=11 {
        assert_within(&w, k);
    }
}

#[test]
fn a_touch_of_the_loads_line_after_eight_touches_in_its_set_waits_for_their_reloads() {
    assert_within(&console_words(), 12);
}
