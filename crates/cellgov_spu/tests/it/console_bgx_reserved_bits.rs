//! `bgx` with RA equal to RB and the reserved bits of the RT borrow input
//! set, held to the `spu_bgx_reserved_bits` probe's committed capture.
//! With RA equal to RB the borrow decides the result, so each word shows
//! which RT bits the console reads as the borrow.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_STATUS_P;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_bgx_reserved_bits/ps3/cech20-cex-493/observation.json";

/// `tests/micro/spu_bgx_reserved_bits/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), from address 0: case
/// `k` loads RT, RA and RB from input set `k`, runs `bgx $3, $4, $5` and
/// stores `$3`.
const PROGRAM: [u32; 21] = [
    0x3382_0003,
    0x3382_0184,
    0x3382_0305,
    0x6861_4203,
    0x2382_7e03,
    0x3382_0383,
    0x3382_0504,
    0x3382_0685,
    0x6861_4203,
    0x2382_7d83,
    0x3382_0703,
    0x3382_0884,
    0x3382_0a05,
    0x6861_4203,
    0x2382_7d03,
    0x3382_0a83,
    0x3382_0c04,
    0x3382_0d85,
    0x6861_4203,
    0x2382_7c83,
    0x0000_2000,
];

/// The input sets at 0x1000, as (RT, RA, RB) quadwords, RA equal to RB.
const INPUTS: [[[u32; 4]; 3]; 4] = [
    [
        [0xFFFF_FFFE, 0xFFFF_FFFF, 0x0000_0002, 0x8000_0001],
        [5; 4],
        [5; 4],
    ],
    [
        [0xFFFF_FFFE, 0xFFFF_FFFF, 0x0000_0002, 0x8000_0001],
        [0; 4],
        [0; 4],
    ],
    [
        [0x7FFF_FFFE, 0x7FFF_FFFF, 0xAAAA_AAAA, 0x5555_5555],
        [0xFFFF_FFFF; 4],
        [0xFFFF_FFFF; 4],
    ],
    [
        [0x0000_0000, 0x0000_0001, 0xFFFF_FFFE, 0x0000_0003],
        [0x8000_0000; 4],
        [0x8000_0000; 4],
    ],
];
const INPUTS_AT: usize = 0x1000;
const OUT_AT: usize = 0x1400;
const CASES: usize = 4;
const UNSTORED: u32 = 0xEEEE_EEEE;

/// (SPU_Status, SPU_NPC, class 0 status, result words) from the capture.
fn console() -> (u32, u32, u64, Vec<u32>) {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 96);
    let word = |at: usize| u32::from_be_bytes(p[4 * at..4 * at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    (
        word(4),
        word(5),
        u64::from(word(6)) << 32 | u64::from(word(7)),
        (0..CASES * 4).map(|k| word(8 + k)).collect(),
    )
}

fn put(ls: &mut [u8], at: usize, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        ls[at + i * 4..at + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
}

#[test]
fn bgx_with_equal_operands_ends_each_reserved_bit_case_as_on_the_console() {
    let (status, npc, class0, console_words) = console();
    assert_eq!(
        status,
        0x2000 << 16 | SPU_STATUS_P,
        "the console stopped at 0x2000"
    );
    assert_eq!(class0, 0, "the console latched no SPU error");

    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    let ls = &mut spu.state_mut().ls;
    put(ls, 0, &PROGRAM);
    for (k, set) in INPUTS.iter().enumerate() {
        for (j, quad) in set.iter().enumerate() {
            put(ls, INPUTS_AT + (3 * k + j) * 16, quad);
        }
    }
    put(ls, OUT_AT, &[UNSTORED; CASES * 4]);
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(
        Budget::new(1000),
        &ExecutionContext::new(&mem),
        &mut effects,
    );
    assert_eq!(result.yield_reason, YieldReason::Finished);
    let stop = spu.stop_registers().expect("a stopped SPU");
    assert_eq!((stop.status, stop.npc), (status, npc));
    let ls = &spu.state().ls;
    let cellgov: Vec<u32> = (0..CASES * 4)
        .map(|k| {
            let at = OUT_AT + 4 * k;
            u32::from_be_bytes(ls[at..at + 4].try_into().expect("4 bytes"))
        })
        .collect();
    assert_eq!(cellgov, console_words, "case k: bgx on input set k");
}
