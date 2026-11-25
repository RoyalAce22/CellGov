//! CellGov's PPU timing against two console captures of loops beside a
//! lower-priority sibling of back-to-back dependent `fdiv`:
//!
//! - `ppu_issue_fdiv_window`: loops of 0 to 16 independent `addi` and a
//!   `bdnz`;
//! - `ppu_issue_fdiv_chain`: loops of 8 or 4 dependent `add`, of 8
//!   independent `addi`, and of a `bdnz`, beside the same sibling and
//!   beside a sibling that runs the same divides in a C loop.
//!
//! For each run that counts, each probe also keeps the sibling's rounds
//! over that run. In most runs the loop takes a fixed number of the
//! sibling's divide periods an iteration. The other runs are faster, and
//! in many of them the sibling kept its full rate. The tests hold the
//! model to the median run. For some loops the median lies under that
//! fixed cost: 12% under for the loop of 12 `addi`.

use crate::console_ppu_issue_shared::{c_round, eight, round_seeds};
use crate::console_ppu_issue_sibling_rate::SIBLINGS;
use crate::console_ppu_issue_timing::{replay_beside_for, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const WINDOW: &str = "../../tests/micro/ppu_issue_fdiv_window/ps3/cech20-cex-493/observation.json";
const CHAIN: &str = "../../tests/micro/ppu_issue_fdiv_chain/ps3/cech20-cex-493/observation.json";

/// The thread exit value each probe's sibling passes.
const SIBLING_EXIT: u64 = 0x5157;

/// The probes' counted runs of each loop.
const REPS: u64 = 15;

/// Each run's iterations on the console.
const ITERATIONS: u64 = 2048;

/// The iterations a replay times, after as many that warm it: enough for
/// a loop of one `bdnz` to span several of the sibling's divides.
const REPLAY_ITERATIONS: u64 = 1024;

/// The `addi` each `ppu_issue_fdiv_window` loop carries.
const K: [usize; 9] = [0, 1, 2, 3, 4, 6, 8, 12, 16];

/// `addi r17, r17, 1` to `addi r24, r24, 1`.
pub(crate) const ADDI8: [u32; 8] = [
    0x3a31_0001,
    0x3a52_0001,
    0x3a73_0001,
    0x3a94_0001,
    0x3ab5_0001,
    0x3ad6_0001,
    0x3af7_0001,
    0x3b18_0001,
];

/// `add r17, r17, r15`.
const ADD: u32 = 0x7e31_7a14;

/// The console's payload at `path` as big-endian words, `len` bytes.
pub(crate) fn words(path: &str, len: usize) -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation
        .memory_regions
        .first()
        .unwrap_or_else(|| panic!("{}: no memory region", path.display()))
        .data;
    assert_eq!(p.len(), len, "{}: the payload's bytes", path.display());
    p.chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

/// The `ppu_issue_fdiv_window` payload, checked whole.
fn window_words() -> Vec<u64> {
    let w = words(WINDOW, 5120);
    assert_eq!(w[639], 0x5050_4657, "the probe wrote every word");
    assert_eq!((w[631], w[632]), (REPS, ITERATIONS));
    for s in 0..2 {
        let b = 556 + 10 * s;
        assert_eq!(
            (w[b + 2], w[b + 3], w[b + 4], w[b + 5], w[b + 6]),
            (0, 0, SIBLING_EXIT, 1, 0),
            "phase {}: the sibling's create rc, join rc, exit value, start and sleep rc",
            s + 1,
        );
        for l in 0..K.len() {
            assert_eq!(w[576 + 9 * s + l], REPS, "phase {} loop {l}", s + 1);
        }
    }
    w
}

/// The `ppu_issue_fdiv_chain` payload, checked whole.
fn chain_words() -> Vec<u64> {
    let w = words(CHAIN, 2560);
    assert_eq!(w[319], 0x5050_4643, "the probe wrote every word");
    assert_eq!((w[311], w[312]), (REPS, ITERATIONS));
    for s in 0..2 {
        let b = 260 + 10 * s;
        assert_eq!(
            (w[b + 2], w[b + 3], w[b + 4], w[b + 5], w[b + 6]),
            (0, 0, SIBLING_EXIT, 1, 0),
            "phase {}: the sibling's create rc, join rc, exit value, start and sleep rc",
            s + 1,
        );
        for l in 0..4 {
            assert_eq!(w[290 + 4 * s + l], REPS, "phase {} loop {l}", s + 1);
        }
    }
    w
}

/// The median of the counted runs whose ticks sit at `first`, every
/// second word, in cycles an iteration.
pub(crate) fn median(w: &[u64], first: usize) -> f64 {
    let mut runs: Vec<f64> = (0..REPS as usize)
        .map(|r| (w[first + 2 * r] * PPU_CYCLES_PER_TB_TICK) as f64 / ITERATIONS as f64)
        .collect();
    runs.sort_by(f64::total_cmp);
    runs[runs.len() / 2]
}

/// `body` closed by its `bdnz`.
fn looped(body: &[u32]) -> &'static [u32] {
    let mut words = body.to_vec();
    let back = (4 * words.len() as u32).wrapping_neg() & 0xfffc;
    words.push(0x4200_0000 | back);
    Box::leak(words.into_boxed_slice())
}

/// The assembly sibling both probes share: four passes of 8 dependent `fdiv`,
/// then the stop flag's load and the round count's increment.
fn asm_sibling() -> &'static Sibling {
    &SIBLINGS
        .iter()
        .find(|(n, _)| *n == "fdiv")
        .expect("a ppu_issue_sibling sibling")
        .1
}

const C_ROUND: [u32; 34] = c_round(eight(0xfc00_6024));
const C_SEEDS: [Seed; 8] = round_seeds(0x3ff8_0000_0000_0000);

/// `ppu_issue_fdiv_chain`'s phase 2 sibling: the same divides in a C loop.
const C_SIBLING: Sibling = Sibling {
    words: &C_ROUND,
    seeds: &C_SEEDS,
};

/// CellGov's cycles an iteration of `body` beside `sibling`.
pub(crate) fn cellgov(body: &[u32], seeds: &'static [Seed], sibling: &Sibling) -> f64 {
    let r = Replay {
        loop_index: 0,
        words: looped(body),
        seeds,
    };
    replay_beside_for(&r, 1, sibling, REPLAY_ITERATIONS).expect("the loop replays")
}

/// `ppu_issue_fdiv_chain`'s loops, in its order.
fn chain_bodies() -> [(&'static str, Vec<u32>); 4] {
    [
        ("8 dependent add", vec![ADD; 8]),
        ("4 dependent add", vec![ADD; 4]),
        ("8 independent addi", ADDI8.to_vec()),
        ("bdnz", Vec::new()),
    ]
}

const CHAIN_SEEDS: &[Seed] = &[Seed::Gpr(15, 3)];

/// The largest share of a loop's median console run the model may sit
/// from it. At [`PPU_SMT_DIVIDE_WINDOW_SHARE`] the model times each loop
/// within 11%.
///
/// [`PPU_SMT_DIVIDE_WINDOW_SHARE`]: cellgov_ps3_abi::hw::ppu_issue::PPU_SMT_DIVIDE_WINDOW_SHARE
pub(crate) const TOLERANCE: f64 = 0.11;

#[test]
fn beside_back_to_back_divides_each_addi_loop_runs_near_the_console_s_median() {
    // On the console each pair of `addi` costs about 1.34 divide periods.
    // A loop of one `bdnz`, or of one `addi` and a `bdnz`, costs 1.47.
    let w = window_words();
    for (l, &k) in K.iter().enumerate() {
        let body: Vec<u32> = (0..k).map(|i| ADDI8[i % 8]).collect();
        let console = median(&w, 16 + 30 * l);
        let cellgov = cellgov(&body, &[], asm_sibling());
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "{k} addi: CellGov {cellgov:.1} cycles an iteration, the console's median {console:.1}",
        );
    }
}

#[test]
fn beside_back_to_back_divides_a_dependent_chain_costs_what_independent_pairs_cost() {
    // On the console 8 dependent `add` take 500 cycles an iteration, and 8
    // independent `addi` take 477. Each dispatch group waits once.
    let w = chain_words();
    for (l, (name, body)) in chain_bodies().iter().enumerate() {
        let console = median(&w, 8 + 30 * l);
        let cellgov = cellgov(body, CHAIN_SEEDS, asm_sibling());
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "{name}: CellGov {cellgov:.1} cycles an iteration, the console's median {console:.1}",
        );
    }
}

#[test]
fn a_sibling_that_runs_the_divides_in_a_c_loop_costs_each_loop_less() {
    // The C loop waits for its last divide each round, so its divides do
    // not stall the shared issue point back to back. On the console each
    // loop then takes from 0.16 to 0.34 of its time beside the assembly
    // loop. The model gives each loop 0.11 to 0.21.
    let w = chain_words();
    for (l, (name, body)) in chain_bodies().iter().enumerate() {
        let (asm, c) = (median(&w, 8 + 30 * l), median(&w, 128 + 30 * l));
        assert!(
            c < 0.4 * asm,
            "{name}: the console {c:.1} beside the C loop, {asm:.1} beside the assembly loop",
        );
        let asm = cellgov(body, CHAIN_SEEDS, asm_sibling());
        let c = cellgov(body, CHAIN_SEEDS, &C_SIBLING);
        assert!(
            c < 0.4 * asm,
            "{name}: CellGov {c:.1} beside the C loop, {asm:.1} beside the assembly loop",
        );
    }
}
