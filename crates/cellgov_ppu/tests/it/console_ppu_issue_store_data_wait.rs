//! CellGov's PPU timing of a store, a store string and a store multiple
//! after a load into the first register they store, against the
//! `ppu_issue_store_data_wait` console capture.
//!
//! The probe times eight loops of four pairs. Each pair is a `lwz`, then
//! one of these instructions that reads the loaded register:
//! - a `stw`;
//! - an `add`;
//! - a `stswi`;
//! - a `stmw`.
//!
//! Each case has a control whose `lwz` loads a register the second
//! instruction does not read. The replays run each loop through the
//! execution unit and its issue model.

use crate::console_ppu_issue_timing::{replay, Replay, Seed, BDNZ_8, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_data_wait/ps3/cech20-cex-493/observation.json";

/// The payload's word 19 when the probe wrote every word.
const DONE: u64 = 0x5050_5344;

/// The loops the probe times.
const LOOPS: usize = 8;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 160);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
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

/// `lwz r24,0(r7)`.
const LWZ_R24: u32 = 0x8307_0000;
/// `lwz r23,0(r7)`.
const LWZ_R23: u32 = 0x82e7_0000;
/// `stw r24,0(r10)`.
const STW_R24: u32 = 0x930a_0000;
/// `add r3,r24,r24`.
const ADD_R24: u32 = 0x7c78_c214;
/// `stswi r24,r10,32`.
const STSWI_R24: u32 = 0x7f0a_05aa;
/// `stmw r24,0(r10)`.
const STMW_R24: u32 = 0xbf0a_0000;

/// Four `a, b` pairs, then the loop's `bdnz`.
const fn pairs(a: u32, b: u32) -> [u32; 9] {
    [a, b, a, b, a, b, a, b, BDNZ_8]
}

const STW_LOADED: [u32; 9] = pairs(LWZ_R24, STW_R24);
const STW_NONE: [u32; 9] = pairs(LWZ_R23, STW_R24);
const ADD_LOADED: [u32; 9] = pairs(LWZ_R24, ADD_R24);
const ADD_NONE: [u32; 9] = pairs(LWZ_R23, ADD_R24);
const STSWI_FIRST: [u32; 9] = pairs(LWZ_R24, STSWI_R24);
const STSWI_NONE: [u32; 9] = pairs(LWZ_R23, STSWI_R24);
const STMW_FIRST: [u32; 9] = pairs(LWZ_R24, STMW_R24);
const STMW_NONE: [u32; 9] = pairs(LWZ_R23, STMW_R24);

/// r10 the probe's first line and r7 its second.
const SEEDS: &[Seed] = &[Seed::Gpr(10, ONE_LINE), Seed::Gpr(7, ONE_LINE + 128)];

const fn chain(loop_index: usize, words: &'static [u32]) -> Replay {
    Replay {
        loop_index,
        words,
        seeds: SEEDS,
    }
}

/// The probe's loops, in its order, and their names.
const PROBE_LOOPS: [(&str, Replay); LOOPS] = [
    ("lwz, stw of the loaded register", chain(0, &STW_LOADED)),
    ("lwz, stw of another register", chain(1, &STW_NONE)),
    ("lwz, add of the loaded register", chain(2, &ADD_LOADED)),
    ("lwz, add of another register", chain(3, &ADD_NONE)),
    ("lwz of the first source, stswi", chain(4, &STSWI_FIRST)),
    ("lwz of no source, stswi", chain(5, &STSWI_NONE)),
    ("lwz of the first source, stmw", chain(6, &STMW_FIRST)),
    ("lwz of no source, stmw", chain(7, &STMW_NONE)),
];

/// The largest share of a loop's fastest console run the model may differ
/// by, when the model falls outside the console's spread.
const TOLERANCE: f64 = 0.20;

/// The loops the model times outside the tolerance, each with the cycles
/// an iteration it may stay apart by.
///
/// - Loop 0: on the console, a `stw` of the register a `lwz` just loaded
///   waits about 3 cycles a pair, and the model waits for the load's
///   two-cycle use latency only.
/// - Loops 4 and 5: a `stswi` of 32 bytes takes the console about 27
///   cycles a pair, and the model's handbook microword count gives 20.
const RESIDUALS: [(usize, f64); 3] = [(0, 14.0), (4, 30.0), (5, 30.0)];

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
fn a_store_string_and_a_store_multiple_wait_for_no_load_into_their_first_source() {
    let w = console_words();
    let console = |l: usize| console_cycles(&w, l).0;
    let cellgov = |l: usize| replay(&PROBE_LOOPS[l].1, 0).expect("the loop replays");
    for (first, none) in [(4, 5), (6, 7)] {
        let name = PROBE_LOOPS[first].0;
        assert!(
            console(first) <= console(none) + 1.0,
            "{name}: the console takes {:.2} cycles an iteration, {:.2} with no source loaded",
            console(first),
            console(none),
        );
        assert!(
            cellgov(first) <= cellgov(none),
            "{name}: CellGov takes {:.2} cycles an iteration, {:.2} with no source loaded",
            cellgov(first),
            cellgov(none),
        );
    }
}
