//! CellGov's PPU timing of `mftb` against the `ppu_issue_mftb` console
//! capture.
//!
//! The probe times seven loops on the main thread:
//!
//! - eight `mftb`;
//! - one `mftb`, then seven independent `addi`, seven dependent `add` or
//!   seven independent `fadd`;
//! - the `addi`, `add` and `fadd` chains with no `mftb`.
//!
//! It times them alone and beside a sibling thread that runs `mftb` in a
//! loop. The replays run each loop through the execution unit and its
//! issue model, alone and beside one PPU unit that runs the sibling's
//! loop.

use crate::console_ppu_issue_shared::eight;
use crate::console_ppu_issue_timing::{replay, replay_beside, Replay, Seed, Sibling, BDNZ_8};
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_SMT_SHARED_STALL_CYCLES};

const CONSOLE: &str = "../../tests/micro/ppu_issue_mftb/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 79 when the probe wrote every word.
const DONE: u64 = 0x5050_4d42;

/// The thread exit value the sibling passes.
const SIBLING_EXIT: u64 = 0x4d42;

/// The loops a phase times.
const CHAINS: usize = 7;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 80);
    assert_eq!(words[79], DONE, "the probe wrote every word");
    assert_eq!(words[72], 0, "the sibling was created");
    assert_eq!(words[73], 0, "the sibling joined");
    assert_eq!(words[74], SIBLING_EXIT, "the sibling exited");
    assert_eq!(words[75], 1, "the sibling ran");
    words
}

/// Loop `CHAINS * phase + chain`'s cycles an iteration on the console:
/// the fastest run, the slowest, and the mean over its runs.
fn console_cycles(words: &[u64], phase: usize, chain: usize) -> (f64, f64, f64) {
    let (reps, iterations) = (words[77], words[78]);
    let at = CHAINS * phase + chain;
    let per = |tb: u64, n: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / n as f64;
    (
        per(words[2 * at], iterations),
        per(words[2 * at + 1], iterations),
        per(words[50 + at], reps * iterations),
    )
}

/// `mftb rt`.
const fn mftb(rt: u32) -> u32 {
    0x7c0c_42e6 | rt << 21
}

/// `bdnz` back over sixteen instructions.
const BDNZ_16: u32 = 0x4200_ffc0;

/// `addi r17..r20,1` in turn.
const ADDI: [u32; 4] = [0x3a31_0001, 0x3a52_0001, 0x3a73_0001, 0x3a94_0001];
/// `addi r18..r21,1` in turn.
const ADDI_AFTER: [u32; 4] = [0x3a52_0001, 0x3a73_0001, 0x3a94_0001, 0x3ab5_0001];
/// `add r9,r9,r15`.
const ADD: u32 = 0x7d29_7a14;
/// `fadd f0,f9,f10,f11 += f12` in turn.
const FADD: [u32; 4] = [0xfc00_602a, 0xfd29_602a, 0xfd4a_602a, 0xfd6b_602a];

const CHAIN_MFTB: [u32; 9] = [
    mftb(17),
    mftb(18),
    mftb(19),
    mftb(20),
    mftb(17),
    mftb(18),
    mftb(19),
    mftb(20),
    BDNZ_8,
];
const CHAIN_MFTB_ADDI: [u32; 9] = [
    mftb(17),
    ADDI_AFTER[0],
    ADDI_AFTER[1],
    ADDI_AFTER[2],
    ADDI_AFTER[3],
    ADDI_AFTER[0],
    ADDI_AFTER[1],
    ADDI_AFTER[2],
    BDNZ_8,
];
const CHAIN_MFTB_ADD: [u32; 9] = [mftb(17), ADD, ADD, ADD, ADD, ADD, ADD, ADD, BDNZ_8];
const CHAIN_MFTB_FADD: [u32; 9] = [
    mftb(17),
    FADD[0],
    FADD[1],
    FADD[2],
    FADD[3],
    FADD[0],
    FADD[1],
    FADD[2],
    BDNZ_8,
];
const CHAIN_ADDI: [u32; 17] = [
    ADDI[0], ADDI[1], ADDI[2], ADDI[3], ADDI[0], ADDI[1], ADDI[2], ADDI[3], ADDI[0], ADDI[1],
    ADDI[2], ADDI[3], ADDI[0], ADDI[1], ADDI[2], ADDI[3], BDNZ_16,
];
const CHAIN_ADD: [u32; 9] = eight(ADD);
const CHAIN_FADD: [u32; 9] = eight(FADD[0]);

const ZERO: u64 = 0;
/// 1.0 as a double.
const ONE: u64 = 0x3ff0_0000_0000_0000;

const FP_SEEDS: &[Seed] = &[
    Seed::Fpr(0, ONE),
    Seed::Fpr(9, ONE),
    Seed::Fpr(10, ONE),
    Seed::Fpr(11, ONE),
    Seed::Fpr(12, ZERO),
];
const ADD_SEEDS: &[Seed] = &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)];

const fn chain(loop_index: usize, words: &'static [u32], seeds: &'static [Seed]) -> Replay {
    Replay {
        loop_index,
        words,
        seeds,
    }
}

/// The probe's loops, in its order, and their names.
const LOOPS: [(&str, Replay); CHAINS] = [
    ("mftb", chain(0, &CHAIN_MFTB, &[])),
    ("mftb then addi", chain(1, &CHAIN_MFTB_ADDI, &[])),
    ("mftb then add", chain(2, &CHAIN_MFTB_ADD, ADD_SEEDS)),
    ("mftb then fadd", chain(3, &CHAIN_MFTB_FADD, FP_SEEDS)),
    ("addi", chain(4, &CHAIN_ADDI, &[])),
    ("add", chain(5, &CHAIN_ADD, ADD_SEEDS)),
    ("fadd", chain(6, &CHAIN_FADD, FP_SEEDS)),
];

/// The probe's sibling: eight `mftb` to r15..r18, then its `bdnz`.
const SIBLING_MFTB: Sibling = Sibling {
    words: &[
        mftb(15),
        mftb(16),
        mftb(17),
        mftb(18),
        mftb(15),
        mftb(16),
        mftb(17),
        mftb(18),
        BDNZ_8,
    ],
    seeds: &[],
};

/// The largest share of a loop's console mean the model may differ by,
/// when the model falls outside the console's spread.
const TOLERANCE: f64 = 0.20;

#[test]
fn alone_the_instructions_after_an_mftb_wait_for_it_as_on_the_console() {
    // On the console, one mftb and seven addi, add or fadd take about
    // 50 to 58 cycles. The seven alone take about 8 to 14. What follows
    // an mftb waits for it, whichever unit it issues to.
    let w = console_words();
    for (c, (name, r)) in LOOPS.iter().enumerate() {
        let cellgov = replay(r, 0).expect("the loop replays");
        let (fastest, slowest, mean) = console_cycles(&w, 0, c);
        assert!(
            (fastest..=slowest).contains(&cellgov) || (cellgov - mean).abs() / mean <= TOLERANCE,
            "{name}: CellGov {cellgov:.2} cycles an iteration, the console {fastest:.2} to \
             {slowest:.2}, mean {mean:.2}",
        );
    }
}

#[test]
fn beside_an_mftb_sibling_every_loop_waits_short_of_the_console_as_beside_a_held_divide() {
    // Beside the mftb sibling, the console's loops wait 20 to 30 cycles
    // an instruction past their alone time. A held divide sibling gives
    // the same level in `ppu_issue_loop_shape`. The model gives the mftb
    // the divide's stall, `PPU_SMT_SHARED_STALL_CYCLES`, and lands short
    // of the console by the same accepted gap.
    let w = console_words();
    for (c, (name, r)) in LOOPS.iter().enumerate() {
        let alone = replay(r, 0).expect("the loop replays");
        let cellgov = replay_beside(r, 1, &SIBLING_MFTB).expect("the loop replays");
        let console = console_cycles(&w, 1, c).2;
        let instructions = (r.words.len() - 1) as f64;
        let wait = (console - console_cycles(&w, 0, c).2) / instructions;
        assert!(
            wait > PPU_SMT_SHARED_STALL_CYCLES as f64,
            "{name}: the console waits {wait:.1} cycles an instruction beside mftb"
        );
        assert!(
            cellgov < console && cellgov > 0.45 * console,
            "{name} beside mftb: CellGov {cellgov:.1} cycles an iteration (alone {alone:.1}), \
             the console's mean {console:.1}",
        );
    }
}
