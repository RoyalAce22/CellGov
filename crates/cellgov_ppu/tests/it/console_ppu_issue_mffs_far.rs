//! CellGov's PPU timing against the `ppu_issue_mffs_far` console capture:
//! an `fadd` or an `stfd` 20 to 48 `li` from an `mffs`, before or after
//! it, and the `li` alone.
//!
//! The probe times 16 loops with `mftb`, five runs each. Each loop starts
//! on a 32-byte boundary.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_mffs_far/ps3/cech20-cex-493/observation.json";

/// The loops the probe times.
const LOOPS: usize = 16;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5050_465a;

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

/// `mffs f0`.
const MFFS: u32 = 0xfc00_048e;
/// `fadd f1, f14, f14`.
const FADD: u32 = 0xfc2e_702a;
/// `stfd f14, 0(r11)`.
const STFD: u32 = 0xd9cb_0000;
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

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; LOOPS] {
    let before = |n, op| [&li(n)[..], &[op, MFFS]].concat();
    let after = |n, op| [&[MFFS][..], &li(n), &[op]].concat();
    [
        before(24, FADD),
        before(24, STFD),
        before(32, FADD),
        before(32, STFD),
        after(24, FADD),
        after(24, STFD),
        after(32, FADD),
        after(32, STFD),
        before(48, FADD),
        before(48, STFD),
        after(48, FADD),
        after(48, STFD),
        after(20, FADD),
        after(20, STFD),
        [&[MFFS][..], &li(32)].concat(),
        li(48),
    ]
}

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let r = Replay {
        loop_index: k,
        words: looped(&bodies()[k]),
        seeds: &[Seed::Gpr(11, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The share of a loop's fastest console run the model may lie on either
/// side of it.
const BAND: f64 = 0.06;

/// The loops the model holds: an `mffs` and 20 `li` before an `fadd` or
/// an `stfd`, and an `mffs` and 32 `li` alone.
const HELD: [usize; 3] = [12, 13, 14];

/// The loops with 24 to 48 `li` between an `mffs` and an `fadd` or an
/// `stfd`, which the model gives more cycles than the console.
///
/// The console takes 42.00 to 44.00 cycles an iteration with 24 or 32
/// `li`, and 53.00 to 55.26 with 48. The model times each `li` after the
/// older `mffs` at one cycle, and the next `mffs` waits
/// `PPU_MFFS_VSU_COMPLETE_CYCLES` past the `fadd` or the `stfd`. So it
/// gives 52 to 62 with 24 or 32 `li`, and 76 or 78 with 48.
const OVER: [usize; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];

/// 48 `li` alone. The console takes 56.77 cycles an iteration and the
/// model gives 49.
const UNDER: usize = 15;

#[test]
fn each_held_loop_lies_within_the_band() {
    let w = console_words();
    for k in HELD {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            (cellgov - console).abs() / console <= BAND,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
}

#[test]
fn an_stfd_far_from_an_mffs_takes_the_cycles_of_an_fadd() {
    // Each pair is the same loop with an `fadd`, then with an `stfd`.
    let w = console_words();
    for (who, cycles) in [
        (
            "the console",
            &(|k| console_cycles(&w, k)) as &dyn Fn(usize) -> f64,
        ),
        ("CellGov", &cellgov_cycles),
    ] {
        for fadd in (0..14).step_by(2) {
            let (f, s) = (cycles(fadd), cycles(fadd + 1));
            assert!(
                (s - f).abs() / f <= BAND,
                "{who}: loop {fadd} {f:.2} cycles an iteration, loop {} {s:.2}",
                fadd + 1,
            );
        }
    }
}

/// When the model times a loop within the band of the console, move the
/// loop into `HELD`.
#[test]
fn each_other_loop_is_a_known_difference() {
    let w = console_words();
    assert_eq!(HELD.len() + OVER.len() + 1, LOOPS);
    for k in OVER {
        let (cellgov, console) = (cellgov_cycles(k), console_cycles(&w, k));
        assert!(
            cellgov > (1.0 + BAND) * console,
            "loop {k}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
             {console:.2}",
        );
    }
    let (cellgov, console) = (cellgov_cycles(UNDER), console_cycles(&w, UNDER));
    assert!(
        cellgov < (1.0 - BAND) * console,
        "loop {UNDER}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
         {console:.2}",
    );
}
