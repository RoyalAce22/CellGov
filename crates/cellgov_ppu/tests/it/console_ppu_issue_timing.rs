//! CellGov's PPU timing against the `ppu_issue_timing` and
//! `ppu_issue_table` console captures.
//!
//! The `ppu_issue_timing` probe times twenty-three loops with `mftb`, five
//! runs each. `the_recorded_constants_are_the_captures` derives again each
//! constant of `cellgov_ps3_abi::hw::ppu_issue` that is a fastest run of
//! this capture. `the_issue_model_times_each_loop_within_the_console_s_spread`
//! runs loops 0 to 19 through the execution unit and its issue model. It
//! holds each loop's cycles an iteration no farther from the fastest run
//! than the slowest run is. Loops 20 to 22 run beside a busy second
//! hardware thread. `the_issue_model_times_each_loop_beside_a_busy_sibling_within_the_console_s_spread`
//! replays them beside a second PPU unit that runs the probe's sibling
//! loop.
//!
//! The `ppu_issue_table` probe times twenty-one more loops the same way,
//! one per latency the model takes from the handbook's instruction table.
//! `the_issue_model_times_each_table_loop_within_the_console_s_spread`
//! replays them.

use cellgov_core::Runtime;
use cellgov_exec::UnitStatus;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::ppu_issue::*;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/ppu_issue_timing/ps3/cech20-cex-493/observation.json";

/// log2 of each loop's iterations on the console, from the probe's
/// header.
const ITERATIONS_LOG2: [u32; 23] = [
    20, 20, 20, 18, 16, 18, 18, 18, 20, 20, 20, 17, 15, 12, 18, 20, 20, 20, 20, 12, 20, 20, 17,
];

/// The payload's word 55 when the probe wrote every word.
const DONE: u64 = 0x5050_4954;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 448);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[55], DONE, "the probe wrote every word");
    assert_eq!(words[54], 5, "five runs a loop");
    assert_eq!(words[47], 0, "the sibling thread was created");
    assert_ne!(words[48], u64::MAX, "the sibling thread ran");
    assert_eq!(words[50], 0, "the sibling thread joined");
    assert_eq!(
        words[51], 0x5157,
        "the sibling thread exited through its own path"
    );
    words
}

/// Loop `k`'s fastest run, in core cycles an iteration, unrounded.
fn console_cycles(words: &[u64], k: usize) -> f64 {
    (words[2 * k] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64
}

/// Loop `k`'s slowest run, in core cycles an iteration.
fn console_slowest(words: &[u64], k: usize) -> f64 {
    (words[2 * k + 1] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << ITERATIONS_LOG2[k]) as f64
}

/// The cycles the handbook's divide formula gives the probe's `divd` of
/// `0x1_2345_6789` by 1: 32 more leading zeros in the divisor, so 17
/// iterations, `(1 + 17 + 1) x 2`.
const PROBE_DIVD_FORMULA_CYCLES: u64 = 38;

fn round(v: f64) -> u64 {
    v.round() as u64
}

#[test]
fn the_recorded_constants_are_the_captures() {
    let w = console_words();
    assert_eq!(w[46], cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ);

    // At 40 cycles a tick the three steadiest loops read whole cycles to
    // half a percent; at 39 or 41 the empty loop does not.
    for k in [0, 1, 11] {
        let c = console_cycles(&w, k);
        assert!(
            (c - c.round()).abs() / c < 0.005,
            "loop {k} reads {c} cycles"
        );
    }
    for ratio in [39, 41] {
        let c = (w[0] * ratio) as f64 / (1u64 << ITERATIONS_LOG2[0]) as f64;
        assert!(
            (c - c.round()).abs() / c > 0.01,
            "at {ratio} the empty loop reads {c} cycles"
        );
    }

    let per = |k: usize, n: f64| round(console_cycles(&w, k) / n);
    assert_eq!(per(0, 1.0), PPU_TAKEN_BRANCH_FETCH_CYCLES);
    // Eight not-taken branches and the loop's taken one.
    assert_eq!(
        round((console_cycles(&w, 15) - PPU_TAKEN_BRANCH_FETCH_CYCLES as f64) / 8.0),
        PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES,
    );
    assert_eq!(per(2, 8.0), PPU_FX_LATENCY_CYCLES);
    assert_eq!(per(3, 8.0), PPU_MULLD_LATENCY_CYCLES);
    assert_eq!(
        per(4, 4.0) - PROBE_DIVD_FORMULA_CYCLES,
        PPU_DIVIDE_REFETCH_CYCLES
    );
    assert_eq!(per(5, 8.0), PPU_FP_LATENCY_CYCLES);
    assert_eq!(per(6, 8.0), PPU_FP_LATENCY_CYCLES);
    assert_eq!(per(7, 8.0), PPU_VMX_FLOAT_LATENCY_CYCLES);
    assert_eq!(per(11, 8.0), PPU_L1_LOAD_USE_CYCLES);
    assert_eq!(per(12, 8.0), PPU_L2_LOAD_USE_CYCLES);
    assert_eq!(per(13, 8.0), PPU_MEMORY_LOAD_USE_CYCLES);
    assert_eq!(per(14, 4.0), PPU_LOAD_HIT_STORE_CYCLES);
    assert_eq!((per(1, 1.0), per(20, 1.0)), PPU_SMT_INDEPENDENT_ADDI_CYCLES);
    assert_eq!((per(2, 1.0), per(21, 1.0)), PPU_SMT_DEPENDENT_ADD_CYCLES);
    assert_eq!((per(11, 1.0), per(22, 1.0)), PPU_SMT_L1_CHASE_CYCLES);
}

/// Where each replay places its loop, its one data line and its rings.
const CODE: u64 = 0x1000;
pub(crate) const ONE_LINE: u64 = 0x2000;
const L1_RING: u64 = 0x1_0000;
const L2_RING: u64 = 0x2_0000;
const MEM_RING: u64 = 0x10_0000;
const LINE: u64 = 128;

/// Iterations a replay times. It runs as many before them, which warm
/// the caches and the branch history the way the probe's earlier runs
/// warm the console's.
const REPLAY_ITERATIONS: u64 = 512;

/// A register a replay seeds before its loop.
#[derive(Clone, Copy)]
pub(crate) enum Seed {
    Gpr(usize, u64),
    Fpr(usize, u64),
    /// A GPR holding the head of a ring of `bytes` at `base`, shuffled or
    /// strided as the probe builds it.
    Ring(usize, u64, u64, bool),
    /// The GPR that counts the loop's iterations down, for a loop that
    /// does not count in CTR.
    Counter(usize),
}

const ONE: u64 = 0x3FF0_0000_0000_0000;

/// One loop of the probe: its body as `powerpc64-ps3-elf-objdump` of the
/// built image prints it, from the aligned label through the `bdnz`, and
/// the registers the body reads.
pub(crate) struct Replay {
    pub(crate) loop_index: usize,
    pub(crate) words: &'static [u32],
    pub(crate) seeds: &'static [Seed],
}

pub(crate) const BDNZ_8: u32 = 0x4200_ffe0;

const REPLAYS: [Replay; 20] = [
    Replay {
        loop_index: 0,
        words: &[0x4200_0000],
        seeds: &[],
    },
    Replay {
        loop_index: 1,
        words: &[
            0x39ce_0001,
            0x39ef_0001,
            0x3a10_0001,
            0x3a31_0001,
            0x3a52_0001,
            0x3a73_0001,
            0x3a94_0001,
            0x3ab5_0001,
            BDNZ_8,
        ],
        seeds: &[],
    },
    Replay {
        loop_index: 2,
        words: &eight_then_bdnz(0x7d29_7a14),
        seeds: &[Seed::Gpr(9, 1), Seed::Gpr(15, 3)],
    },
    Replay {
        loop_index: 3,
        words: &eight_then_bdnz(0x7d29_79d2),
        seeds: &[Seed::Gpr(9, 7), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 4,
        words: &[
            0x7d29_7bd2,
            0x7d29_7bd2,
            0x7d29_7bd2,
            0x7d29_7bd2,
            0x4200_fff0,
        ],
        seeds: &[Seed::Gpr(9, 0x1_2345_6789), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 5,
        words: &eight_then_bdnz(0xfc00_602a),
        seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
    },
    Replay {
        loop_index: 6,
        words: &eight_then_bdnz(0xfc00_603a),
        seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
    },
    Replay {
        loop_index: 7,
        words: &eight_then_bdnz(0x1042_180a),
        seeds: &[],
    },
    Replay {
        loop_index: 8,
        words: &[
            0x39ce_0001,
            0xea48_0000,
            0x39ef_0001,
            0xea68_0008,
            0x3a10_0001,
            0xea88_0010,
            0x3a31_0001,
            0xeaa8_0018,
            BDNZ_8,
        ],
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    },
    Replay {
        loop_index: 9,
        words: &[
            0x39ce_0001,
            0xfc21_282a,
            0x39ef_0001,
            0xfc42_282a,
            0x3a10_0001,
            0xfc63_282a,
            0x3a31_0001,
            0xfc84_282a,
            BDNZ_8,
        ],
        seeds: &[
            Seed::Fpr(1, ONE),
            Seed::Fpr(2, ONE),
            Seed::Fpr(3, ONE),
            Seed::Fpr(4, ONE),
            Seed::Fpr(5, 0),
        ],
    },
    Replay {
        loop_index: 10,
        words: &[
            0xfc21_482a,
            0xfc42_482a,
            0xfc63_482a,
            0xfc84_482a,
            0xfca5_482a,
            0xfcc6_482a,
            0xfce7_482a,
            0xfd08_482a,
            BDNZ_8,
        ],
        seeds: &[
            Seed::Fpr(1, ONE),
            Seed::Fpr(2, ONE),
            Seed::Fpr(3, ONE),
            Seed::Fpr(4, ONE),
            Seed::Fpr(5, ONE),
            Seed::Fpr(6, ONE),
            Seed::Fpr(7, ONE),
            Seed::Fpr(8, ONE),
            Seed::Fpr(9, 0),
        ],
    },
    Replay {
        loop_index: 11,
        words: &eight_then_bdnz(0xe884_0000),
        seeds: &[Seed::Ring(4, L1_RING, 16 * 1024, false)],
    },
    Replay {
        loop_index: 12,
        words: &eight_then_bdnz(0xe884_0000),
        seeds: &[Seed::Ring(4, L2_RING, 256 * 1024, true)],
    },
    Replay {
        loop_index: 13,
        words: &eight_then_bdnz(0xe884_0000),
        seeds: &[Seed::Ring(4, MEM_RING, 8 * 1024 * 1024, true)],
    },
    Replay {
        loop_index: 14,
        words: &[
            0xf927_0000,
            0xe927_0000,
            0xf927_0000,
            0xe927_0000,
            0xf927_0000,
            0xe927_0000,
            0xf927_0000,
            0xe927_0000,
            BDNZ_8,
        ],
        seeds: &[Seed::Gpr(9, 5), Seed::Gpr(7, ONE_LINE)],
    },
    Replay {
        loop_index: 15,
        words: &[
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x2c2e_0000,
            0x4082_0004,
            0x4200_ffc0,
        ],
        seeds: &[Seed::Gpr(14, 0)],
    },
    Replay {
        loop_index: 16,
        words: &eight_then_bdnz(0x4800_0004),
        seeds: &[],
    },
    Replay {
        loop_index: 17,
        words: &LCG_LOOP,
        seeds: &[
            Seed::Gpr(14, 1),
            Seed::Gpr(16, 0x5851_f42d),
            Seed::Gpr(9, 0),
            Seed::Gpr(3, 0),
        ],
    },
    Replay {
        loop_index: 18,
        words: &LCG_LOOP,
        seeds: &[
            Seed::Gpr(14, 1),
            Seed::Gpr(16, 0x5851_f42d),
            Seed::Gpr(9, 0),
            Seed::Gpr(3, 1),
        ],
    },
    // The probe's loop of calls: the eight argument registers, the
    // syscall number, `sc`, then the count in r31.
    Replay {
        loop_index: 19,
        words: &[
            0x3860_0000,
            0x3880_0000,
            0x38a0_0000,
            0x38c0_0000,
            0x38e0_0000,
            0x3900_0000,
            0x3920_0000,
            0x3940_0000,
            0x3960_0093,
            0x4400_0002,
            0x2c3f_0001,
            0x3bff_ffff,
            0x4082_ffd0,
        ],
        seeds: &[Seed::Counter(31)],
    },
];

const TABLE: &str = "../../tests/micro/ppu_issue_table/ps3/cech20-cex-493/observation.json";

/// log2 of each `ppu_issue_table` loop's iterations on the console, from
/// the probe's header.
const TABLE_ITERATIONS_LOG2: [u32; 21] = [
    20, 18, 18, 16, 16, 16, 16, 16, 16, 16, 16, 16, 18, 18, 18, 18, 18, 18, 18, 16, 18,
];

/// The `ppu_issue_table` payload's word 44 when the probe wrote every
/// word.
const TABLE_DONE: u64 = 0x5050_5442;

/// The `ppu_issue_table` capture's payload as big-endian words.
fn table_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(TABLE))
        .unwrap_or_else(|e| panic!("{TABLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 360);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[44], TABLE_DONE, "the probe wrote every word");
    assert_eq!(words[43], 5, "five runs a loop");
    words
}

const BDNZ_4: u32 = 0x4200_fff0;

/// The `ppu_issue_table` probe's loops, one per table latency.
const TABLE_REPLAYS: [Replay; 21] = [
    Replay {
        loop_index: 0,
        words: &[0x4200_0000],
        seeds: &[],
    },
    Replay {
        loop_index: 1,
        words: &eight_then_bdnz(0x1d29_0001),
        seeds: &[Seed::Gpr(9, 7)],
    },
    Replay {
        loop_index: 2,
        words: &eight_then_bdnz(0x7d29_79d6),
        seeds: &[Seed::Gpr(9, 7), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 3,
        words: &four_then_bdnz(0x7d29_7bd6),
        seeds: &[Seed::Gpr(9, 0x7fff_ffff), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 4,
        words: &four_then_bdnz(0x7d29_7bd2),
        seeds: &[Seed::Gpr(9, 0x7fff_ffff_ffff_ffff), Seed::Gpr(15, 1)],
    },
    Replay {
        loop_index: 5,
        words: &four_then_bdnz(0xfc00_0024),
        seeds: &[Seed::Fpr(0, ONE)],
    },
    Replay {
        loop_index: 6,
        words: &four_then_bdnz(0xfc00_002c),
        seeds: &[Seed::Fpr(0, ONE)],
    },
    Replay {
        loop_index: 7,
        words: &eight_then_bdnz(0xfc00_602b),
        seeds: &[Seed::Fpr(0, ONE), Seed::Fpr(12, 0)],
    },
    Replay {
        loop_index: 8,
        words: &eight_then_bdnz(0x5529_003f),
        seeds: &[Seed::Gpr(9, 5)],
    },
    Replay {
        loop_index: 9,
        words: &eight_then_bdnz(0x7d20_0026),
        seeds: &[],
    },
    Replay {
        loop_index: 10,
        words: &eight_then_bdnz(0xfc00_048e),
        seeds: &[],
    },
    Replay {
        loop_index: 11,
        words: &eight_then_bdnz(0x7d2c_42e6),
        seeds: &[],
    },
    Replay {
        loop_index: 12,
        words: &eight_then_bdnz(0x1042_10eb),
        seeds: &[],
    },
    Replay {
        loop_index: 13,
        words: &eight_then_bdnz(0x1042_1880),
        seeds: &[],
    },
    Replay {
        loop_index: 14,
        words: &eight_then_bdnz(0x1042_1848),
        seeds: &[],
    },
    Replay {
        loop_index: 15,
        words: &eight_then_bdnz(0x1040_110a),
        seeds: &[],
    },
    Replay {
        loop_index: 16,
        words: &[0xa9c8_0000, 0xa9e8_0008, 0xaa08_0010, 0xaa28_0018, BDNZ_4],
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    },
    Replay {
        loop_index: 17,
        words: &[0xbf08_0000, 0xbb08_0000, 0x4200_fff8],
        seeds: &[Seed::Gpr(8, ONE_LINE)],
    },
    // Four calls, each to a `blr` the `b` after it skips on the way back.
    Replay {
        loop_index: 18,
        words: &[
            0x4800_0009,
            0x4800_0008,
            0x4e80_0020,
            0x4800_0009,
            0x4800_0008,
            0x4e80_0020,
            0x4800_0009,
            0x4800_0008,
            0x4e80_0020,
            0x4800_0009,
            0x4800_0008,
            0x4e80_0020,
            0x4200_ffd0,
        ],
        seeds: &[],
    },
    Replay {
        loop_index: 19,
        words: &eight_then_bdnz(0x7d4f_f120),
        seeds: &[Seed::Gpr(10, 0x2222_2222)],
    },
    Replay {
        loop_index: 20,
        words: &[
            0xc828_0040,
            0xfc21_602a,
            0xc828_0040,
            0xfc21_602a,
            0xc828_0040,
            0xfc21_602a,
            0xc828_0040,
            0xfc21_602a,
            BDNZ_8,
        ],
        seeds: &[Seed::Gpr(8, ONE_LINE), Seed::Fpr(12, 0)],
    },
];

/// The body of a four-instruction loop: the word four times, then the
/// loop's `bdnz`.
const fn four_then_bdnz(word: u32) -> [u32; 5] {
    [word, word, word, word, BDNZ_4]
}

const LCG_LOOP: [u32; 8] = [
    0x7dce_81d2,
    0x39ce_0001,
    0x79d1_0fe0,
    0x7e31_1838,
    0x2c31_0000,
    0x4182_0008,
    0x3929_0001,
    0x4200_ffe4,
];

/// The body of an eight-instruction loop: the word eight times, then the
/// loop's `bdnz`.
const fn eight_then_bdnz(word: u32) -> [u32; 9] {
    [word, word, word, word, word, word, word, word, BDNZ_8]
}

fn write_u64(mem: &mut GuestMemory, at: u64, v: u64) {
    let range = ByteRange::new(GuestAddr::new(at), 8).expect("range");
    mem.apply_commit(range, &v.to_be_bytes()).expect("mapped");
}

/// The probe's `build_ring`: one doubleword a 128-byte line, linked in
/// order or in its LCG-shuffled order. Returns the head.
fn build_ring(mem: &mut GuestMemory, base: u64, bytes: u64, shuffle: bool) -> u64 {
    let n = (bytes / LINE) as usize;
    let mut order: Vec<u64> = (0..n as u64).collect();
    if shuffle {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        for i in (1..n).rev() {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let j = ((seed >> 33) % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
    }
    for i in 0..n {
        let next = base + order[(i + 1) % n] * LINE;
        write_u64(mem, base + order[i] * LINE, next);
    }
    base + order[0] * LINE
}

/// Where a replay places its siblings' loop.
const SIBLING_CODE: u64 = 0x1800;

/// The probe's sibling thread: sixteen independent `addi`, alternating
/// r15 and r16, then its `bdnz`.
const SIBLING_LOOP: [u32; 17] = [
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x39ef_0001,
    0x3a10_0001,
    0x4200_ffc0,
];

/// A loop each sibling unit runs from [`SIBLING_CODE`], and the
/// registers it starts with.
pub(crate) struct Sibling {
    pub(crate) words: &'static [u32],
    pub(crate) seeds: &'static [Seed],
}

const PROBE_SIBLING: Sibling = Sibling {
    words: &SIBLING_LOOP,
    seeds: &[],
};

/// Run one replay's loop twice `REPLAY_ITERATIONS` times, one
/// instruction a step, beside `siblings` PPU units that run the probe's
/// sibling loop.
///
/// Returns the cycles an iteration of the second half, or `None` when
/// the unit faults.
pub(crate) fn replay(r: &Replay, siblings: usize) -> Option<f64> {
    replay_beside(r, siblings, &PROBE_SIBLING)
}

/// [`replay`] beside `siblings` PPU units that each run `sibling`.
pub(crate) fn replay_beside(r: &Replay, siblings: usize, sibling: &Sibling) -> Option<f64> {
    replay_measured(r, siblings, sibling).map(|m| m.cycles)
}

/// What [`replay_measured`] times over the loop's second half.
pub(crate) struct Measured {
    /// The loop's cycles an iteration.
    pub(crate) cycles: f64,
    /// The first sibling's own cycles an iteration of its loop, or `None`
    /// with no sibling or none of its iterations in the half.
    pub(crate) sibling_cycles: Option<f64>,
}

/// [`replay_beside`], with the first sibling's rate over the same half.
pub(crate) fn replay_measured(r: &Replay, siblings: usize, sibling: &Sibling) -> Option<Measured> {
    let words = r.words;
    let ring_end = r
        .seeds
        .iter()
        .filter_map(|s| match s {
            Seed::Ring(_, base, bytes, _) => Some(base + bytes),
            _ => None,
        })
        .fold(L1_RING, u64::max);
    let mut mem = GuestMemory::new(ring_end as usize);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    for (i, w) in sibling.words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(SIBLING_CODE + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let mut gprs = Vec::new();
    for seed in r.seeds {
        if let Seed::Ring(reg, base, bytes, shuffle) = *seed {
            gprs.push((reg, build_ring(&mut mem, base, bytes, shuffle)));
        }
    }
    let exit = CODE + 4 * words.len() as u64;

    let counter = r.seeds.iter().find_map(|s| match s {
        Seed::Counter(k) => Some(*k),
        _ => None,
    });
    let mut rt = Runtime::new(mem, Budget::new(1), usize::MAX);
    let seeds = r.seeds;
    let unit = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        let state = ppu.state_mut();
        state.pc = CODE;
        match counter {
            Some(k) => state.set_gpr(k, 2 * REPLAY_ITERATIONS),
            None => state.set_ctr(2 * REPLAY_ITERATIONS),
        }
        for seed in seeds {
            match *seed {
                Seed::Gpr(k, v) => state.set_gpr(k, v),
                Seed::Fpr(k, v) => state.set_fpr(k, v),
                Seed::Ring(..) | Seed::Counter(_) => {}
            }
        }
        for &(k, v) in &gprs {
            state.set_gpr(k, v);
        }
        ppu
    });
    let sibling_ids: Vec<_> = (0..siblings)
        .map(|_| {
            rt.register_unit_with(|id| {
                let mut ppu = PpuExecutionUnit::new(id);
                let state = ppu.state_mut();
                state.pc = SIBLING_CODE;
                state.set_ctr(u64::MAX);
                for seed in sibling.seeds {
                    match *seed {
                        Seed::Gpr(k, v) => state.set_gpr(k, v),
                        Seed::Fpr(k, v) => state.set_fpr(k, v),
                        Seed::Ring(..) | Seed::Counter(_) => {}
                    }
                }
                ppu
            })
        })
        .collect();

    let state = |rt: &Runtime| -> cellgov_ppu::state::PpuState {
        rt.registry()
            .get(unit)
            .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
            .expect("the PPU unit")
            .state()
            .clone()
    };
    // The count the loop's head sees after the warming half runs.
    let left = |s: &cellgov_ppu::state::PpuState| match counter {
        Some(k) => s.gpr[k],
        None => s.ctr(),
    };
    // The first sibling's pc, and its clock and count of passes through
    // its loop's head so far.
    let sibling_pc = |rt: &Runtime| {
        sibling_ids.first().map(|&id| {
            rt.registry()
                .get(id)
                .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
                .expect("the sibling PPU unit")
                .state()
                .pc
        })
    };
    let mut sibling_rounds = 0u64;
    let mut sibling_was_at_head = false;
    let sibling_mark = |rt: &Runtime, rounds: u64| {
        sibling_ids
            .first()
            .map(|&id| (rt.registry().clock(id).raw(), rounds))
    };
    let mut halfway = None;
    // A sibling's instructions cost fewer cycles than most loops', so the
    // scheduler gives it several steps for each of the loop's. A loop
    // with an inner loop runs up to four times its words an iteration.
    let bound =
        4 * (2 * REPLAY_ITERATIONS * (words.len() as u64 + 1) + 16) * (16 * siblings as u64 + 1);
    for _ in 0..bound {
        let s = state(&rt);
        if s.pc == exit {
            break;
        }
        if let Some(pc) = sibling_pc(&rt) {
            let at_head = pc == SIBLING_CODE;
            if at_head && !sibling_was_at_head {
                sibling_rounds += 1;
            }
            sibling_was_at_head = at_head;
        }
        if halfway.is_none() && s.pc == CODE && left(&s) == REPLAY_ITERATIONS {
            halfway = Some((
                rt.registry().clock(unit).raw(),
                sibling_mark(&rt, sibling_rounds),
            ));
        }
        if rt.registry().effective_status(unit) == Some(UnitStatus::Faulted) {
            return None;
        }
        let step = rt
            .step()
            .unwrap_or_else(|e| panic!("loop {} at pc {:#x}: {e:?}", r.loop_index, s.pc));
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(state(&rt).pc, exit, "loop {} ran to its exit", r.loop_index);
    for &s in &sibling_ids {
        assert_eq!(
            rt.registry().effective_status(s),
            Some(UnitStatus::Runnable),
            "the sibling ran beside loop {}",
            r.loop_index,
        );
    }
    let (halfway, sibling_half) = halfway.expect("the loop passed its halfway count");
    let sibling_cycles = match (sibling_half, sibling_mark(&rt, sibling_rounds)) {
        (Some((c0, n0)), Some((c1, n1))) if n1 > n0 => Some((c1 - c0) as f64 / (n1 - n0) as f64),
        _ => None,
    };
    Some(Measured {
        cycles: (rt.registry().clock(unit).raw() - halfway) as f64 / REPLAY_ITERATIONS as f64,
        sibling_cycles,
    })
}

/// The `ppu_issue_table` loop whose body the interpreter refuses:
/// `vrefp`.
const TABLE_REFUSED_LOOP: usize = 15;

#[test]
fn the_issue_model_times_each_table_loop_within_the_console_s_spread() {
    let w = table_words();
    let cycles = |k: usize, word: usize| {
        (w[word] * PPU_CYCLES_PER_TB_TICK) as f64 / (1u64 << TABLE_ITERATIONS_LOG2[k]) as f64
    };
    let mut rows = Vec::new();
    for r in &TABLE_REPLAYS {
        let Some(cellgov) = replay(r, 0) else {
            assert_eq!(
                r.loop_index, TABLE_REFUSED_LOOP,
                "table loop {} faults in the interpreter",
                r.loop_index,
            );
            continue;
        };
        assert_ne!(
            r.loop_index, TABLE_REFUSED_LOOP,
            "the interpreter now executes vrefp, so table loop 15 joins the replayed rows",
        );
        let fastest = cycles(r.loop_index, 2 * r.loop_index);
        let spread = cycles(r.loop_index, 2 * r.loop_index + 1) - fastest;
        rows.push((r.loop_index, cellgov, fastest, spread));
    }
    assert_eq!(
        rows.len(),
        20,
        "every table loop but the refused one replays"
    );
    for &(k, cellgov, fastest, spread) in &rows {
        let within = (cellgov - fastest).abs() <= spread;
        match TABLE_RESIDUALS.iter().find(|r| r.0 == k) {
            Some(&(_, below)) => {
                assert!(
                    !within,
                    "table loop {k} now lands within the console's spread; drop it \
                     from the residuals: {rows:?}",
                );
                let short = fastest - cellgov;
                assert!(
                    short > 0.0 && short < below,
                    "table loop {k}: CellGov {cellgov:.3} cycles an iteration, the \
                     console's fastest {fastest:.3}, more than {below} apart: {rows:?}",
                );
            }
            None => assert!(
                within,
                "table loop {k}: CellGov {cellgov:.3} cycles an iteration, the \
                 console's fastest {fastest:.3} with a spread of {spread:.3}: {rows:?}",
            ),
        }
    }
}

/// The `ppu_issue_table` loops the model times below the console's
/// spread, each with the cycles an iteration it may fall short by.
///
/// The dependent `mulli` and `mullw` loops take 65.76 and 90.83 cycles
/// an iteration. Like the dependent `add` loop, each multiply takes a
/// fraction of a cycle more than its whole latency of 8 or 11, which
/// ADR 0014 records as an accepted gap from the `ppu_issue_chain`
/// capture (`console_ppu_issue_chain.rs`). The
/// `stmw` and `lmw` pair to one line takes 138.8; the model's drain of
/// the store queue's 16-byte entries gives 120, as it does for the same
/// loop of the `ppu_issue_multiple` capture
/// (`console_ppu_issue_multiple.rs`).
const TABLE_RESIDUALS: [(usize, f64); 3] = [(1, 2.0), (2, 3.0), (17, 20.0)];

/// The loop whose body the interpreter refuses: `vaddfp`.
const REFUSED_LOOP: usize = 7;

/// The loop the model times outside the console's spread: eight
/// dependent `add`. The model's 16 cycles an iteration is the add's
/// latency eight times; the console's fastest run takes 16.28 and its
/// slowest 16.41. ADR 0014 records the excess as an accepted gap from
/// the `ppu_issue_chain` capture, where the same loop's fastest run
/// takes 16.51.
const RESIDUAL_LOOP: usize = 2;

#[test]
fn the_issue_model_times_each_loop_within_the_console_s_spread() {
    let w = console_words();
    let mut rows = Vec::new();
    for r in &REPLAYS {
        let Some(cellgov) = replay(r, 0) else {
            assert_eq!(
                r.loop_index, REFUSED_LOOP,
                "loop {} faults in the interpreter",
                r.loop_index,
            );
            continue;
        };
        assert_ne!(
            r.loop_index, REFUSED_LOOP,
            "the interpreter now executes vaddfp, so loop 7 joins the replayed rows",
        );
        let fastest = console_cycles(&w, r.loop_index);
        let spread = console_slowest(&w, r.loop_index) - fastest;
        rows.push((r.loop_index, cellgov, fastest, spread));
    }
    assert_eq!(rows.len(), 19, "every loop but the refused one replays");
    for &(k, cellgov, fastest, spread) in &rows {
        let within = (cellgov - fastest).abs() <= spread;
        if k == RESIDUAL_LOOP {
            assert!(
                !within,
                "loop {k} now lands within the console's spread; drop it from the \
                 residual: {rows:?}",
            );
            // A loop that moves farther than a cycle from the console's
            // fastest run is a regression.
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

/// The loops the probe times beside its busy sibling, each with the loop
/// whose body it repeats.
const SIBLING_LOOPS: [(usize, usize); 3] = [(20, 1), (21, 2), (22, 11)];

/// The `REPLAYS` entry for loop `k`.
pub(crate) fn replay_of(k: usize) -> &'static Replay {
    REPLAYS
        .iter()
        .find(|r| r.loop_index == k)
        .expect("a replay of the loop")
}

#[test]
fn the_issue_model_times_each_loop_beside_a_busy_sibling_within_the_console_s_spread() {
    let w = console_words();
    let mut rows = Vec::new();
    for (k, body) in SIBLING_LOOPS {
        let base = replay_of(body);
        let r = Replay {
            loop_index: k,
            words: base.words,
            seeds: base.seeds,
        };
        let cellgov = replay(&r, 1).expect("the loop replays");
        let fastest = console_cycles(&w, k);
        let spread = console_slowest(&w, k) - fastest;
        rows.push((k, cellgov, fastest, spread));
    }
    for &(k, cellgov, fastest, spread) in &rows {
        assert!(
            (cellgov - fastest).abs() <= spread,
            "loop {k}: CellGov {cellgov:.3} cycles an iteration, the console's \
             fastest {fastest:.3} with a spread of {spread:.3}: {rows:?}",
        );
    }
}
