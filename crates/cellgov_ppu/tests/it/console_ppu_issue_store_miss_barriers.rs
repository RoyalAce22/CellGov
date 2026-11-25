//! CellGov's PPU timing against the `ppu_issue_store_miss_barriers`
//! console capture. Each loop flushes the store's line or the next line,
//! then runs a `stw`, one barrier and 256 dependent `addi`. The barrier
//! is an `lwsync`, an `isync` or a `sync`.
//!
//! The chain outlasts the line's trip from memory. A barrier that waits
//! for the line adds the trip to the loop; a barrier that does not wait
//! hides the trip under the chain.

use crate::console_ppu_issue_timing::{replay_beside_for, Replay, Seed, Sibling};
use cellgov_ps3_abi::hw::ppu_issue::{PPU_CYCLES_PER_TB_TICK, PPU_STORE_MISS_FETCH_CYCLES};

const CONSOLE: &str =
    "../../tests/micro/ppu_issue_store_miss_barriers/ps3/cech20-cex-493/observation.json";

/// The payload's words.
const WORDS: usize = 18;

/// The payload's last word when the probe wrote every word.
const DONE: u64 = 0x5053_4D42;

/// The line every store writes, in the replay's memory.
const X: u64 = 0x8000;

/// The value the store writes.
const V: u64 = 0x5354_4F52;

/// The barrier after the store, in case order.
const LWSYNC: u32 = 0x7C20_04AC;
const ISYNC: u32 = 0x4C00_012C;
const SYNC: u32 = 0x7C00_04AC;

/// The dependent `addi` after the barrier.
const CHAIN: usize = 256;

/// The cases in the probe's order: the barrier, and the line the loop
/// flushes.
const CASES: [(u32, u64); 6] = [
    (LWSYNC, X),
    (LWSYNC, X + 0x80),
    (ISYNC, X),
    (ISYNC, X + 0x80),
    (SYNC, X),
    (SYNC, X + 0x80),
];

/// `dcbf 0,r3; sync; stw r8,0(r4); barrier`, [`CHAIN`] times
/// `addi r9,r9,1`, then `bdnz`, as the built image holds it.
fn case(c: usize) -> Replay {
    let (barrier, flushed) = CASES[c];
    let mut words = vec![0x7C00_18AC, SYNC, 0x9104_0000, barrier];
    words.extend(std::iter::repeat_n(0x3929_0001u32, CHAIN));
    words.push(0x4200_FBF0);
    Replay {
        loop_index: c,
        words: Box::leak(words.into_boxed_slice()),
        seeds: Box::leak(
            vec![Seed::Gpr(3, flushed), Seed::Gpr(4, X), Seed::Gpr(8, V)].into_boxed_slice(),
        ),
    }
}

/// Each case's smallest cycles an iteration on the console.
fn console() -> Vec<f64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(CONSOLE);
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4 * WORDS);
    let word = |i: usize| {
        u64::from(u32::from_be_bytes(
            p[4 * i..4 * i + 4].try_into().expect("4"),
        ))
    };
    assert_eq!(word(2), CHAIN as u64, "the probe's chain");
    assert_eq!(word(16), 1, "the stored word held");
    assert_eq!(word(WORDS - 1), DONE, "the probe wrote every word");
    let iterations = word(1);
    (0..CASES.len())
        .map(|c| (word(4 + 2 * c) * PPU_CYCLES_PER_TB_TICK) as f64 / iterations as f64)
        .collect()
}

/// No sibling: each loop replays alone.
const ALONE: Sibling = Sibling {
    words: &[],
    seeds: &[],
};

/// Iterations each replay times, after as many that warm it. The
/// `ppu_issue_store_miss_eieio` loops of the same shape give the same
/// cycles at 8 as at 512, and 512 takes the test past its time budget.
const LOOPS: u64 = 8;

fn cellgov() -> Vec<f64> {
    (0..CASES.len())
        .map(|c| {
            replay_beside_for(&case(c), 0, &ALONE, LOOPS)
                .unwrap_or_else(|| panic!("case {c} faults"))
        })
        .collect()
}

/// What the flush of the stored line adds to case `c`'s barrier over
/// the flush of the other line.
fn wait(cycles: &[f64], c: usize) -> f64 {
    cycles[c] - cycles[c + 1]
}

#[test]
fn only_a_sync_after_a_store_miss_waits_for_the_line() {
    let fetch = PPU_STORE_MISS_FETCH_CYCLES as f64;
    for (who, cycles) in [("console", console()), ("CellGov", cellgov())] {
        let sync = wait(&cycles, 4);
        for (name, c) in [("lwsync", 0), ("isync", 2)] {
            let other = wait(&cycles, c);
            assert!(
                sync - other > fetch / 2.0 && other < fetch / 2.0,
                "{who}: a store miss adds {sync:.0} cycles to a sync and {other:.0} to an {name}",
            );
        }
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
