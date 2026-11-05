//! CellGov's PPU timing against the `ppu_issue_stmw_words` console
//! capture: a store multiple at 1 to 18 words, alone and with eight
//! independent `addi` after it.
//!
//! Every store multiple writes from 0(r12), the start of one line, as the
//! built probe lays it out.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_stmw_words/ps3/cech20-cex-493/observation.json";

/// The payload's word 26 when the probe wrote every word.
const DONE: u64 = 0x5050_5357;

/// log2 of each loop's iterations on the console.
const ITERATIONS_LOG2: u64 = 18;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 216);
    let w: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(w[26], DONE, "the probe wrote every word");
    assert_eq!((w[24], w[25]), (5, ITERATIONS_LOG2));
    w
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(w: &[u64], k: usize) -> (f64, f64) {
    let per = |t: u64| (t * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2) as f64;
    (per(w[2 * k]), per(w[2 * k + 1]))
}

/// `stmw rs,0(r12)`.
const fn stmw(rs: u32) -> u32 {
    0xbc0c_0000 | rs << 21
}

/// `addi rn,rn,1` for r3 through r10.
const ADDI8: [u32; 8] = [
    0x3863_0001,
    0x3884_0001,
    0x38a5_0001,
    0x38c6_0001,
    0x38e7_0001,
    0x3908_0001,
    0x3929_0001,
    0x394a_0001,
];

/// `std r31,0(r12)`.
const STD: u32 = 0xfbec_0000;

/// Each loop's body before its `bdnz`, in the probe's order.
fn bodies() -> [Vec<u32>; 12] {
    let with_addi = |rs| [&[stmw(rs)][..], &ADDI8].concat();
    [
        vec![stmw(31)],
        vec![stmw(30)],
        vec![stmw(28)],
        vec![stmw(24)],
        vec![stmw(16)],
        vec![stmw(14)],
        with_addi(28),
        with_addi(24),
        with_addi(16),
        with_addi(14),
        vec![STD],
        ADDI8.to_vec(),
    ]
}

/// The words a loop stores, or 0 for the loops without a store multiple.
const STORED_WORDS: [u32; 12] = [1, 2, 4, 8, 16, 18, 4, 8, 16, 18, 0, 0];

/// CellGov's cycles an iteration for loop `k`.
fn cellgov_cycles(k: usize) -> f64 {
    let mut words = bodies()[k].clone();
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: &[Seed::Gpr(12, ONE_LINE)],
    };
    replay(&r, 0).expect("the loop replays")
}

/// The loops the model times outside the console's spread, each with the
/// cycles an iteration it may stay apart by. Each is a store multiple of
/// 16 or 18 words, which fills four or five store-queue entries. From 8 to
/// 16 words the console adds about 2 cycles a word, and the 18-word loop
/// takes 27 more than the 16-word one. The model adds 1 a word.
const RESIDUALS: [(usize, f64); 3] = [(4, 11.0), (5, 36.0), (9, 22.0)];

/// The loops whose console runs spread wider than the model stays apart
/// from the fastest run, each with the cycles an iteration it may stay
/// apart by. Loop 8, the 16-word store multiple with the `addi`, has runs
/// that spread over about 30 cycles. The model runs short of the fastest,
/// as it does for the 16- and 18-word loops in `RESIDUALS`.
const WIDE_SPREAD: [(usize, f64); 1] = [(8, 17.0)];

#[test]
fn each_store_multiple_loop_lies_within_the_console_s_spread_or_its_named_residual() {
    let w = console_words();
    let rows: Vec<(usize, f64, f64, f64)> = (0..bodies().len())
        .map(|k| {
            let (fastest, slowest) = console_cycles(&w, k);
            (k, cellgov_cycles(k), fastest, slowest - fastest)
        })
        .collect();
    for &(k, cellgov, fastest, spread) in &rows {
        let apart = (cellgov - fastest).abs();
        match RESIDUALS.iter().find(|r| r.0 == k) {
            Some(&(_, bound)) => {
                assert!(
                    apart > spread,
                    "loop {k} now lands within the console's spread; drop it from the \
                     residuals: {rows:?}",
                );
                assert!(
                    apart < bound,
                    "loop {k} ({} words): CellGov {cellgov:.2} cycles an iteration, the \
                     console's fastest {fastest:.2}, more than {bound} apart",
                    STORED_WORDS[k],
                );
            }
            None => assert!(
                apart <= spread,
                "loop {k} ({} words): CellGov {cellgov:.2} cycles an iteration, the \
                 console's fastest {fastest:.2} with a spread of {spread:.2}",
                STORED_WORDS[k],
            ),
        }
        if let Some(&(_, bound)) = WIDE_SPREAD.iter().find(|r| r.0 == k) {
            assert!(
                bound < spread,
                "loop {k}: the console's spread of {spread:.2} is no wider than {bound}; \
                 drop it from the wide-spread loops: {rows:?}",
            );
            assert!(
                apart < bound,
                "loop {k} ({} words): CellGov {cellgov:.2} cycles an iteration, the \
                 console's fastest {fastest:.2}, more than {bound} apart",
                STORED_WORDS[k],
            );
        }
    }
}

/// The loops of one `stmw` alone, at 1, 2, 4 and 8 words.
const SHORT: [usize; 4] = [0, 1, 2, 3];

#[test]
fn a_short_store_multiple_costs_about_a_cycle_a_word_over_its_fixed_cost() {
    let w = console_words();
    for (who, cycles) in [
        ("CellGov", SHORT.map(cellgov_cycles)),
        ("console", SHORT.map(|k| console_cycles(&w, k).0)),
    ] {
        let per_word = (cycles[3] - cycles[0]) / 7.0;
        assert!(
            (1.0..1.3).contains(&per_word),
            "{who}: {per_word:.2} cycles a word from 1 to 8 words: {cycles:?}",
        );
    }
}
