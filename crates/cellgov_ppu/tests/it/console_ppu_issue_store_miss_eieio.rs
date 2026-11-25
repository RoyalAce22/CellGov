//! CellGov's PPU timing against the `ppu_issue_store_miss_eieio` console
//! capture: a `stw` to a flushed line, then an `eieio`, a `sync`, a load
//! of a held line or nothing, then 256 dependent `addi`.
//!
//! The chain outlasts the line's trip from memory. An instruction that
//! waits for the line adds the trip to the loop; one that does not wait
//! hides the trip under the chain.

use crate::console_ppu_issue_timing::{replay_beside_for, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_STORE_MISS_FETCH_CYCLES};

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_miss_eieio/ps3/cech20-cex-493/observation.json";

/// The payload's words.
const WORDS: usize = 28;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5053_4D45;

/// The line every store writes, in the replay's memory.
pub(crate) const X: u64 = 0x8000;

/// The line every load reads.
const Y: u64 = X + 0x100;

/// The value the store writes.
const V: u64 = 0x5354_4F52;

const DCBF: u32 = 0x7C00_18AC;
pub(crate) const SYNC: u32 = 0x7C00_04AC;
pub(crate) const LWSYNC: u32 = 0x7C20_04AC;
pub(crate) const STW: u32 = 0x9104_0000;
pub(crate) const EIEIO: u32 = 0x7C00_06AC;
pub(crate) const LWZ: u32 = 0x8125_0000;
const ADDI: u32 = 0x3929_0001;

/// The dependent `addi` after the body.
pub(crate) const CHAIN: usize = 256;

/// The cases in the probe's order: the body after `dcbf 0,r3; sync`, and
/// the line the loop flushes.
const CASES: [(&[u32], u64); 11] = [
    (&[STW, EIEIO], X),
    (&[STW, EIEIO], X + 0x80),
    (&[STW, EIEIO, LWZ], X),
    (&[STW, EIEIO, LWZ], X + 0x80),
    (&[STW, LWZ], X),
    (&[STW, LWZ], X + 0x80),
    (&[STW, SYNC], X),
    (&[STW, SYNC], X + 0x80),
    (&[STW], X + 0x80),
    (&[EIEIO], X + 0x80),
    (&[], X + 0x80),
];

/// Loop `index`: `dcbf 0,r3; sync`, `body`, [`CHAIN`] times
/// `addi r9,r9,1`, then `bdnz` to the `dcbf`, as the built images hold
/// it. r3 holds `flushed`, r4 [`X`], r5 the loaded line and r8 the
/// stored value.
pub(crate) fn flush_store_loop(index: usize, body: &[u32], flushed: u64) -> Replay {
    let mut words = vec![DCBF, SYNC];
    words.extend_from_slice(body);
    words.extend(std::iter::repeat_n(ADDI, CHAIN));
    let back = (words.len() * 4) as u32;
    words.push(0x4200_0000 | (back.wrapping_neg() & 0xFFFC));
    Replay {
        loop_index: index,
        words: Box::leak(words.into_boxed_slice()),
        seeds: Box::leak(
            vec![
                Seed::Gpr(3, flushed),
                Seed::Gpr(4, X),
                Seed::Gpr(5, Y),
                Seed::Gpr(8, V),
            ]
            .into_boxed_slice(),
        ),
    }
}

/// Each of `cases` loops' smallest cycles an iteration in the capture at
/// `path`, a payload of `words` big-endian u32 words that ends with the
/// held-word flag and `done`.
pub(crate) fn console_cycles(path: &str, words: usize, done: u64, cases: usize) -> Vec<f64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4 * words);
    let word = |i: usize| {
        u64::from(u32::from_be_bytes(
            p[4 * i..4 * i + 4].try_into().expect("4"),
        ))
    };
    assert_eq!(4 + 2 * cases + 2, words, "the payload holds every case");
    assert_eq!(word(2), CHAIN as u64, "the probe's chain");
    assert_eq!(word(words - 2), 1, "the stored word held");
    assert_eq!(word(words - 1), done, "the probe wrote every word");
    let iterations = word(1);
    (0..cases)
        .map(|c| (word(4 + 2 * c) * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64)
        .collect()
}

fn console() -> Vec<f64> {
    console_cycles(CONSOLE, WORDS, DONE, CASES.len())
}

/// No sibling: each loop replays alone.
const ALONE: Sibling = Sibling {
    words: &[],
    seeds: &[],
};

/// Iterations each replay times, after as many that warm it. For every
/// loop of both captures, 8 give the same cycles as 512.
const LOOPS: u64 = 8;

/// CellGov's cycles an iteration of each of `cases`, the body after
/// `dcbf 0,r3; sync` and the line the loop flushes.
pub(crate) fn cellgov_cycles(cases: &[(&[u32], u64)]) -> Vec<f64> {
    cases
        .iter()
        .enumerate()
        .map(|(c, &(body, flushed))| {
            replay_beside_for(&flush_store_loop(c, body, flushed), 0, &ALONE, LOOPS)
                .unwrap_or_else(|| panic!("case {c} faults"))
        })
        .collect()
}

fn cellgov() -> Vec<f64> {
    cellgov_cycles(&CASES)
}
/// What the flush of the stored line adds to case `c` over the flush of
/// the other line in case `c + 1`.
fn wait(cycles: &[f64], c: usize) -> f64 {
    cycles[c] - cycles[c + 1]
}

#[test]
fn a_sync_after_a_store_miss_waits_for_the_line_and_an_eieio_does_not() {
    let fetch = PPU_STORE_MISS_FETCH_CYCLES as f64;
    for (who, cycles) in [("console", console()), ("CellGov", cellgov())] {
        let (eieio, sync) = (wait(&cycles, 0), wait(&cycles, 6));
        assert!(
            sync > fetch / 2.0 && eieio < fetch / 2.0,
            "{who}: a store miss adds {sync:.0} cycles to a sync and {eieio:.0} to an eieio",
        );
    }
}

#[test]
fn an_eieio_does_not_hold_a_younger_load_for_a_store_missed_line() {
    let fetch = PPU_STORE_MISS_FETCH_CYCLES as f64;
    for (who, cycles) in [("console", console()), ("CellGov", cellgov())] {
        let load = wait(&cycles, 2);
        assert!(
            load < fetch / 2.0,
            "{who}: a store miss adds {load:.0} cycles to a load after an eieio",
        );
    }
}

/// The most cycles an iteration an `eieio` may add to a loop with no
/// store miss. In the capture it adds 2 after a store and none alone.
const EIEIO_ADDS: f64 = 8.0;

#[test]
fn an_eieio_after_a_store_to_a_held_line_or_alone_adds_under_eight_cycles() {
    for (who, cycles) in [("console", console()), ("CellGov", cellgov())] {
        let (after_store, alone) = (cycles[1] - cycles[8], cycles[9] - cycles[10]);
        assert!(
            after_store.abs() < EIEIO_ADDS && alone.abs() < EIEIO_ADDS,
            "{who}: an eieio adds {after_store:.1} cycles after a store and {alone:.1} alone",
        );
    }
}

/// The largest share of the console's cycles that the model may differ by.
const BAND: f64 = 0.20;

#[test]
fn each_case_comes_within_the_band_of_the_console() {
    let (console, cellgov) = (console(), cellgov());
    for c in 0..CASES.len() {
        assert!(
            (cellgov[c] - console[c]).abs() <= BAND * console[c],
            "case {c}: CellGov {:.2} cycles an iteration, the console {:.2}",
            cellgov[c],
            console[c],
        );
    }
}
