//! CellGov's PPU timing against the `ppu_issue_touch_reload` console
//! capture. The capture times four loops of a chase through memory in
//! line order:
//!
//! - loop 0: the touch, then the load;
//! - loop 1: the touch, eight touches of other lines in its L1 set, then
//!   the load;
//! - loop 2: loop 1 with a second touch of the line before the load;
//! - loop 3: the eight touches and the load, with no touch of the line.
//!
//! The probe means the eight touches to push the first touch's copy out
//! of the L1 while the line is still on its way from memory. In CellGov,
//! the eight touches wait for the first touch's reload, which writes its
//! line first. The second touch finds the line on its way and sends no
//! request, so the line arrives when the first touch brings it.
//!
//! The tests do not assert loop 1. The console's load waits for the
//! eight touches and gets no help from the touch of its line. CellGov's
//! load waits for that touch alone. In the `ppu_issue_touch_queue`
//! capture, such a load waits for its set's later touch reloads until
//! the set holds 8 touches that missed the L1. Some
//! of the eight touches here hit the L1, and that capture does not show
//! whether a touch that hits counts. Loops 1 and 2 fit neither answer, so
//! CellGov keeps the wait for the touch alone after such a hit.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_touch_reload/ps3/cech20-cex-493/observation.json";

/// The payload's word 11 when the probe wrote every word.
const DONE: u64 = 0x5050_5452;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 12;

/// `ld r4, 0(r4)`.
const LD: u32 = 0xe884_0000;
/// `dcbt 0, r4`: the line the next load reads.
const DCBT_R4: u32 = 0x7c00_222c;
/// `rlwinm r10, r4, 0, 19, 24`: the L1 set bits of r4's line.
const SET_BITS: u32 = 0x548a_04f0;
/// `add r10, r10, r20`: the line of r20's buffer in the same L1 set.
const IN_BUFFER: u32 = 0x7d4a_a214;
/// `dcbt 0, r10`.
const DCBT_R10: u32 = 0x7c00_522c;
/// `addi r10, r10, 8192`: the next line in the same L1 set.
const NEXT_IN_SET: u32 = 0x394a_2000;
/// `ori 0, 0, 0`.
const NOP: u32 = 0x6000_0000;

/// Where the replay places the 8 MB ring and the conflict buffer.
const MEM_RING: u64 = 0x100_0000;
const CONFLICT: u64 = 0x80_0000;

const SEEDS: &[Seed] = &[
    Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, false),
    Seed::Gpr(20, CONFLICT),
];

/// The console's payload as big-endian words.
pub(crate) fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 96);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[11], DONE, "the probe wrote every word");
    assert_eq!((words[9], words[10]), (5, u64::from(ITERATIONS_LOG2)));
    words
}

/// Loop `k`'s fastest run, in core cycles a load.
pub(crate) fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    words.push(0x4200_0000 | ((4 * body.len() as u32).wrapping_neg() & 0xfffc));
    Box::leak(words.into_boxed_slice())
}

/// The eight touches of other lines in the L1 set of r4's line.
fn conflicts() -> Vec<u32> {
    let mut w = vec![SET_BITS, IN_BUFFER];
    for _ in 0..8 {
        w.extend([DCBT_R10, NEXT_IN_SET]);
    }
    w
}

/// Loop `k`'s body, in the probe's order.
fn body(k: usize) -> Vec<u32> {
    let c = conflicts();
    match k {
        0 => vec![DCBT_R4, LD],
        1 => [&[DCBT_R4][..], &c, &[NOP, LD]].concat(),
        2 => [&[DCBT_R4][..], &c, &[DCBT_R4, LD]].concat(),
        _ => [&c[..], &[LD]].concat(),
    }
}

/// CellGov's cycles a load for loop `k`.
pub(crate) fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&body(k)),
        seeds: SEEDS,
    };
    replay(&r, 0).expect("the loop replays")
}

/// The largest share of the console's fastest run the retouch loop may
/// differ by. A chase in line order runs faster on the console than the
/// model's memory latency gives, and the touch loop alone differs by
/// about 5%.
const RETOUCH_GAP: f64 = 0.05;

/// The gap of the touch loop alone, a chase in line order through memory.
pub(crate) const IN_ORDER_CHASE_GAP: f64 = 0.10;

pub(crate) fn assert_within(k: usize, cellgov: f64, console: f64, gap: f64) {
    assert!(
        (console - cellgov).abs() / console <= gap,
        "loop {k}: CellGov {cellgov:.2} cycles a load, the console's fastest {console:.2}, \
         more than {gap} of it apart",
    );
}

#[test]
fn a_second_touch_after_eight_touches_in_its_l1_set_brings_the_line_no_earlier_than_the_first() {
    let w = console_words();
    let (touch, retouch) = (cellgov_cycles(0), cellgov_cycles(2));
    let (console_touch, console_retouch) = (console_cycles(&w, 0), console_cycles(&w, 2));
    assert_within(0, touch, console_touch, IN_ORDER_CHASE_GAP);
    assert_within(2, retouch, console_retouch, RETOUCH_GAP);
    for (who, retouch, touch) in [
        ("CellGov", retouch, touch),
        ("console", console_retouch, console_touch),
    ] {
        assert!(
            retouch >= touch,
            "{who}: a load after a second touch takes {retouch:.2} cycles, \
             less than after the first touch alone ({touch:.2})",
        );
    }
}
