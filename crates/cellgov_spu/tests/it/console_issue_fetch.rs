//! CellGov's SPU issue timing against the `spu_issue_fetch` probe's
//! console capture: sixteen loops of 65536 iterations of lqa or stqa,
//! each timed with the decrementer, run through the execution unit.
//! Each loop must take the console's cycles an iteration. Every build runs
//! the loops at 1024 iterations; a release build also runs them at 8192.

use crate::console_issue_runout::{
    console_slots, cycles_per_iteration, run_blocks, tail, Block, CHECK_ITERATIONS,
    SHORT_ITERATIONS,
};

/// A case's head: `ilhu`, the decrementer read, the hint and the branch
/// into its loop.
const fn head(hint: u32, br: u32) -> [u32; 4] {
    [0x4100_0083, 0x01a0_0414, hint, br]
}

const LQA: &[u32] = &[0x3092_000a];
const STQA: &[u32] = &[0x2092_000b];
const LNOP: &[u32] = &[0x0020_0000];

/// `tests/micro/spu_issue_fetch/spu/main.S` as the SPU assembler encodes
/// it (`spu-objdump` of the built image), block by block, each block a
/// list of repeated runs. The table omits the zero words and the result
/// slots, because local store starts zeroed.
const BLOCKS: [Block; 34] = [
    (
        0x0000,
        &[(&[0x413f_ff84, 0x60ff_ff84, 0x21a0_0384, 0x3000_8000], 1)],
    ),
    (0x0400, &[(&head(0x1200_0f5f, 0x3200_0e80), 1)]),
    (
        0x0480,
        &[(LQA, 64), (&tail(0x2090_0016, 0x3001_4000, 0x217f_df83), 1)],
    ),
    (0x0a00, &[(&head(0x1200_0f67, 0x3200_0e80), 1)]),
    (
        0x0a80,
        &[(LQA, 72), (&tail(0x2090_0216, 0x3002_0000, 0x217f_db83), 1)],
    ),
    (0x1000, &[(&head(0x1200_0f6f, 0x3200_0e80), 1)]),
    (
        0x1080,
        &[(LQA, 80), (&tail(0x2090_0416, 0x3002_c000, 0x217f_d783), 1)],
    ),
    (0x1600, &[(&head(0x1200_0f77, 0x3200_0e80), 1)]),
    (
        0x1680,
        &[(LQA, 88), (&tail(0x2090_0616, 0x3003_8000, 0x217f_d383), 1)],
    ),
    (0x1c00, &[(&head(0x1200_0f7f, 0x3200_0e80), 1)]),
    (
        0x1c80,
        &[(LQA, 96), (&tail(0x2090_0816, 0x3004_4000, 0x217f_cf83), 1)],
    ),
    (0x2200, &[(&head(0x1280_0f07, 0x3200_0e80), 1)]),
    (
        0x2280,
        &[
            (LQA, 104),
            (&tail(0x2090_0a16, 0x3005_0000, 0x217f_cb83), 1),
        ],
    ),
    (0x2800, &[(&head(0x1280_0f0f, 0x3200_0e80), 1)]),
    (
        0x2880,
        &[
            (LQA, 112),
            (&tail(0x2090_0c16, 0x3005_c000, 0x217f_c783), 1),
        ],
    ),
    (0x2e00, &[(&head(0x1280_0f17, 0x3200_0e80), 1)]),
    (
        0x2e80,
        &[
            (LQA, 120),
            (&tail(0x2090_0e16, 0x3006_8000, 0x217f_c383), 1),
        ],
    ),
    (0x3400, &[(&head(0x1280_0f1f, 0x3200_0e80), 1)]),
    (
        0x3480,
        &[
            (LQA, 128),
            (&tail(0x2090_1016, 0x3007_4000, 0x217f_bf83), 1),
        ],
    ),
    (0x3a00, &[(&head(0x1280_0f3f, 0x3200_0e80), 1)]),
    (
        0x3a80,
        &[
            (LQA, 160),
            (&tail(0x2090_1216, 0x3008_0000, 0x217f_af83), 1),
        ],
    ),
    (0x4000, &[(&head(0x1280_0f5f, 0x3200_0e80), 1)]),
    (
        0x4080,
        &[
            (LQA, 192),
            (&tail(0x2090_1416, 0x3008_c000, 0x217f_9f83), 1),
        ],
    ),
    (0x4600, &[(&head(0x1280_0f7f, 0x3200_0e80), 1)]),
    (
        0x4680,
        &[
            (LQA, 224),
            (&tail(0x2090_1616, 0x3009_8000, 0x217f_8f83), 1),
        ],
    ),
    (0x4c00, &[(&head(0x1280_0f1f, 0x3200_0e80), 1)]),
    (
        0x4c80,
        &[
            (STQA, 128),
            (&tail(0x2090_1816, 0x300a_4000, 0x217f_bf83), 1),
        ],
    ),
    (0x5200, &[(&head(0x1280_0f20, 0x3200_0e80), 1)]),
    (
        0x5280,
        &[
            (LQA, 64),
            (LNOP, 1),
            (LQA, 64),
            (&tail(0x2090_1a16, 0x300b_0000, 0x217f_bf03), 1),
        ],
    ),
    (0x5800, &[(&head(0x1280_0f23, 0x3200_0e80), 1)]),
    (
        0x5880,
        &[
            (LQA, 32),
            (LNOP, 1),
            (LQA, 32),
            (LNOP, 1),
            (LQA, 32),
            (LNOP, 1),
            (LQA, 32),
            (LNOP, 1),
            (&tail(0x2090_1c16, 0x300b_c000, 0x217f_bd83), 1),
        ],
    ),
    (0x5e00, &[(&head(0x1280_172f, 0x3200_1680), 1)]),
    (
        0x5ec0,
        &[
            (LQA, 128),
            (&tail(0x2090_1e16, 0x300c_8000, 0x217f_bf83), 1),
        ],
    ),
    (0x6400, &[(&[0x0000_2000], 1)]),
];

/// The loops the probe times, and the iterations of each.
const CASES: usize = 16;
const ITERATIONS: u64 = 65536;

const CONSOLE: &str = "../../tests/micro/spu_issue_fetch/ps3/cech20-cex-493/observation.json";

/// The loop whose target lies 64 bytes into its fetch line.
const MID_LINE: usize = 15;

/// CellGov's cycles an iteration of each case, with each loop shortened
/// to `short` iterations.
fn cellgov_cycles(short: u16) -> Vec<u64> {
    run_blocks(&BLOCKS, 0x1000, CASES, (short, CASES))
        .into_iter()
        .map(|delta| cycles_per_iteration(delta, u64::from(short)))
        .collect()
}

/// CellGov's cycles an iteration of each case in the CI run, and the
/// console's.
fn cycles() -> (Vec<u64>, Vec<u64>) {
    let console = console_slots(CONSOLE, 96, CASES)
        .into_iter()
        .map(|delta| cycles_per_iteration(delta, ITERATIONS))
        .collect();
    (cellgov_cycles(SHORT_ITERATIONS), console)
}

#[test]
fn every_aligned_fetch_loop_takes_the_consoles_cycles_an_iteration() {
    let (mut cellgov, mut console) = cycles();
    cellgov.remove(MID_LINE);
    console.remove(MID_LINE);
    assert_eq!(cellgov, console, "cycles an iteration, loop by loop");
}

/// The console takes one cycle more an iteration for the runout of the
/// loop whose target lies 64 bytes into its line. Whether its prefetch
/// holds lines or instructions from the target, the capture has one loop
/// to tell by, so the model keeps whole lines.
#[test]
fn a_runout_after_a_mid_line_target_takes_one_cycle_more_on_the_console() {
    let (cellgov, console) = cycles();
    assert_eq!(console[MID_LINE], cellgov[MID_LINE] + 1);
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
        "long and short runs, loop by loop"
    );
}
