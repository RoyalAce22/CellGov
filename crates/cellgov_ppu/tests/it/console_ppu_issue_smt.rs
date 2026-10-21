//! CellGov's PPU timing against the `ppu_issue_smt` console capture.
//!
//! The probe times eight loops with `mftb`, five runs each, alone and
//! beside busy sibling threads. In each phase with two siblings, one of
//! them never runs while the timed loop and the other sibling hold the
//! PPE's two hardware threads, so the timed loop runs at the two-thread
//! rate. The replay runs each loop beside as many PPU units as the
//! probe created siblings, and the model charges each running thread the
//! two-thread rate whatever the count. It holds each loop's cycles an
//! iteration no farther from the fastest run than the slowest run is.
//!
//! Loops 2 and 3, a not-taken `bne` paired with an `addi`, are not
//! replayed. Alone, that loop's fastest run took 41.9, 45.0 and 51.5
//! cycles an iteration in three captures of three builds, so it does
//! not answer the same way twice.

use crate::console_ppu_issue_timing::{replay, replay_of, Replay, Seed, BDNZ_8, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_smt/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 8] = [18, 18, 20, 20, 20, 20, 20, 20];

/// The payload's word 39 when the probe wrote every word.
const DONE: u64 = 0x5050_5354;

/// The thread exit value each sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 320);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[39], DONE, "the probe wrote every word");
    assert_eq!(words[38], 5, "five runs a loop");
    for k in 0..5 {
        assert_eq!(words[16 + k], 0, "sibling {k} was created");
        assert_eq!(words[21 + k], 0, "sibling {k} joined");
        assert_eq!(words[26 + k], SIBLING_EXIT, "sibling {k} exited");
    }
    // Bit k of word 36: sibling k reported it runs. The lone sibling
    // runs; of siblings 1 and 2, and of 3 and 4, exactly one runs, and
    // the other runs no round while the timed loops do.
    let ran = |k: usize| words[36] >> k & 1 == 1;
    assert!(ran(0), "the lone sibling ran: {:#x}", words[36]);
    for (a, b) in [(1, 2), (3, 4)] {
        assert!(
            ran(a) != ran(b),
            "exactly one of siblings {a} and {b} ran: {:#x}",
            words[36]
        );
        let idle = if ran(a) { b } else { a };
        assert_eq!(words[31 + idle], 0, "sibling {idle} ran no round");
    }
    words
}

/// Loop `k`'s fastest and slowest runs, in core cycles an iteration.
fn console_cycles(words: &[u64], k: usize) -> (f64, f64) {
    let per = |w: u64| (w * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64;
    (per(words[2 * k]), per(words[2 * k + 1]))
}

/// Four microcoded `lha`, each paired with an independent `addi`, then
/// the loop's `bdnz`, as the built probe lays them out.
const LHA_LOOP: [u32; 9] = [
    0xaa48_0000,
    0x39ce_0001,
    0xaa68_0008,
    0x39ef_0001,
    0xaa88_0010,
    0x3a10_0001,
    0xaaa8_0018,
    0x3a31_0001,
    BDNZ_8,
];

const LHA_REPLAY: Replay = Replay {
    loop_index: 0,
    words: &LHA_LOOP,
    seeds: &[Seed::Gpr(8, ONE_LINE)],
};

#[test]
fn the_issue_model_times_each_smt_loop_within_the_console_s_spread() {
    let w = console_words();
    // (loop, the loop it replays, PPU units beside it)
    let loops: [(usize, &Replay, usize); 6] = [
        (0, &LHA_REPLAY, 0),
        (1, &LHA_REPLAY, 1),
        (4, replay_of(1), 2),
        (5, replay_of(2), 2),
        (6, replay_of(1), 2),
        (7, replay_of(2), 2),
    ];
    let mut rows = Vec::new();
    for (k, r, siblings) in loops {
        let cellgov = replay(r, siblings).expect("the loop replays");
        let (fastest, slowest) = console_cycles(&w, k);
        rows.push((k, cellgov, fastest, slowest - fastest));
    }
    for &(k, cellgov, fastest, spread) in &rows {
        let within = (cellgov - fastest).abs() <= spread;
        if k == RESIDUAL_LOOP {
            assert!(
                !within,
                "loop {k} now lands within the console's spread; drop it from the \
                 residual: {rows:?}",
            );
            assert!(
                (cellgov - fastest).abs() < 1.0,
                "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
                 fastest {fastest:.3}: {rows:?}",
            );
        } else {
            assert!(
                within,
                "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
                 fastest {fastest:.3} with a spread of {spread:.3}: {rows:?}",
            );
        }
    }
}

#[test]
fn a_busy_sibling_holds_issue_after_the_instruction_after_each_microcoded_lha() {
    // Loop 1's runs spread over 15.9 cycles an iteration, so its own
    // spread does not separate a hold after the paired addi from none.
    // The cycles the sibling adds land within loop 0's spread.
    let w = console_words();
    let (alone, alone_slowest) = console_cycles(&w, 0);
    let (beside, _) = console_cycles(&w, 1);
    let added = replay(&LHA_REPLAY, 1).expect("the loop replays beside a sibling")
        - replay(&LHA_REPLAY, 0).expect("the loop replays alone");
    assert!(
        (added - (beside - alone)).abs() <= alone_slowest - alone,
        "the sibling adds {added:.3} cycles an iteration in CellGov and {:.3} on the \
         console, with loop 0 spread over {:.3}",
        beside - alone,
        alone_slowest - alone,
    );
}

/// The loop the model times outside the console's spread: dependent
/// `add` beside two siblings at the main thread's priority. Its fastest
/// run takes 23.93 cycles an iteration, 0.6 more than the same body
/// beside two lower-priority siblings, which the model times at 23 like
/// the two-thread loop. CellGov does not model what LV2 does among
/// threads of one priority.
const RESIDUAL_LOOP: usize = 7;
