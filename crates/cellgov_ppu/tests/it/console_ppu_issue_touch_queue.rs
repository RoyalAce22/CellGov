//! CellGov's PPU timing against the `ppu_issue_touch_queue` console
//! capture. The capture times a chase through memory in line order. T is
//! a touch of the line the next load reads, and S(n) is n touches of
//! L2-held lines in that line's L1 set:
//!
//! - loops 0 to 8: T, S(n) for n from 0 to 8, then the load;
//! - loops 9 to 12: T, S(n) for n from 3 to 6, a second T, the load;
//! - loops 13 to 16: S(3), S(5), S(6) or S(7), then the load;
//! - loops 17 to 19: T, 40 `mulld`, S(0), S(3) or S(7), the load;
//! - loop 20: T, S(3), 4 touches in the next 4 sets, the load;
//! - loops 21 and 22: T, S(4) or S(7) with 2 `mulld` after each touch,
//!   then the load.

use crate::console_ppu_issue_timing::{replay, Replay, Seed};
use crate::console_ppu_issue_touch_set_pressure as set_pressure;
use crate::console_ppu_issue_touch_set_pressure::{
    looped, CONFLICT, DCBT_R10, GROUP_BITS, IN_BUFFER, IN_GROUP, IN_ORDER_CHASE_GAP, LD, MEM_RING,
    NEXT_IN_SET, SET_BITS,
};
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_L1_SET_TOUCH_LIMIT};

const CONSOLE: &str = "../../tests/micro/ppu_issue_touch_queue/ps3/cech20-cex-493/observation.json";

/// The loops the capture times.
const LOOPS: usize = 23;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_5451;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u32 = 12;

/// `dcbt 0, r4`: the line the next load reads.
const T: u32 = 0x7c00_222c;
/// `addi r10, r10, 8320`: a line in the next L1 set.
const NEXT_SET: u32 = 0x394a_2080;
/// `mulld r12, r12, r15`, with r15 = 1.
const MULLD: u32 = 0x7d8c_79d2;
/// `ori 0, 0, 0`.
const NOP: u32 = 0x6000_0000;

const SEEDS: &[Seed] = &[
    Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, false),
    Seed::Gpr(20, CONFLICT),
    Seed::Gpr(12, 1),
    Seed::Gpr(15, 1),
];

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
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

/// The prologue and `n` touches of lines in r4's L1 set, each followed
/// by `after`.
fn same(n: usize, after: &[u32]) -> Vec<u32> {
    let mut w = vec![SET_BITS, GROUP_BITS, IN_GROUP, IN_BUFFER];
    for _ in 0..n {
        w.extend([DCBT_R10, NEXT_IN_SET]);
        w.extend_from_slice(after);
    }
    w
}

/// Loop `k`'s body, in the probe's order.
fn body(k: usize) -> Vec<u32> {
    let s = |n| same(n, &[]);
    let wait = vec![MULLD; 40];
    match k {
        0..=8 => [vec![T], s(k), vec![NOP, LD]].concat(),
        9..=12 => [vec![T], s(k - 6), vec![T, LD]].concat(),
        13..=16 => [s([3, 5, 6, 7][k - 13]), vec![LD]].concat(),
        17..=19 => [vec![T], wait, s([0, 3, 7][k - 17]), vec![NOP, LD]].concat(),
        20 => {
            let others: Vec<u32> = std::iter::once(NEXT_SET)
                .chain((0..4).flat_map(|_| [DCBT_R10, NEXT_SET]))
                .collect();
            [vec![T], s(3), others, vec![NOP, LD]].concat()
        }
        _ => [
            vec![T],
            same([4, 7][k - 21], &[MULLD, MULLD]),
            vec![NOP, LD],
        ]
        .concat(),
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

/// Asserts that loop `k` replays within `IN_ORDER_CHASE_GAP` of the
/// console's fastest run, and returns both, CellGov first.
fn assert_within(words: &[u64], k: usize) -> (f64, f64) {
    let (cellgov, console) = (cellgov_cycles(k), console_cycles(words, k));
    assert!(
        (console - cellgov).abs() / console <= IN_ORDER_CHASE_GAP,
        "loop {k}: CellGov {cellgov:.2} cycles a load, the console's fastest {console:.2}, \
         more than {IN_ORDER_CHASE_GAP} of it apart",
    );
    (cellgov, console)
}

/// The loop with T and S(n).
const fn touch_then(n: usize) -> usize {
    n
}

/// The loop with T, S(n) and a second T, for n from 3 to 6.
const fn touch_then_retouch(n: usize) -> usize {
    n + 6
}

/// The cycles a load that waits for its line's touch alone may take
/// above the loop with T and no later touch: half of one reload of an
/// L2-held line.
const ALONE_MARGIN: f64 = 24.0;

#[test]
fn the_recorded_touch_limit_is_the_capture() {
    let w = console_words();
    let alone = console_cycles(&w, touch_then(0)) + ALONE_MARGIN;
    let first_alone = (1..=8)
        .find(|&n| console_cycles(&w, touch_then(n)) <= alone)
        .expect("a count of later touches frees the load");
    assert_eq!(1 + first_alone, PPU_L1_SET_TOUCH_LIMIT);
}

#[test]
fn a_load_of_a_touched_line_waits_behind_up_to_six_later_touches_in_its_set_within_the_chase_spread(
) {
    let w = console_words();
    let (cellgov_alone, console_alone) = assert_within(&w, touch_then(0));
    for n in 1..PPU_L1_SET_TOUCH_LIMIT - 1 {
        let (cellgov, console) = assert_within(&w, touch_then(n));
        for (who, waits, alone) in [
            ("CellGov", cellgov, cellgov_alone),
            ("console", console, console_alone),
        ] {
            assert!(
                waits > alone + ALONE_MARGIN,
                "{who}: a touch of the line and {n} later touches in its set take {waits:.2} \
                 cycles a load, no more than {ALONE_MARGIN} above the touch alone ({alone:.2})",
            );
        }
    }
}

#[test]
fn a_load_of_a_touched_line_waits_for_its_touch_alone_once_its_set_holds_eight_touches() {
    let w = console_words();
    let (cellgov_alone, console_alone) = assert_within(&w, touch_then(0));
    let freed = [
        touch_then(PPU_L1_SET_TOUCH_LIMIT - 1),
        touch_then(PPU_L1_SET_TOUCH_LIMIT),
        touch_then_retouch(PPU_L1_SET_TOUCH_LIMIT - 2),
    ];
    for k in freed {
        let (cellgov, console) = assert_within(&w, k);
        for (who, freed, alone) in [
            ("CellGov", cellgov, cellgov_alone),
            ("console", console, console_alone),
        ] {
            assert!(
                freed <= alone + 2.0 * ALONE_MARGIN,
                "{who}: loop {k} takes {freed:.2} cycles a load, more than \
                 {} above the touch alone ({alone:.2})",
                2.0 * ALONE_MARGIN,
            );
        }
    }
}

#[test]
fn a_load_after_a_second_touch_of_its_line_waits_behind_three_to_five_later_touches_within_the_chase_spread(
) {
    let w = console_words();
    for n in 3..=5 {
        assert_within(&w, touch_then_retouch(n));
    }
}

/// The loop with S(n) and no T, for n of 3, 5, 6 or 7.
fn untouched_then(n: usize) -> usize {
    13 + [3, 5, 6, 7]
        .iter()
        .position(|&m| m == n)
        .expect("the capture times S(3), S(5), S(6) and S(7) alone")
}

/// The largest share of a load's cycles with S(n) alone that a touch of
/// its line may save it below the touch limit.
const NO_HELP_SHARE: f64 = 0.10;

#[test]
fn a_touch_of_the_loads_line_saves_it_almost_nothing_below_the_touch_limit_and_much_at_it() {
    let w = console_words();
    for n in [3, 5, 6] {
        let (cellgov_with, console_with) = assert_within(&w, touch_then(n));
        let (cellgov_without, console_without) = assert_within(&w, untouched_then(n));
        for (who, with, without) in [
            ("CellGov", cellgov_with, cellgov_without),
            ("console", console_with, console_without),
        ] {
            assert!(
                (without - with).abs() / without <= NO_HELP_SHARE,
                "{who}: S({n}) takes {without:.2} cycles a load, and {with:.2} after a touch \
                 of the load's line, more than {NO_HELP_SHARE} of it apart",
            );
        }
    }
    let n = PPU_L1_SET_TOUCH_LIMIT - 1;
    let (cellgov_with, console_with) = assert_within(&w, touch_then(n));
    let (cellgov_without, console_without) = assert_within(&w, untouched_then(n));
    for (who, with, without) in [
        ("CellGov", cellgov_with, cellgov_without),
        ("console", console_with, console_without),
    ] {
        assert!(
            with + 4.0 * ALONE_MARGIN < without,
            "{who}: S({n}) takes {without:.2} cycles a load, and {with:.2} after a touch of \
             the load's line, less than {} apart",
            4.0 * ALONE_MARGIN,
        );
    }
}

/// The loop with T, 40 `mulld` that let T return, and then S(n), for n
/// of 0, 3 or 7.
fn returned_then(n: usize) -> usize {
    17 + [0, 3, 7]
        .iter()
        .position(|&m| m == n)
        .expect("the capture times S(0), S(3) and S(7) after the wait")
}

/// The largest share of the cycles that later touches add to the
/// console's load after its line's touch returns that the model may
/// differ by.
const RETURNED_GAP: f64 = 0.25;

#[test]
fn after_the_touch_of_the_loads_line_returns_each_later_touch_adds_its_turn_to_the_load() {
    let w = console_words();
    let (cellgov_none, console_none) = assert_within(&w, returned_then(0));
    assert_within(&w, returned_then(3));
    let (cellgov_seven, console_seven) = assert_within(&w, returned_then(7));
    let (cellgov, console) = (cellgov_seven - cellgov_none, console_seven - console_none);
    assert!(
        (console - cellgov).abs() / console <= RETURNED_GAP,
        "7 later touches add {cellgov:.2} cycles to CellGov's load and {console:.2} to the \
         console's, more than {RETURNED_GAP} of it apart",
    );
}

#[test]
fn a_load_after_a_touch_of_its_line_and_three_or_four_later_touches_waits_for_them_in_the_set_pressure_capture(
) {
    let w = set_pressure::console_words();
    for k in 7..=8 {
        set_pressure::assert_within(&w, k);
    }
}

#[test]
fn touches_in_other_sets_and_spaced_touches_change_no_count_within_the_chase_spread() {
    let w = console_words();
    for k in 20..=22 {
        assert_within(&w, k);
    }
}
