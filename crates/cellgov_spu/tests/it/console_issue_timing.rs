//! CellGov's SPU issue timing against the `spu_issue_timing` probe's
//! console capture: forty-three loops of 65536 iterations, each timed
//! with the decrementer, run through the execution unit. Each loop must
//! take the console's cycles an iteration. Every build runs the loops at
//! 2048 iterations; a release build also runs them at 65536.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu_issue::SPU_CYCLES_PER_TB_TICK;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `tests/micro/spu_issue_timing/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), block by block. The
/// table omits the zero words and the result slots, because local store
/// starts zeroed.
const BLOCKS: [(u32, &[u32]); 94] = [
    (
        0x0000,
        &[
            0x413f_ff84,
            0x60ff_ff84,
            0x21a0_0384,
            0x4080_0004,
            0x4080_0089,
            0x4080_0085,
            0x4080_0006,
            0x4080_0007,
            0x3000_8000,
        ],
    ),
    (
        0x0400,
        &[0x4100_0083, 0x1200_2750, 0x01a0_0414, 0x3200_2600],
    ),
    (
        0x053c,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0016,
            0x3000_c000,
        ],
    ),
    (0x0600, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x0700,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0216,
            0x3001_0000,
        ],
    ),
    (0x0800, &[0x4100_0083, 0x01a0_0414, 0x3200_1f80]),
    (
        0x0904,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0416,
            0x3001_4000,
        ],
    ),
    (0x0a00, &[0x4100_0083, 0x01a0_0414, 0x3200_2000]),
    (
        0x0b08,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0616,
            0x3001_8000,
        ],
    ),
    (0x0c00, &[0x4100_0083, 0x01a0_0414, 0x3200_2080]),
    (
        0x0d0c,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0816,
            0x3001_c000,
        ],
    ),
    (0x0e00, &[0x4100_0083, 0x01a0_0414, 0x3200_2100]),
    (
        0x0f10,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0a16,
            0x3002_0000,
        ],
    ),
    (0x1000, &[0x4100_0083, 0x01a0_0414, 0x3200_2300]),
    (
        0x1120,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0c16,
            0x3002_4000,
        ],
    ),
    (0x1200, &[0x4100_0083, 0x01a0_0414, 0x3200_2500]),
    (
        0x1330,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0e16,
            0x3002_8000,
        ],
    ),
    (0x1400, &[0x4100_0083, 0x01a0_0414, 0x3200_2680]),
    (
        0x153c,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1016,
            0x3002_c000,
        ],
    ),
    (0x1600, &[0x4100_0083, 0x01a0_0414, 0x3200_2400]),
    (0x1728, &[0x4080_0009, 0x4100_0088, 0x3200_0700]),
    (
        0x1768,
        &[
            0x7802_0496,
            0x2100_0296,
            0x1c00_4489,
            0x01e0_000a,
            0x4020_007f,
            0x207f_fd8a,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1216,
            0x3003_0000,
        ],
    ),
    (
        0x1800,
        &[0x4100_0083, 0x1200_1fc2, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x1900,
        &[
            0x01e0_000a,
            0x1802_8183,
            0x1cff_c183,
            0x217f_fe83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1416,
            0x3003_4000,
        ],
    ),
    (
        0x1a00,
        &[0x4100_0083, 0x1200_1fc2, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x1b00,
        &[
            0x01e0_000a,
            0x01e0_000b,
            0x1cff_c183,
            0x217f_fe83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1616,
            0x3003_8000,
        ],
    ),
    (
        0x1c00,
        &[0x4100_0083, 0x1200_1fce, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x1d00,
        &[
            0x01e0_000a,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x01e0_000b,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x1cff_c183,
            0x217f_f883,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1816,
            0x3003_c000,
        ],
    ),
    (
        0x1e00,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x1f00,
        &[
            0x0040_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1a16,
            0x3004_0000,
        ],
    ),
    (
        0x2000,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x2100,
        &[
            0x0060_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1c16,
            0x3004_4000,
        ],
    ),
    (
        0x2200,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x2300,
        &[
            0x0050_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_1e16,
            0x3004_8000,
        ],
    ),
    (
        0x2400,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x2500,
        &[
            0x5981_c305,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2016,
            0x3004_c000,
        ],
    ),
    (0x2600, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x2700,
        &[
            0x1200_0809,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x2100_0384,
            0x1cff_c183,
            0x217f_fa83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2216,
            0x3005_0000,
            0x0000_2017,
        ],
    ),
    (0x2800, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x2900,
        &[
            0x0020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x2100_0384,
            0x1cff_c183,
            0x217f_fa83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2416,
            0x3005_4000,
            0x0000_2018,
        ],
    ),
    (0x2a00, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x2b00,
        &[
            0x1200_0003,
            0x4020_0000,
            0x1cff_c183,
            0x217f_fe83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2616,
            0x3005_8000,
        ],
    ),
    (0x2c00, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x2d00,
        &[
            0x1200_000c,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x1cff_c183,
            0x217f_fa03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2816,
            0x3005_c000,
        ],
    ),
    (
        0x2e00,
        &[0x4100_0083, 0x1200_1fc6, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x2f00,
        &[
            0x1cff_c183,
            0x0020_0000,
            0x4020_0000,
            0x0020_0000,
            0x4020_0000,
            0x0020_0000,
            0x4020_0000,
            0x217f_fc83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2a16,
            0x3006_0000,
        ],
    ),
    (
        0x3000,
        &[0x4100_0083, 0x1200_2047, 0x01a0_0414, 0x3200_1f00],
    ),
    (
        0x3104,
        &[
            0x1cff_c183,
            0x0020_0000,
            0x4020_0000,
            0x0020_0000,
            0x4020_0000,
            0x0020_0000,
            0x4020_0000,
            0x217f_fc83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2c16,
            0x3006_4000,
        ],
    ),
    (
        0x3200,
        &[0x4100_0083, 0x1200_1fc2, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x3300,
        &[
            0x308f_e005,
            0x1801_4183,
            0x1cff_c183,
            0x217f_fe83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_2e16,
            0x3006_8000,
        ],
    ),
    (
        0x3400,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x3500,
        &[
            0x3f80_0183,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3016,
            0x3006_c000,
        ],
    ),
    (
        0x3600,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x3700,
        &[
            0x7882_4285,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3216,
            0x3007_0000,
        ],
    ),
    (
        0x3800,
        &[0x4100_0083, 0x1200_1fc1, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x3900,
        &[
            0x2100_0384,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3416,
            0x3007_4000,
            0x0000_2026,
        ],
    ),
    (
        0x3a00,
        &[0x4100_0083, 0x1200_1fc6, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x3b00,
        &[
            0x4020_0000,
            0x2100_0584,
            0x1cff_c183,
            0x0020_0000,
            0x4020_0000,
            0x0020_0000,
            0x4020_0000,
            0x217f_fc83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3616,
            0x3007_8000,
            0x0000_2027,
        ],
    ),
    (
        0x3c00,
        &[0x4100_0083, 0x1200_2043, 0x01a0_0414, 0x3200_1f00],
    ),
    (
        0x3d04,
        &[
            0x01e0_000a,
            0x2100_038a,
            0x1cff_c183,
            0x217f_fe83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3816,
            0x3007_c000,
            0x0000_2028,
        ],
    ),
    (
        0x3e00,
        &[0x4100_0083, 0x1200_2043, 0x01a0_0414, 0x3200_1f00],
    ),
    (
        0x3f04,
        &[
            0x7802_4216,
            0x2100_0396,
            0x1cff_c183,
            0x217f_fe83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3a16,
            0x3008_0000,
            0x0000_2029,
        ],
    ),
    (0x4000, &[0x4100_0083, 0x01a0_0414, 0x3200_2400]),
    (0x4128, &[0x4080_0009, 0x4100_0088, 0x3200_0700]),
    (
        0x4168,
        &[
            0x7802_0496,
            0x2100_0296,
            0x1c00_4489,
            0x308f_e00a,
            0x4020_007f,
            0x207f_fd8a,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3c16,
            0x3008_4000,
        ],
    ),
    (0x4200, &[0x4100_0083, 0x01a0_0414, 0x3200_2400]),
    (0x4328, &[0x4080_0009, 0x4100_0088, 0x3200_0700]),
    (
        0x4368,
        &[
            0x7802_0496,
            0x2100_0396,
            0x1c00_4489,
            0x0020_0000,
            0x4020_007f,
            0x01e0_000a,
            0x4020_007f,
            0x207f_fc8a,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_3e16,
            0x3008_8000,
        ],
    ),
    (0x4400, &[0x4100_0083, 0x01a0_0414, 0x3200_2400]),
    (0x4528, &[0x4080_0009, 0x4100_0088, 0x3200_0700]),
    (
        0x4568,
        &[
            0x4020_007f,
            0x01e0_000a,
            0x7802_0496,
            0x2100_0296,
            0x1c00_4489,
            0x0020_0000,
            0x4020_007f,
            0x207f_fc8a,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4016,
            0x3008_c000,
        ],
    ),
    (0x4600, &[0x4100_0083, 0x01a0_0414, 0x3200_2400]),
    (0x4728, &[0x4080_0009, 0x4100_0088, 0x3200_0700]),
    (
        0x4768,
        &[
            0x4020_007f,
            0x308f_e00a,
            0x7802_0496,
            0x2100_0296,
            0x1c00_4489,
            0x0020_0000,
            0x4020_007f,
            0x207f_fc8a,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4216,
            0x3009_0000,
        ],
    ),
    (0x4800, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x4900,
        &[
            0x4020_0000,
            0x2100_0584,
            0x4020_0000,
            0x0020_0000,
            0x1cff_c183,
            0x0020_0000,
            0x4020_0000,
            0x217f_fc83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4416,
            0x3009_4000,
            0x0000_2034,
        ],
    ),
    (0x4a00, &[0x4100_0083, 0x01a0_0414, 0x3200_1f00]),
    (
        0x4b00,
        &[
            0x4020_0000,
            0x0020_0000,
            0x4020_0000,
            0x2100_0484,
            0x1cff_c183,
            0x0020_0000,
            0x4020_0000,
            0x217f_fc83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4616,
            0x3009_8000,
            0x0000_2035,
        ],
    ),
    (0x4c00, &[0x4100_0083, 0x01a0_0414, 0x3200_2700]),
    (
        0x4d40,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4816,
            0x3009_c000,
        ],
    ),
    (0x4e00, &[0x4100_0083, 0x01a0_0414, 0x3200_2800]),
    (
        0x4f48,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4a16,
            0x300a_0000,
        ],
    ),
    (0x5000, &[0x4100_0083, 0x01a0_0414, 0x3200_2b00]),
    (
        0x5160,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4c16,
            0x300a_4000,
        ],
    ),
    (0x5200, &[0x4100_0083, 0x01a0_0414, 0x3200_2c00]),
    (
        0x5368,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_4e16,
            0x300a_8000,
        ],
    ),
    (0x5400, &[0x4100_0083, 0x01a0_0414, 0x3200_2d00]),
    (
        0x5570,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_5016,
            0x300a_c000,
        ],
    ),
    (0x5600, &[0x4100_0083, 0x01a0_0414, 0x3200_2e80]),
    (
        0x577c,
        &[
            0x4020_0000,
            0x1cff_c183,
            0x217f_ff03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_5216,
            0x300b_0000,
        ],
    ),
    (0x5800, &[0x4100_0083, 0x01a0_0414, 0x3200_2400]),
    (0x5928, &[0x4080_0009, 0x4100_0088, 0x3200_0f00]),
    (
        0x59a8,
        &[
            0x7802_0496,
            0x2100_0296,
            0x1c00_4489,
            0x01e0_000a,
            0x4020_007f,
            0x207f_fd8a,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_5416,
            0x300b_4000,
        ],
    ),
    (0x5a00, &[0x0000_2000]),
];

/// The loops the probe times, and the iterations of each on the console.
const CASES: usize = 43;
const ITERATIONS: u64 = 65536;

/// The iterations of each loop in the CI run. The model takes a whole
/// number of cycles for each iteration. A shorter loop changes a count
/// only by the cycles outside the loop and one decrementer tick, divided
/// by the iterations. Measured: at most 0.05 cycles from the count at
/// [`ITERATIONS`], so every loop rounds to the same cycle.
const SHORT_ITERATIONS: u16 = 2048;
// `il` sign-extends its immediate.
const _: () = assert!(SHORT_ITERATIONS <= 0x7fff);

/// `ilhu r3, 1` and `ilhu r8, 1`: the probe sets each loop's count to
/// 65536 with one of these.
const LOOP_COUNTS: [(u32, usize); 2] = [(0x4100_0083, CASES), (0x4100_0088, 6)];

/// `il rt, imm`, with the register field of `ilhu`.
fn il(ilhu: u32, imm: u16) -> u32 {
    0x4080_0000 | (u32::from(imm) << 7) | (ilhu & 0x7f)
}

/// Where the program stores each loop's decrementer delta: the preferred
/// slot of the quadword at `SLOTS_LS + 16 k`.
const SLOTS_LS: usize = 0x8000;

const CONSOLE: &str = "../../tests/micro/spu_issue_timing/ps3/cech20-cex-493/observation.json";

/// The console's decrementer delta for each loop: the payload's slots
/// from offset 32.
fn console_deltas() -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 224);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    assert_eq!(word(16), 0x2000_0002, "the SPU stopped with 0x2000");
    (0..CASES).map(|k| word(32 + 4 * k)).collect()
}

/// Converts a decrementer delta over a loop of `iterations` to cycles an
/// iteration, rounded to the nearest cycle.
fn cycles_per_iteration(delta: u32, iterations: u64) -> u64 {
    (u64::from(delta) * SPU_CYCLES_PER_TB_TICK + iterations / 2) / iterations
}

fn run_to_stop(rt: &mut Runtime, unit: UnitId) {
    for _ in 0..100_000 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            return;
        }
        let step = rt.step().expect("the SPU runs");
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
    }
    panic!("the SPU never stopped");
}

/// Runs the probe with each loop's count set to `iterations` (the
/// console's when `None`), and returns each loop's cycles an iteration.
fn cellgov_cycles(iterations: Option<u16>) -> Vec<u64> {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(10_000), 100_000);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let mut rewritten = [0; LOOP_COUNTS.len()];
        for (base, words) in BLOCKS {
            for (i, &word) in words.iter().enumerate() {
                let mut word = word;
                if let (Some(n), Some(c)) =
                    (iterations, LOOP_COUNTS.iter().position(|&(w, _)| w == word))
                {
                    word = il(word, n);
                    rewritten[c] += 1;
                }
                let at = base as usize + i * 4;
                state.ls[at..at + 4].copy_from_slice(&word.to_be_bytes());
            }
        }
        if iterations.is_some() {
            assert_eq!(
                rewritten.to_vec(),
                LOOP_COUNTS.map(|(_, n)| n).to_vec(),
                "every loop's count was rewritten"
            );
        }
        spu
    });
    run_to_stop(&mut rt, unit);

    let spu = rt
        .registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit");
    let ls = &spu.state().ls;
    let n = iterations.map_or(ITERATIONS, u64::from);
    (0..CASES)
        .map(|k| {
            let at = SLOTS_LS + 16 * k;
            cycles_per_iteration(
                u32::from_be_bytes(ls[at..at + 4].try_into().expect("4 bytes")),
                n,
            )
        })
        .collect()
}

fn console_cycles() -> Vec<u64> {
    console_deltas()
        .into_iter()
        .map(|delta| cycles_per_iteration(delta, ITERATIONS))
        .collect()
}

#[test]
fn every_loop_takes_the_consoles_cycles_an_iteration() {
    assert_eq!(
        cellgov_cycles(Some(SHORT_ITERATIONS)),
        console_cycles(),
        "cycles an iteration, loop by loop"
    );
}

/// The probe's own iteration counts: under a second in a release build
/// and up to 18 s in a debug one, so only the release run holds it.
#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "the full-length run; the release test run holds it"
)]
fn the_full_length_run_gives_the_short_runs_cycles_an_iteration() {
    let full = cellgov_cycles(None);
    assert_eq!(
        full,
        cellgov_cycles(Some(SHORT_ITERATIONS)),
        "full and short runs, loop by loop"
    );
    assert_eq!(full, console_cycles(), "cycles an iteration, loop by loop");
}
