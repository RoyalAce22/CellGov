//! CellGov's SPU issue timing against the `spu_issue_retire` probe's
//! console capture: five loops of 65536 iterations, each timed with the
//! decrementer, run through the execution unit. Each loop puts a
//! 2-cycle instruction 3, 4 or 5 cycles after a 6-cycle one on the even
//! pipeline, and must take the console's cycles an iteration.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu_issue::SPU_CYCLES_PER_TB_TICK;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `tests/micro/spu_issue_retire/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), block by block. The
/// table omits the zero words and the result slots, because local store
/// starts zeroed.
const BLOCKS: [(u32, &[u32]); 12] = [
    (
        0x0000,
        &[
            0x413f_ff84,
            0x60ff_ff84,
            0x21a0_0384,
            0x4080_0005,
            0x4080_0006,
            0x4080_0007,
            0x3000_8000,
        ],
    ),
    (
        0x0400,
        &[0x4100_0083, 0x1200_1fc4, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x0500,
        &[
            0x5881_828a,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x1cff_c183,
            0x217f_fd83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0016,
            0x3000_c000,
        ],
    ),
    (
        0x0600,
        &[0x4100_0083, 0x1200_1fc4, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x0700,
        &[
            0x5881_828a,
            0x4020_0000,
            0x4020_0000,
            0x1cff_c183,
            0x4020_0000,
            0x217f_fd83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0216,
            0x3001_0000,
        ],
    ),
    (
        0x0800,
        &[0x4100_0083, 0x1200_1fc4, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x0900,
        &[
            0x58c1_828a,
            0x1801_c30b,
            0x1801_c30c,
            0x1801_c30d,
            0x1cff_c183,
            0x217f_fd83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0416,
            0x3001_4000,
        ],
    ),
    (
        0x0a00,
        &[0x4100_0083, 0x1200_1fc4, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x0b00,
        &[
            0x58c1_828a,
            0x1801_c30b,
            0x1801_c30c,
            0x1cff_c183,
            0x1801_c30d,
            0x217f_fd83,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0616,
            0x3001_8000,
        ],
    ),
    (
        0x0c00,
        &[0x4100_0083, 0x1200_1fc5, 0x01a0_0414, 0x3200_1e80],
    ),
    (
        0x0d00,
        &[
            0x5881_828a,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x4020_0000,
            0x1cff_c183,
            0x217f_fd03,
            0x01a0_0415,
            0x0805_0a96,
            0x2090_0816,
            0x3001_c000,
        ],
    ),
    (0x0e00, &[0x0000_2000]),
];

/// The loops the probe times, and the iterations of each.
const CASES: usize = 5;
const ITERATIONS: u64 = 65536;

/// Where the program stores each loop's decrementer delta: the preferred
/// slot of the quadword at `SLOTS_LS + 16 k`.
const SLOTS_LS: usize = 0x8000;

const CONSOLE: &str = "../../tests/micro/spu_issue_retire/ps3/cech20-cex-493/observation.json";

/// The console's decrementer delta for each loop: the payload's slots
/// from offset 32.
fn console_deltas() -> Vec<u32> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 64);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    assert_eq!(word(16), 0x2000_0002, "the SPU stopped with 0x2000");
    (0..CASES).map(|k| word(32 + 4 * k)).collect()
}

/// Converts a decrementer delta over a loop to cycles an iteration,
/// rounded to the nearest cycle.
fn cycles_per_iteration(delta: u32) -> u64 {
    (u64::from(delta) * SPU_CYCLES_PER_TB_TICK + ITERATIONS / 2) / ITERATIONS
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

#[test]
fn an_instruction_that_completes_with_an_earlier_one_on_its_pipeline_takes_the_consoles_cycles() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(10_000), 100_000);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (base, words) in BLOCKS {
            for (i, word) in words.iter().enumerate() {
                let at = base as usize + i * 4;
                state.ls[at..at + 4].copy_from_slice(&word.to_be_bytes());
            }
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
    let cellgov: Vec<u64> = (0..CASES)
        .map(|k| {
            let at = SLOTS_LS + 16 * k;
            cycles_per_iteration(u32::from_be_bytes(
                ls[at..at + 4].try_into().expect("4 bytes"),
            ))
        })
        .collect();
    let console: Vec<u64> = console_deltas()
        .into_iter()
        .map(cycles_per_iteration)
        .collect();
    assert_eq!(cellgov, console, "cycles an iteration, loop by loop");
}
