//! CellGov's PPU timing of the registers a load string, a store multiple
//! and an overflow-enabled multiply touch past their first, against the
//! `ppu_issue_range_registers` console capture.
//!
//! The probe times eight loops of four pairs. In each pair the second
//! instruction reads a register the first touches:
//! - r0, the target an `lswi r31` wraps into;
//! - r31, the first target of that `lswi`;
//! - the XER that `mullwo` writes;
//! - the eighth source of `stmw r24`.
//!
//! Each case has a control whose second instruction reads a register the
//! first leaves alone. The replays run each loop through the execution
//! unit and its issue model.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, BDNZ_8, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_range_registers/ps3/cech20-cex-493/cgov_frame.bin";

/// The payload's word 19 when the probe wrote every word.
const DONE: u64 = 0x5050_5752;

/// The loops the probe times.
const LOOPS: usize = 8;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u64> = frame[8..]
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words.len(), 20);
    assert_eq!(words[19], DONE, "the probe wrote every word");
    words
}

/// Loop `l`'s cycles an iteration on the console: the fastest run and the
/// slowest.
fn console_cycles(words: &[u64], l: usize) -> (f64, f64) {
    let iterations = words[18];
    let per = |tb: u64| (tb * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64;
    (per(words[2 * l]), per(words[2 * l + 1]))
}

/// `lswi r31,r10,8`: r31, then r0.
const LSWI_8: u32 = 0x7fea_44aa;
/// `lswi r31,r10,4`: r31 alone.
const LSWI_4: u32 = 0x7fea_24aa;
/// `add r3,r0,r0`.
const ADD_R0: u32 = 0x7c60_0214;
/// `add r3,r31,r31`.
const ADD_R31: u32 = 0x7c7f_fa14;
/// `mullwo r3,r4,r5`.
const MULLWO: u32 = 0x7c64_2dd6;
/// `mullw r3,r4,r5`.
const MULLW: u32 = 0x7c64_29d6;
/// `mfxer r6`.
const MFXER: u32 = 0x7cc1_02a6;
/// `lwz r31,0(r7)`.
const LWZ_R31: u32 = 0x83e7_0000;
/// `lwz r23,0(r7)`.
const LWZ_R23: u32 = 0x82e7_0000;
/// `lwz r24,0(r7)`.
const LWZ_R24: u32 = 0x8307_0000;
/// `stmw r24,0(r10)`.
const STMW_R24: u32 = 0xbf0a_0000;

/// Four `a, b` pairs, then the loop's `bdnz`.
const fn pairs(a: u32, b: u32) -> [u32; 9] {
    [a, b, a, b, a, b, a, b, BDNZ_8]
}

const LSWI_WRAPPED: [u32; 9] = pairs(LSWI_8, ADD_R0);
const LSWI_FIRST: [u32; 9] = pairs(LSWI_8, ADD_R31);
const LSWI_NONE: [u32; 9] = pairs(LSWI_4, ADD_R0);
const MULLWO_XER: [u32; 9] = pairs(MULLWO, MFXER);
const MULLW_XER: [u32; 9] = pairs(MULLW, MFXER);
const STMW_LAST: [u32; 9] = pairs(LWZ_R31, STMW_R24);
const STMW_NONE: [u32; 9] = pairs(LWZ_R23, STMW_R24);
const STMW_FIRST: [u32; 9] = pairs(LWZ_R24, STMW_R24);

/// r10 the probe's first line, r7 its second, r4 = 3 and r5 = 5.
const SEEDS: &[Seed] = &[
    Seed::Gpr(10, ONE_LINE),
    Seed::Gpr(7, ONE_LINE + 128),
    Seed::Gpr(4, 3),
    Seed::Gpr(5, 5),
];

const fn chain(loop_index: usize, words: &'static [u32]) -> Replay {
    Replay {
        loop_index,
        words,
        seeds: SEEDS,
    }
}

/// The probe's loops, in its order, and their names.
const PROBE_LOOPS: [(&str, Replay); LOOPS] = [
    ("lswi, add of the wrapped target", chain(0, &LSWI_WRAPPED)),
    ("lswi, add of the first target", chain(1, &LSWI_FIRST)),
    ("lswi, add of no target", chain(2, &LSWI_NONE)),
    ("mullwo, mfxer", chain(3, &MULLWO_XER)),
    ("mullw, mfxer", chain(4, &MULLW_XER)),
    ("lwz of the last source, stmw", chain(5, &STMW_LAST)),
    ("lwz of no source, stmw", chain(6, &STMW_NONE)),
    ("lwz of the first source, stmw", chain(7, &STMW_FIRST)),
];

/// The largest share of a loop's fastest console run the model may differ
/// by, when the model falls outside the console's spread.
const TOLERANCE: f64 = 0.20;

/// The loops the model times outside the tolerance, each with the cycles
/// an iteration it may stay apart by.
///
/// - Loops 3 and 4: an `mfxer` costs the console about 37 cycles a pair,
///   and the model issues it as a one-cycle fixed-point move.
const RESIDUALS: [(usize, f64); 2] = [(3, 110.0), (4, 110.0)];

#[test]
fn each_loop_lands_within_the_console_s_tolerance_or_its_named_residual() {
    let w = console_words();
    for (l, (name, r)) in PROBE_LOOPS.iter().enumerate() {
        let cellgov = replay(r, 0).expect("the loop replays");
        let (fastest, slowest) = console_cycles(&w, l);
        let apart = (cellgov - fastest).abs();
        let within = (fastest..=slowest).contains(&cellgov) || apart / fastest <= TOLERANCE;
        match RESIDUALS.iter().find(|r| r.0 == l) {
            Some(&(_, bound)) => {
                assert!(
                    !within,
                    "{name} now lands within the console's tolerance; drop it from the \
                     residuals",
                );
                assert!(
                    apart < bound,
                    "{name}: CellGov {cellgov:.2} cycles an iteration, the console's fastest \
                     {fastest:.2}, more than {bound} apart",
                );
            }
            None => assert!(
                within,
                "{name}: CellGov {cellgov:.2} cycles an iteration, the console {fastest:.2} \
                 to {slowest:.2}",
            ),
        }
    }
}

#[test]
fn an_add_of_the_target_lswi_wraps_into_waits_as_for_its_first_target() {
    let w = console_words();
    let console = |l: usize| console_cycles(&w, l).0;
    let cellgov = |l: usize| replay(&PROBE_LOOPS[l].1, 0).expect("the loop replays");
    assert!(
        console(0) >= console(1),
        "the console waits for the wrapped target"
    );
    assert!(
        cellgov(0) >= cellgov(1),
        "CellGov {:.1} after an add of the wrapped target, {:.1} after an add of the first",
        cellgov(0),
        cellgov(1),
    );
}
