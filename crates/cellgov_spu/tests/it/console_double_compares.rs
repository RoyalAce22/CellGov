//! The optional double-precision compares on a retail console: the
//! `spu_double_compares` probe's committed capture. The CBE does not
//! provide dfceq, dfcmeq, dfcgt, dfcmgt or dftsv. Each one, like an
//! unassigned word, stops a raw SPU as an invalid instruction before the
//! store that follows it runs.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_STATUS_I;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_double_compares/ps3/cech20-cex-493/observation.json";

/// `tests/micro/spu_double_compares/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image): entry k at 0x40k is
/// `lqr $4`, `lqr $5`, the word, `stqr $3` to result slot k, and
/// `stop 0x2000 + k`.
const ENTRIES: [[u32; 5]; 6] = [
    [
        0x3380_4004,
        0x3380_4185,
        0x7861_4203,
        0x2380_5e83,
        0x0000_2000,
    ],
    [
        0x3380_3804,
        0x3380_3985,
        0x7961_4203,
        0x2380_5883,
        0x0000_2001,
    ],
    [
        0x3380_3004,
        0x3380_3185,
        0x5861_4203,
        0x2380_5283,
        0x0000_2002,
    ],
    [
        0x3380_2804,
        0x3380_2985,
        0x5961_4203,
        0x2380_4c83,
        0x0000_2003,
    ],
    [
        0x3380_2004,
        0x3380_2185,
        0x77ff_c203,
        0x2380_4683,
        0x0000_2004,
    ],
    [
        0x3380_1804,
        0x3380_1985,
        0x9000_0000,
        0x2380_4083,
        0x0000_2005,
    ],
];

/// The operands at 0x200 and 0x210, and the result slots at 0x300.
const OPERAND_A: [u32; 4] = [0x3FF0_0000, 0, 0x7FF8_0000, 0];
const OPERAND_B: [u32; 4] = [0x3FF0_0000, 0, 0x4000_0000, 0];
const RESULTS_AT: usize = 0x300;
const UNSTORED: u32 = 0xEEEE_EEEE;

/// One entry's record: (SPU_Status, SPU_NPC, class 0 status, result slot).
struct Record {
    status: u32,
    npc: u32,
    class0: u64,
    result: [u32; 4],
}

fn console() -> Vec<Record> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    (0..6)
        .map(|k| {
            let at = 48 + k * 48;
            assert_eq!(
                word(at + 12),
                0,
                "entry {k}: the class 0 status read succeeded"
            );
            Record {
                status: word(at),
                npc: word(at + 4),
                class0: u64::from(word(at + 16)) << 32 | u64::from(word(at + 20)),
                result: std::array::from_fn(|i| word(at + 32 + i * 4)),
            }
        })
        .collect()
}

fn put(spu: &mut SpuExecutionUnit, at: usize, words: &[u32]) {
    for (i, word) in words.iter().enumerate() {
        spu.state_mut().ls[at + i * 4..at + i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
}

/// Runs entry `k` on a fresh SPU: (SPU_Status, SPU_NPC, result slot k).
fn run(k: usize) -> (u32, u32, [u32; 4]) {
    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    for (e, entry) in ENTRIES.iter().enumerate() {
        put(&mut spu, e * 0x40, entry);
        put(&mut spu, RESULTS_AT + e * 16, &[UNSTORED; 4]);
    }
    put(&mut spu, 0x200, &OPERAND_A);
    put(&mut spu, 0x210, &OPERAND_B);
    spu.state_mut().pc = (k * 0x40) as u32;
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(Budget::new(100), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Finished, "entry {k}");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let ls = &spu.state().ls;
    let slot = RESULTS_AT + k * 16;
    let result = std::array::from_fn(|i| {
        u32::from_be_bytes(ls[slot + i * 4..slot + i * 4 + 4].try_into().expect("4"))
    });
    (stop.status, stop.npc, result)
}

#[test]
fn each_optional_compare_stops_the_spu_as_an_invalid_instruction_before_its_store() {
    let console = console();
    for (k, record) in console.iter().enumerate() {
        let (status, _, result) = run(k);
        assert_eq!(
            record.status, SPU_STATUS_I,
            "entry {k}: the console stopped on I"
        );
        assert_eq!(status, record.status, "entry {k}: SPU_Status");
        assert_eq!(
            record.result, [UNSTORED; 4],
            "entry {k}: the console stored nothing"
        );
        assert_eq!(result, record.result, "entry {k}: the result slot");
    }
}

/// The console resumes past the invalid word, by a distance that depends
/// on the word, and latches a class 0 status; CellGov resumes at the word
/// itself. The capture gives no rule for the distance, so this test pins
/// the difference.
#[test]
fn the_resume_address_after_an_invalid_word_differs_from_the_console() {
    let console = console();
    for (k, record) in console.iter().enumerate() {
        let word_at = (k * 0x40 + 8) as u32;
        let (_, npc, _) = run(k);
        assert_eq!(npc, word_at, "entry {k}: CellGov resumes at the word");
        assert!(
            record.npc > word_at,
            "entry {k}: the console resumes past it"
        );
        assert_ne!(
            record.class0, 0,
            "entry {k}: the console latched a class 0 status"
        );
    }
}
