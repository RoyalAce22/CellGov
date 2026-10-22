//! CellGov's SPU issue timing against the `spu_issue_runout` probe's
//! console capture: ten loops timed with the decrementer, run through
//! the execution unit. Six run lnop, lqa or stqa, alone or paired with
//! nop; four run lnop or lqa with and without sixteen 16 KB gets in
//! flight. Each loop must take the console's cycles an iteration, except
//! the lqa loop beside the gets, which the model times as without them.
//! Every build runs the 65536-iteration loops at 1024 iterations; a
//! release build also runs them at 8192.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu_issue::SPU_CYCLES_PER_TB_TICK;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// One run of a block: `words` repeated `times` times.
pub(crate) type Run = (&'static [u32], usize);

/// A program block: its local-store address and its runs.
pub(crate) type Block = (u32, &'static [Run]);

/// `tests/micro/spu_issue_runout/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), block by block, each
/// block a list of repeated runs. The table omits the zero words and the
/// result slots, because local store starts zeroed.
const BLOCKS: [Block; 22] = [
    (
        0x0000,
        &[(
            &[
                0x413f_ff84,
                0x60ff_ff84,
                0x21a0_0384,
                0x308f_e01f,
                0x4080_0020,
                0x21a0_08a0,
                0x4100_00a1,
                0x40a0_0022,
                0x4080_00a3,
                0x4080_2024,
                0x4080_0125,
                0x4080_0126,
                0x3000_8000,
            ],
            1,
        )],
    ),
    (
        0x0400,
        &[(&[0x4100_0083, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x0580,
        &[
            (&[0x0020_0000], 64),
            (&tail(0x2090_0016, 0x3001_8000, 0x217f_df83), 1),
        ],
    ),
    (
        0x0c00,
        &[(&[0x4100_0083, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x0d80,
        &[
            (&[0x3092_000a], 64),
            (&tail(0x2090_0216, 0x3002_8000, 0x217f_df83), 1),
        ],
    ),
    (
        0x1400,
        &[(&[0x4100_0083, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x1580,
        &[
            (&[0x2092_000b], 64),
            (&tail(0x2090_0416, 0x3003_8000, 0x217f_df83), 1),
        ],
    ),
    (
        0x1c00,
        &[(&[0x4100_0083, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x1d80,
        &[
            (&[0x4020_0000, 0x0020_0000], 32),
            (&tail(0x2090_0616, 0x3004_8000, 0x217f_df83), 1),
        ],
    ),
    (
        0x2400,
        &[(&[0x4100_0083, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x2580,
        &[
            (&[0x4020_0000, 0x3092_000a], 32),
            (&tail(0x2090_0816, 0x3005_8000, 0x217f_df83), 1),
        ],
    ),
    (
        0x2c00,
        &[(&[0x4100_0083, 0x1280_2fe0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x2d80,
        &[
            (&[0x3092_000a], 128),
            (&tail(0x2090_0a16, 0x3006_8000, 0x217f_bf83), 1),
        ],
    ),
    (
        0x3400,
        &[
            (&GETS, 16),
            (&[0x4081_0003, 0x1200_07d0, 0x01a0_0414, 0x3200_0680], 1),
        ],
    ),
    (
        0x3580,
        &[
            (&[0x0020_0000], 64),
            (&tail_wait(0x2090_0c16, 0x2090_1416, 0x3007_8000), 1),
        ],
    ),
    (
        0x3c00,
        &[(&[0x4081_0003, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x3d80,
        &[
            (&[0x0020_0000], 64),
            (&tail(0x2090_0e16, 0x3008_8000, 0x217f_df83), 1),
        ],
    ),
    (
        0x4400,
        &[
            (&GETS, 16),
            (&[0x4081_0003, 0x1200_07d0, 0x01a0_0414, 0x3200_0680], 1),
        ],
    ),
    (
        0x4580,
        &[
            (&[0x3092_000a], 64),
            (&tail_wait(0x2090_1016, 0x2090_1616, 0x3009_8000), 1),
        ],
    ),
    (
        0x4c00,
        &[(&[0x4081_0003, 0x1280_2fa0, 0x01a0_0414, 0x3200_2e80], 1)],
    ),
    (
        0x4d80,
        &[
            (&[0x3092_000a], 64),
            (&tail(0x2090_1216, 0x300a_8000, 0x217f_df83), 1),
        ],
    ),
    (0x5400, &[(&[0x0000_2000], 1)]),
];

/// One queued 16 KB get: `wrch` of the LSA, EAL, size, tag and command.
const GETS: [u32; 5] = [
    0x21a0_0821,
    0x21a0_091f,
    0x21a0_09a2,
    0x21a0_0a23,
    0x21a0_0aa4,
];

/// A loop's `ai` and `brnz`, then the decrementer read, the subtract,
/// the store of the count and the branch to the next case.
pub(crate) const fn tail(store: u32, next: u32, brnz: u32) -> [u32; 6] {
    [0x1cff_c183, brnz, 0x01a0_0415, 0x0805_0a96, store, next]
}

/// [`tail`] for a case that then waits for tag 1 and stores the count
/// to the end of the wait.
const fn tail_wait(store: u32, wait_store: u32, next: u32) -> [u32; 12] {
    [
        0x1cff_c183,
        0x217f_df83,
        0x01a0_0415,
        0x0805_0a96,
        store,
        0x21a0_0b25,
        0x21a0_0ba6,
        0x01a0_0c27,
        0x01a0_0415,
        0x0805_0a96,
        wait_store,
        next,
    ]
}

/// The loops the probe times, and the iterations of each.
const CASES: usize = 10;
const ITERATIONS: [u64; CASES] = [65536, 65536, 65536, 65536, 65536, 65536, 512, 512, 512, 512];

/// The lqa loop with the gets in flight, and the same loop without them.
const LQA_BESIDE_GETS: usize = 8;
const LQA_ALONE: usize = 9;

/// The slots holding the count to the end of case 6's and case 8's tag
/// wait.
const WAIT_SLOTS: [(usize, usize); 2] = [(6, 10), (8, 11)];

const CONSOLE: &str = "../../tests/micro/spu_issue_runout/ps3/cech20-cex-493/observation.json";

/// The console's decrementer count in each of `slots` slots, from a
/// capture whose payload is `len` bytes: the payload's words from offset
/// 32.
pub(crate) fn console_slots(path: &str, len: usize, slots: usize) -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), len);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    assert_eq!(word(16), 0x2000_0002, "the SPU stopped with 0x2000");
    (0..slots).map(|k| word(32 + 4 * k)).collect()
}

/// Converts a decrementer delta over a loop of `iterations` to cycles an
/// iteration, rounded to the nearest cycle.
pub(crate) fn cycles_per_iteration(delta: u32, iterations: u64) -> u64 {
    (u64::from(delta) * SPU_CYCLES_PER_TB_TICK + iterations / 2) / iterations
}

fn run_to_stop(rt: &mut Runtime, unit: UnitId) {
    for _ in 0..1_000_000 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            return;
        }
        let step = rt.step().expect("the SPU runs");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    panic!("the SPU never stopped");
}

/// `ilhu r3, 1`: the probes set the count of each 65536-iteration loop
/// with it.
const ILHU_65536: u32 = 0x4100_0083;

/// The iterations of each 65536-iteration loop in the CI run. The model
/// takes a whole number of cycles for each iteration. A shorter loop
/// changes a count only by the cycles outside the loop and one
/// decrementer tick, divided by the iterations. A release build measured
/// each loop of this file and `console_issue_fetch` against 65536
/// iterations: each is at most 0.062 cycles from its full-length count,
/// and at least 0.39 cycles from a rounding boundary.
pub(crate) const SHORT_ITERATIONS: u16 = 1024;

/// The iterations of the release build's check that a longer run rounds
/// to the same cycles. The full 65536 take 10 s in a release build of
/// `console_issue_fetch`.
pub(crate) const CHECK_ITERATIONS: u16 = 8192;
// `il` sign-extends its immediate.
const _: () = assert!(SHORT_ITERATIONS < CHECK_ITERATIONS && CHECK_ITERATIONS <= 0x7fff);

/// `il r3, imm`.
const fn il_r3(imm: u16) -> u32 {
    0x4080_0000 | ((imm as u32) << 7) | (ILHU_65536 & 0x7f)
}

/// Runs the program `blocks` on one SPU beside `memory` bytes of guest
/// memory, and returns the word in each of `slots` result slots: the
/// preferred slot of the quadword at `0x8000 + 16 k`.
///
/// With `short` as `(n, loops)`, each 65536-iteration loop runs `n`
/// iterations, and the program must hold exactly `loops` such loops.
pub(crate) fn run_blocks(
    blocks: &[Block],
    memory: usize,
    slots: usize,
    short: (u16, usize),
) -> Vec<u32> {
    let mut rt = Runtime::new(GuestMemory::new(memory), Budget::new(10_000), 1_000_000);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let mut rewritten = 0;
        for (base, runs) in blocks {
            let mut at = *base as usize;
            for (words, times) in *runs {
                for _ in 0..*times {
                    for &word in *words {
                        let word = if word == ILHU_65536 {
                            rewritten += 1;
                            il_r3(short.0)
                        } else {
                            word
                        };
                        state.ls[at..at + 4].copy_from_slice(&word.to_be_bytes());
                        at += 4;
                    }
                }
            }
        }
        assert_eq!(rewritten, short.1, "every loop's count was rewritten");
        spu
    });
    run_to_stop(&mut rt, unit);

    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    let ls = &spu.state().ls;
    (0..slots)
        .map(|k| {
            let at = 0x8000 + 16 * k;
            u32::from_be_bytes(ls[at..at + 4].try_into().expect("4 bytes"))
        })
        .collect()
}

/// The cases that run 65536 iterations on the console.
const LONG_CASES: usize = 6;

/// CellGov's cycles an iteration of each case, with each 65536-iteration
/// loop shortened to `short` iterations.
fn cellgov_cycles(short: u16) -> Vec<u64> {
    // The gets read 16 KB from effective address 0, the word at 0x7F00.
    run_blocks(&BLOCKS, 0x4000, CASES, (short, LONG_CASES))
        .into_iter()
        .zip(ITERATIONS)
        .map(|(delta, iterations)| match iterations {
            65536 => cycles_per_iteration(delta, u64::from(short)),
            _ => cycles_per_iteration(delta, iterations),
        })
        .collect()
}

/// CellGov's cycles an iteration of each case in the CI run, and the
/// console's.
fn cycles() -> (Vec<u64>, Vec<u64>) {
    let console = console_slots(CONSOLE, 80, CASES)
        .into_iter()
        .zip(ITERATIONS)
        .map(|(delta, iterations)| cycles_per_iteration(delta, iterations))
        .collect();
    (cellgov_cycles(SHORT_ITERATIONS), console)
}

#[test]
fn the_gets_outlast_the_loops_they_run_beside_on_the_console() {
    let console = console_slots(CONSOLE, 80, 12);
    for (case, slot) in WAIT_SLOTS {
        assert!(
            console[slot] > console[case],
            "case {case}: the tag wait ends after the loop"
        );
    }
}

#[test]
fn every_runout_loop_takes_the_consoles_cycles_an_iteration() {
    let (cellgov, console) = cycles();
    let without = |v: &[u64]| -> Vec<u64> {
        v.iter()
            .enumerate()
            .filter(|&(k, _)| k != LQA_BESIDE_GETS)
            .map(|(_, &c)| c)
            .collect()
    };
    assert_eq!(
        without(&cellgov),
        without(&console),
        "cycles an iteration, loop by loop"
    );
}

/// The model leaves out local-store contention with DMA: on the console
/// the gets slow a loop of loads, and in CellGov they do not.
#[test]
fn the_model_times_loads_beside_gets_as_loads_alone() {
    let (cellgov, console) = cycles();
    assert!(
        console[LQA_BESIDE_GETS] > console[LQA_ALONE],
        "the console's gets slow the loads"
    );
    assert_eq!(cellgov[LQA_BESIDE_GETS], cellgov[LQA_ALONE]);
}

/// Eight times the short run's iterations: too slow for a debug build, so
/// only the release run holds it.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "the long run; the release test run holds it"
)]
fn a_longer_run_gives_the_short_runs_cycles_an_iteration() {
    assert_eq!(
        cellgov_cycles(CHECK_ITERATIONS),
        cellgov_cycles(SHORT_ITERATIONS),
        "long and short runs, case by case"
    );
}
