//! `addx`, `sfx`, `cgx` and `bgx` with the reserved bits of the RT carry
//! input set, held to the `spu_carry_reserved_bits` probe's committed
//! capture. Each instruction runs on four input sets. For `addx`, `sfx`
//! and `cgx` the console takes the carry from bit 31 of each RT word
//! alone. No `bgx` input set has RA equal to RB, so the borrow never
//! changes its result and the capture leaves its reserved bits open.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::SPU_STATUS_P;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str =
    "../../tests/micro/spu_carry_reserved_bits/ps3/cech20-cex-493/observation.json";

/// `tests/micro/spu_carry_reserved_bits/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), from address 0: case
/// `4i + k` loads RT, RA and RB from input set `k`, runs instruction `i`
/// as `insn $3, $4, $5` and stores `$3`.
const PROGRAM: [u32; 81] = [
    0x3382_0003,
    0x3382_0184,
    0x3382_0305,
    0x6801_4203,
    0x2382_7e03,
    0x3382_0383,
    0x3382_0504,
    0x3382_0685,
    0x6801_4203,
    0x2382_7d83,
    0x3382_0703,
    0x3382_0884,
    0x3382_0a05,
    0x6801_4203,
    0x2382_7d03,
    0x3382_0a83,
    0x3382_0c04,
    0x3382_0d85,
    0x6801_4203,
    0x2382_7c83,
    0x3381_f603,
    0x3381_f784,
    0x3381_f905,
    0x6821_4203,
    0x2382_7c03,
    0x3381_f983,
    0x3381_fb04,
    0x3381_fc85,
    0x6821_4203,
    0x2382_7b83,
    0x3381_fd03,
    0x3381_fe84,
    0x3382_0005,
    0x6821_4203,
    0x2382_7b03,
    0x3382_0083,
    0x3382_0204,
    0x3382_0385,
    0x6821_4203,
    0x2382_7a83,
    0x3381_ec03,
    0x3381_ed84,
    0x3381_ef05,
    0x6841_4203,
    0x2382_7a03,
    0x3381_ef83,
    0x3381_f104,
    0x3381_f285,
    0x6841_4203,
    0x2382_7983,
    0x3381_f303,
    0x3381_f484,
    0x3381_f605,
    0x6841_4203,
    0x2382_7903,
    0x3381_f683,
    0x3381_f804,
    0x3381_f985,
    0x6841_4203,
    0x2382_7883,
    0x3381_e203,
    0x3381_e384,
    0x3381_e505,
    0x6861_4203,
    0x2382_7803,
    0x3381_e583,
    0x3381_e704,
    0x3381_e885,
    0x6861_4203,
    0x2382_7783,
    0x3381_e903,
    0x3381_ea84,
    0x3381_ec05,
    0x6861_4203,
    0x2382_7703,
    0x3381_ec83,
    0x3381_ee04,
    0x3381_ef85,
    0x6861_4203,
    0x2382_7683,
    0x0000_2000,
];

/// The input sets at 0x1000, as (RT, RA, RB) quadwords. The RT words set
/// reserved bits with bit 31 clear and set.
const INPUTS: [[[u32; 4]; 3]; 4] = [
    [
        [0xFFFF_FFFE, 0xFFFF_FFFF, 0x0000_0002, 0x8000_0001],
        [5; 4],
        [3; 4],
    ],
    [
        [0xFFFF_FFFE, 0xFFFF_FFFF, 0x0000_0002, 0x8000_0001],
        [0xFFFF_FFFF; 4],
        [1; 4],
    ],
    [
        [0xFFFF_FFFE, 0xFFFF_FFFF, 0x0000_0002, 0x8000_0001],
        [0; 4],
        [0xFFFF_FFFF; 4],
    ],
    [
        [0x7FFF_FFFE, 0x7FFF_FFFF, 0xAAAA_AAAA, 0x5555_5555],
        [3; 4],
        [5; 4],
    ],
];
const INPUTS_AT: usize = 0x1000;
const OUT_AT: usize = 0x1400;
const CASES: usize = 16;
const UNSTORED: u32 = 0xEEEE_EEEE;

/// (SPU_Status, SPU_NPC, class 0 status, result words) from the capture.
fn console() -> (u32, u32, u64, Vec<u32>) {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 288);
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
fn the_carry_instructions_end_each_reserved_bit_case_as_on_the_console() {
    let (status, npc, class0, console_words) = console();
    assert_eq!(
        status,
        0x2000 << 16 | SPU_STATUS_P,
        "the console stopped at 0x2000"
    );
    assert_eq!(class0, 0, "the console latched no SPU error");
    // addx on input set 0: the RT words 0xFFFFFFFE and 0x00000002 carry
    // nothing, 0xFFFFFFFF and 0x80000001 carry one.
    assert_eq!(console_words[..4], [8, 9, 8, 9], "addx, input set 0");

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
    assert_eq!(
        cellgov, console_words,
        "case 4i + k: instruction i on input set k"
    );
}
