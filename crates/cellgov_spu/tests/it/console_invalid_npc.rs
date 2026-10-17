//! Where an SPU resumes after an invalid word, held to the
//! `spu_invalid_npc` probe's committed capture. The probe runs fifteen
//! invalid words, each in five arrangements, twice per case on a raw SPU.
//!
//! The console stops each one with I set and latches the class 0 SPU
//! error status. A word that issues alone resumes 4 bytes past it: the
//! word before it ran and the word after it did not. A word that issues
//! together with the next word resumes 8 bytes past it, and the next word
//! does not run. CellGov does not model issue pairs and resumes 4 bytes
//! past every invalid word.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{SPU_NPC_OFFSET, SPU_RUN_CNTL_OFFSET, SPU_STATUS_OFFSET};
use cellgov_ps3_abi::hw::spu::{SPU_STATUS_I, SPU_STATUS_P};
use cellgov_ps3_abi::lv2::spu::raw_spu;
use cellgov_ps3_abi::lv2::syscall::RAW_SPU_GET_INT_STAT;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

use crate::raw_spu_window::{
    create_ops, poll_stopped, read, register, run_ops, runtime, Op, OUT_PTR, WINDOW,
};

const CONSOLE: &str = "../../tests/micro/spu_invalid_npc/ps3/cech20-cex-493/observation.json";

/// The probe's fifteen words, in case order.
const WORDS: [u32; 15] = [
    0x7861_4203, // dfceq
    0x7961_4203, // dfcmeq
    0x5861_4203, // dfcgt
    0x5961_4203, // dfcmgt
    0x77FF_C203, // dftsv
    0x9000_0000,
    0xA000_0000,
    0x0080_0000,
    0x0100_0000,
    0x2820_0000,
    0x3B40_0000,
    0x3E20_0000,
    0x5000_0000,
    0x6000_0000,
    0x7000_0000,
];

/// The word the console runs as a no-op, though no opcode row names it.
const RUNS_ON_THE_CONSOLE: u32 = 0x0100_0000;

const ARRANGEMENTS: usize = 5;
const CASE_STRIDE: u32 = 0x40;
const MARKS: u32 = 0x2000;
const DUMP_SLOT: u32 = 0x1900;
/// The word `$8` holds in each slot, and so the word a `stqr $8` stores.
const MARKED: u32 = 0x77;

/// One case's record: what each of the two runs left, and the neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    status: u32,
    npc: u32,
    class0: u64,
    mark: u32,
    r6: u32,
    status2: u32,
    npc2: u32,
}

fn console() -> Vec<Record> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    (0..WORDS.len() * ARRANGEMENTS)
        .map(|k| {
            let at = 16 + k * 32;
            let record = Record {
                status: word(at),
                npc: word(at + 4),
                class0: u64::from(word(at + 8)) << 32 | u64::from(word(at + 12)),
                mark: word(at + 16),
                r6: word(at + 20),
                status2: word(at + 24),
                npc2: word(at + 28),
            };
            assert_eq!(
                (record.status2, record.npc2),
                (record.status, record.npc),
                "case {k}: the second run agrees"
            );
            record
        })
        .collect()
}

/// An RI16 instruction word: opcode, 16-bit immediate, RT.
fn ri16(opcode: u32, imm: i32, rt: u32) -> u32 {
    opcode << 23 | (imm as u32 & 0xFFFF) << 7 | rt
}

/// `stqr`/`lqr` at `pc` naming `target`.
fn relative(opcode: u32, rt: u32, pc: u32, target: u32) -> u32 {
    ri16(opcode, (target as i32 - pc as i32) >> 2, rt)
}

const STQR: u32 = 0b0_0100_0111;
const LQR: u32 = 0b0_0110_0111;
const IL: u32 = 0b0_1000_0001;
/// `ai $6, $6, 1`.
const AI_R6_1: u32 = 0x1C00_4306;

/// Case `k`'s five words, as `spu/main.S` places them at `k * 0x40`.
fn case_words(k: usize) -> [u32; 5] {
    let base = k as u32 * CASE_STRIDE;
    let word = WORDS[k / ARRANGEMENTS];
    let mark = MARKS + k as u32 * 16;
    let stqr_at = |pc: u32| relative(STQR, 8, pc, mark);
    let il6 = ri16(IL, 0, 6);
    let il8 = ri16(IL, MARKED as i32, 8);
    let stop = 0x2000 + k as u32;
    match k % ARRANGEMENTS {
        0 => [il6, il8, word, stqr_at(base + 0xC), stop],
        1 => [il6, il8, word, AI_R6_1, stop],
        2 => [il6, il8, AI_R6_1, word, stop],
        3 => [il6, il8, stqr_at(base + 0x8), word, stop],
        _ => [
            relative(LQR, 4, base, DUMP_SLOT),
            relative(LQR, 5, base + 4, DUMP_SLOT),
            word,
            stqr_at(base + 0xC),
            stop,
        ],
    }
}

/// The address of case `k`'s invalid word.
fn word_at(k: usize) -> u32 {
    let base = k as u32 * CASE_STRIDE;
    match k % ARRANGEMENTS {
        2 | 3 => base + 0xC,
        _ => base + 0x8,
    }
}

/// Runs case `k` on a fresh SPU: (SPU_Status, SPU_NPC, mark word, $6).
fn run(k: usize) -> (u32, u32, u32, u32) {
    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    let base = k * CASE_STRIDE as usize;
    for (i, w) in case_words(k).iter().enumerate() {
        spu.state_mut().ls[base + i * 4..base + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    // Arrangement 4 sets no `$8`; on the console it holds 0x77 from the case
    // before (case 44's stqr stored it), so a stqr that ran would mark.
    spu.state_mut().set_reg_word_splat(8, MARKED);
    spu.state_mut().pc = base as u32;
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(Budget::new(100), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Finished, "case {k}");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let state = spu.state();
    let at = (MARKS as usize) + k * 16;
    let mark = u32::from_be_bytes(state.ls[at..at + 4].try_into().expect("4"));
    let r6 = u32::from_be_bytes(state.regs.as_array()[6][..4].try_into().expect("4"));
    (stop.status, stop.npc, mark, r6)
}

/// The cases the console stopped on an invalid word; the word it runs is
/// left out.
fn invalid_cases() -> impl Iterator<Item = usize> {
    (0..WORDS.len() * ARRANGEMENTS).filter(|&k| WORDS[k / ARRANGEMENTS] != RUNS_ON_THE_CONSOLE)
}

#[test]
fn a_word_that_issues_alone_resumes_past_it_with_the_consoles_neighbours() {
    let console = console();
    let mut held = 0;
    for k in invalid_cases() {
        let record = console[k];
        if record.npc != word_at(k) + 4 {
            continue;
        }
        assert_eq!(
            record.status, SPU_STATUS_I,
            "case {k}: the console stopped on I"
        );
        assert_eq!(
            record.class0,
            raw_spu::INT_STAT_SPU_ERROR,
            "case {k}: the console latched SE"
        );
        let (status, npc, mark, r6) = run(k);
        assert_eq!(status, record.status, "case {k}: SPU_Status");
        assert_eq!(npc, record.npc, "case {k}: SPU_NPC");
        assert_eq!(mark, record.mark, "case {k}: the stqr mark");
        assert_eq!(r6, record.r6, "case {k}: the ai count");
        held += 1;
    }
    // Every case of arrangements 0 to 3, and arrangement 4 for the five
    // words the console never pairs with the store after them.
    assert_eq!(held, 14 * 4 + 5);
}

/// The console issues an even-pipe word at an even address together with
/// a ready odd-pipe word after it. It resumes past both and runs neither.
/// CellGov models no issue pairs: it resumes past the invalid word alone.
#[test]
fn a_word_issued_with_the_next_one_resumes_past_both_on_the_console_only() {
    let console = console();
    let paired: Vec<usize> = invalid_cases()
        .filter(|&k| console[k].npc == word_at(k) + 8)
        .collect();
    assert_eq!(paired.len(), 9, "{paired:?}");
    for k in paired {
        assert_eq!(k % ARRANGEMENTS, 4, "case {k}: only after the loads");
        let record = console[k];
        let (status, npc, mark, _) = run(k);
        assert_eq!(status, record.status, "case {k}: SPU_Status");
        assert_eq!(mark, record.mark, "case {k}: the paired stqr did not run");
        assert_eq!(
            npc,
            word_at(k) + 4,
            "case {k}: CellGov resumes past the word"
        );
    }
}

/// `0x01000000` names no opcode row. The console runs it as a no-op;
/// CellGov stops on it.
#[test]
fn the_console_runs_one_unassigned_word_that_cellgov_stops_on() {
    let console = console();
    let w = WORDS
        .iter()
        .position(|&w| w == RUNS_ON_THE_CONSOLE)
        .expect("the word");
    let cases = w * ARRANGEMENTS..(w + 1) * ARRANGEMENTS;
    for (k, &record) in cases.clone().zip(&console[cases]) {
        assert_eq!(
            record.status,
            (0x2000 + k as u32) << 16 | SPU_STATUS_P,
            "case {k}: the console reached the closing stop"
        );
        assert_eq!(record.class0, 0, "case {k}: no SPU error");
        let (status, npc, _, _) = run(k);
        assert_eq!(status, SPU_STATUS_I, "case {k}: CellGov stops on it");
        assert_eq!(npc, word_at(k) + 4, "case {k}");
    }
}

/// A raw SPU that stops on an invalid word latches class 0 SE, as the
/// console did with the class 0 mask at 0, and shows the stop in its
/// window. The program is the probe's case for `0x90000000` after `il`.
#[test]
fn a_raw_spu_stopped_on_an_invalid_word_latches_class_0_se() {
    let k = 5 * ARRANGEMENTS;
    let record = console()[k];
    let base = k as u32 * CASE_STRIDE;
    let mut ops = create_ops();
    ops.extend(
        case_words(k)
            .iter()
            .enumerate()
            .map(|(i, &w)| Op::Store(WINDOW + u64::from(base) + 4 * i as u64, w)),
    );
    ops.extend([
        Op::Store(register(SPU_NPC_OFFSET), base),
        Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
        poll_stopped(),
        Op::Call([
            RAW_SPU_GET_INT_STAT,
            0,
            u64::from(raw_spu::CLASS_ERROR),
            OUT_PTR,
        ]),
    ]);
    let n = ops.len();
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, n);
    assert_eq!(read(&rt, register(SPU_STATUS_OFFSET)), record.status);
    assert_eq!(read(&rt, register(SPU_NPC_OFFSET)), record.npc);
    let class0 = u64::from(read(&rt, OUT_PTR)) << 32 | u64::from(read(&rt, OUT_PTR + 4));
    assert_eq!(class0, record.class0);
}
