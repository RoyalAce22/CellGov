//! CellGov's PPU timing against the `ppu_issue_mfvscr_stores` console
//! capture: how the count and the addresses of the stores at the end of an
//! `mfvscr`'s held work set the next `mfvscr`'s wait.
//!
//! The probe times 16 loops with `mftb`, five runs each. Each loop is an
//! `mfvscr`, 16 `li` and its stores, on a 128-byte boundary.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_mfvscr_stores/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 16;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_4d53;

/// log2 of each loop's iterations on the console: the probe's `ITER`.
const ITERATIONS_LOG2: u32 = 16;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let words: Vec<u64> = observation.memory_regions[0]
        .data
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 2 * LOOPS + 3);
    assert_eq!(words[2 * LOOPS + 2], DONE, "the probe wrote every word");
    assert_eq!(
        (words[2 * LOOPS], words[2 * LOOPS + 1]),
        (5, u64::from(ITERATIONS_LOG2))
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64
}

/// `mfvscr v3`.
const MFVSCR: u32 = 0x1060_0604;
/// `li r3, 1` to `li r10, 1`.
const LI: [u32; 8] = [
    0x3860_0001,
    0x3880_0001,
    0x38a0_0001,
    0x38c0_0001,
    0x38e0_0001,
    0x3900_0001,
    0x3920_0001,
    0x3940_0001,
];

/// `std r14, d(r29)`.
const fn std(d: u16) -> u32 {
    0xf9dd_0000 | d as u32
}

/// `stw r14, d(r29)`.
const fn stw(d: u16) -> u32 {
    0x91dd_0000 | d as u32
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// `n` `li`, cycling r3 to r10.
fn li(n: usize) -> Vec<u32> {
    LI.iter().copied().cycle().take(n).collect()
}

/// The words after each loop's `mfvscr` and 16 `li`, in the probe's order.
fn tails() -> [Vec<u32>; LOOPS] {
    let run = |n: u16| (0..n).map(|i| std(8 * i)).collect::<Vec<_>>();
    [
        run(1),
        run(2),
        vec![std(0), std(16)],
        vec![std(0), std(64)],
        vec![std(0), std(128)],
        run(3),
        run(4),
        run(5),
        run(6),
        run(8),
        vec![std(0), std(0)],
        vec![std(0); 4],
        vec![stw(0), stw(4)],
        [&[std(0)][..], &li(1), &[std(8)]].concat(),
        [&[std(0)][..], &li(4), &[std(8)]].concat(),
        [&run(2)[..], &li(4)].concat(),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let body = [&[MFVSCR][..], &li(16), &tails()[k]].concat();
    let r = Replay {
        loop_index: k,
        words: looped(&body),
        seeds: &[Seed::Gpr(29, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops the model does not hold, each under the console:
///
/// - `std 0; li; std 8`: the console takes 63.36 cycles an iteration and
///   the model gives 53.
/// - `std 0; std 8; 4 li`: 55.96 and 52.
///
/// With one store, `li` after it cost the next `mfvscr` nothing until
/// they outlast the store's wait. With two stores, the four `li` after
/// them cost the console 4 cycles, and one `li` between them 11.
const UNDER: [usize; 2] = [13, 15];

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    let held: Vec<usize> = (0..LOOPS).filter(|k| !UNDER.contains(k)).collect();
    assert_eq!(held.len(), LOOPS - UNDER.len());
    let misses: Vec<String> = held
        .into_iter()
        .map(|k| (k, cellgov_cycles(k), console_cycles(&w, k)))
        .filter(|&(_, cellgov, console)| (cellgov - console).abs() / console > BAND)
        .map(|(k, cellgov, console)| {
            format!(
                "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
                 {console:.2}"
            )
        })
        .collect();
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

#[test]
fn a_second_store_adds_four_cycles_each_later_one_one_and_a_shared_doubleword_four() {
    // Loop 0 ends with one store. Loops 1 and 9 end with 2 and 8 stores to
    // other doublewords. Loop 10 ends with 2 stores to one doubleword. The
    // console's steps are 3.99, 6.00 and 4.14 cycles.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        let (one, two, eight, shared) = (cycles(0), cycles(1), cycles(9), cycles(10));
        assert!(
            (two - one - 4.0).abs() < 0.5,
            "{who}: {one:.2} cycles an iteration with 1 store, {two:.2} with 2",
        );
        assert!(
            (eight - two - 6.0).abs() < 0.5,
            "{who}: {two:.2} cycles an iteration with 2 stores, {eight:.2} with 8",
        );
        assert!(
            (shared - two - 4.0).abs() < 0.5,
            "{who}: {shared:.2} cycles an iteration with 2 stores to one doubleword, \
             {two:.2} to two",
        );
    }
}

/// The test fails when the model comes within the band of the console.
/// Then remove the loop from `UNDER`.
#[test]
fn each_li_with_two_stores_loop_is_a_known_difference() {
    let w = console_words();
    for k in UNDER {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov < (1.0 - BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}
