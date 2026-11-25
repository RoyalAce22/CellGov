//! CellGov's PPU timing against the `ppu_issue_vscr_miss` console
//! capture: whether an `mtvscr` that holds an FP or VMX load in the VSU
//! issue queue also delays the load's cache miss.
//!
//! Each loop is a pointer chase through FP or vector registers: the load
//! reads the node, a store puts the node's first doubleword in a scratch
//! doubleword at r8, and an `ld` takes it back into r9. The nodes are a
//! shuffled 8 MB ring, one L2 miss a load, or one line that points to
//! itself, one L1 hit a load.

use crate::console_ppu_issue_timing::{replay_beside_for, Replay, Seed, Sibling, ONE_LINE};
use cellgov_ps3_abi::hw::ppu_issue::PPU_CYCLES_PER_TB_TICK;

const CONSOLE: &str = "../../tests/micro/ppu_issue_vscr_miss/ps3/cech20-cex-493/observation.json";

/// The payload's word 23 when the probe wrote every word.
const DONE: u64 = 0x5050_5658;

/// The probe's runs of each loop.
const REPS: u64 = 5;

/// log2 of the iterations of each loop over the ring, and over the node.
const RING_LOG2: u32 = 12;
const NODE_LOG2: u32 = 14;

/// Where the replay builds its ring, and the ring's size, as the probe's.
const RING: u64 = 0x10_0000;
const RING_BYTES: u64 = 8 * 1024 * 1024;

/// Iterations each replay times, after as many that warm it. Every
/// iteration over the ring misses the L2, so a short replay keeps each
/// test inside its time budget.
const ITERATIONS: u64 = 128;

/// No sibling: each loop replays alone.
const ALONE: Sibling = Sibling {
    words: &[],
    seeds: &[],
};

/// `mtvscr v0`.
const MTVSCR: u32 = 0x1000_0644;
/// `lfd f1,0(r9)`, `stfd f1,0(r8)`, `ld r9,0(r8)`.
const FP_CHASE: [u32; 3] = [0xc829_0000, 0xd828_0000, 0xe928_0000];
/// `lvx v1,0,r9`, `stvx v1,0,r8`, `ld r9,0(r8)`.
const VMX_CHASE: [u32; 3] = [0x7c20_48ce, 0x7c20_41ce, 0xe928_0000];

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 192);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[23], DONE, "the probe wrote every word");
    assert_eq!(
        (words[20], words[21], words[22]),
        (REPS, u64::from(RING_LOG2), u64::from(NODE_LOG2))
    );
    assert_eq!(words[18], 1, "every ring run ended on a ring line");
    assert_eq!(words[19], 1, "every node run ended on the node");
    words
}

/// Loop `k`'s fastest run on the console, in core cycles an iteration.
fn console_cycles(w: &[u64], k: usize) -> f64 {
    let log2 = if k < 5 { RING_LOG2 } else { NODE_LOG2 };
    (w[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << log2) as f64
}

/// CellGov's cycles an iteration for ring loop `k`: `chase`, after an
/// `mtvscr` when `fenced` is true.
fn cellgov_cycles(k: usize, chase: &[u32], fenced: bool) -> f64 {
    let nodes = Seed::Ring(9, RING, RING_BYTES, true);
    let mut words = if fenced { vec![MTVSCR] } else { Vec::new() };
    words.extend_from_slice(chase);
    words.push(0x4200_0000 | ((4 * words.len() as u32).wrapping_neg() & 0xfffc));
    let r = Replay {
        loop_index: k,
        words: Box::leak(words.into_boxed_slice()),
        seeds: Box::leak(vec![nodes, Seed::Gpr(8, ONE_LINE)].into_boxed_slice()),
    };
    replay_beside_for(&r, 0, &ALONE, ITERATIONS).expect("the chase replays")
}

/// The most cycles an iteration CellGov's `mtvscr` hold may sit from the
/// console's.
const HOLD_GAP: f64 = 8.0;

/// The largest share of the console's fastest run a ring loop may sit
/// from it.
///
/// The node loops are not held one by one: CellGov times the FP chase
/// over the node about 18 cycles an iteration short of the console.
const TOLERANCE: f64 = 0.05;

/// Checks the hold an `mtvscr` adds to `chase` when each load misses the
/// L2. `loop0` is the probe's ring loop of `chase` alone, the loop after
/// it adds the `mtvscr`, and `loop0 + 4` and `loop0 + 5` are the same
/// pair over the node.
fn check_miss_hold(chase: &[u32], loop0: usize) {
    let w = console_words();
    let hit = console_cycles(&w, loop0 + 5) - console_cycles(&w, loop0 + 4);
    let miss = console_cycles(&w, loop0 + 1) - console_cycles(&w, loop0);
    assert!(
        miss > 0.75 * hit,
        "on the console the hold on a miss is {miss:.1} cycles and on a hit {hit:.1}"
    );
    let alone = cellgov_cycles(loop0, chase, false);
    let fenced = cellgov_cycles(loop0 + 1, chase, true);
    assert!(
        (fenced - alone - miss).abs() <= HOLD_GAP,
        "loops {loop0} and {}: CellGov's hold {:.1} cycles, the console's {miss:.1}",
        loop0 + 1,
        fenced - alone,
    );
    for (k, cellgov) in [(loop0, alone), (loop0 + 1, fenced)] {
        let console = console_cycles(&w, k);
        assert!(
            (cellgov - console).abs() / console <= TOLERANCE,
            "loop {k}: CellGov {cellgov:.1} cycles an iteration, the console {console:.1}",
        );
    }
}

#[test]
fn an_mtvscr_adds_its_hold_to_a_fenced_lfd_that_misses_the_l2() {
    check_miss_hold(&FP_CHASE, 1);
}

#[test]
fn an_mtvscr_adds_its_hold_to_a_fenced_lvx_that_misses_the_l2() {
    check_miss_hold(&VMX_CHASE, 3);
}
