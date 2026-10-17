//! A channel instruction in the wrong direction, held to the
//! `spu_invalid_channel` probe's committed capture: a read of the
//! write-only SPU_WrOutMbox and a write to the read-only SPU_RdInMbox, in
//! the even and the odd slot, twice per case on a raw SPU.
//!
//! The console does not stop on either. The read returns zero, the write
//! has no effect, the SPU runs on to its closing stop, and the SPU
//! latches no class 0 status.

use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    SPU_NPC_OFFSET, SPU_RUN_CNTL_OFFSET, SPU_STATUS_OFFSET, SPU_STATUS_P,
};
use cellgov_ps3_abi::lv2::spu::raw_spu;
use cellgov_ps3_abi::lv2::syscall::RAW_SPU_GET_INT_STAT;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

use crate::raw_spu_window::{
    create_ops, poll_stopped, read, register, run_ops, runtime, Op, OUT_PTR, WINDOW,
};

const CONSOLE: &str = "../../tests/micro/spu_invalid_channel/ps3/cech20-cex-493/observation.json";

const CASES: usize = 4;
const CASE_STRIDE: u32 = 0x40;
const MARKS: u32 = 0x2000;
/// The word each mark holds before any store.
const UNSTORED: u32 = 0xEEEE_EEEE;
/// What `il $3` loads before the access.
const R3_BEFORE: u32 = 0x55;
/// What `il $8` loads; cases 2 and 3 store it before the access.
const R8: u32 = 0x77;

/// `rdch $3, $ch28` (SPU_WrOutMbox) and `wrch $ch29, $3` (SPU_RdInMbox).
const RDCH_WR_OUT_MBOX: u32 = 0x01A0_0E03;
const WRCH_RD_IN_MBOX: u32 = 0x21A0_0E83;

/// One case's record: what the first run left, and the second run's
/// stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    status: u32,
    npc: u32,
    class0: u64,
    class2: u64,
    mark: u32,
    status2: u32,
    npc2: u32,
}

fn console() -> Vec<Record> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    let dword = |at: usize| u64::from(word(at)) << 32 | u64::from(word(at + 4));
    assert_eq!((word(0), word(4)), (0, 0), "the probe ran every step");
    (0..CASES)
        .map(|k| {
            let at = 16 + k * 40;
            let record = Record {
                status: word(at),
                npc: word(at + 4),
                class0: dword(at + 8),
                class2: dword(at + 16),
                mark: word(at + 24),
                status2: word(at + 32),
                npc2: word(at + 36),
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

/// `stqr rt` at `pc` naming `target`.
fn stqr(rt: u32, pc: u32, target: u32) -> u32 {
    0b0_0100_0111 << 23 | (((target as i32 - pc as i32) >> 2) as u32 & 0xFFFF) << 7 | rt
}

/// `il rt, imm`.
fn il(rt: u32, imm: u32) -> u32 {
    0b0_1000_0001 << 23 | (imm & 0xFFFF) << 7 | rt
}

/// Case `k`'s five words, as `spu/main.S` places them at `k * 0x40`.
fn case_words(k: usize) -> [u32; 5] {
    let base = k as u32 * CASE_STRIDE;
    let mark = MARKS + k as u32 * 16;
    let access = if k.is_multiple_of(2) {
        RDCH_WR_OUT_MBOX
    } else {
        WRCH_RD_IN_MBOX
    };
    let head = [il(3, R3_BEFORE), il(8, R8)];
    let stop = 0x2000 + k as u32;
    if k < 2 {
        [head[0], head[1], access, stqr(3, base + 0xC, mark), stop]
    } else {
        [head[0], head[1], stqr(8, base + 0x8, mark), access, stop]
    }
}

/// Runs case `k` on a fresh SPU: (SPU_Status, SPU_NPC, mark word).
fn run(k: usize) -> (u32, u32, u32) {
    let mut spu = SpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    let base = k * CASE_STRIDE as usize;
    for (i, w) in case_words(k).iter().enumerate() {
        spu.state_mut().ls[base + i * 4..base + i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    let at = MARKS as usize + k * 16;
    for i in 0..4 {
        spu.state_mut().ls[at + i * 4..at + i * 4 + 4].copy_from_slice(&UNSTORED.to_be_bytes());
    }
    spu.state_mut().pc = base as u32;
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    let result = spu.run_until_yield(Budget::new(100), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Finished, "case {k}");
    let stop = spu.stop_registers().expect("a stopped SPU");
    let mark = u32::from_be_bytes(spu.state().ls[at..at + 4].try_into().expect("4"));
    (stop.status, stop.npc, mark)
}

/// Each case runs on to its closing stop: the read returns zero, the
/// write has no effect, and the store after the access runs.
#[test]
fn a_wrong_direction_channel_instruction_runs_on_as_on_the_console() {
    for (k, record) in console().into_iter().enumerate() {
        let base = k as u32 * CASE_STRIDE;
        assert_eq!(
            record.status,
            (0x2000 + k as u32) << 16 | SPU_STATUS_P,
            "case {k}: the console's closing stop"
        );
        assert_eq!(record.npc, base + 0x14, "case {k}: past the closing stop");
        let console_mark = match k {
            0 => 0,
            1 => R3_BEFORE,
            _ => R8,
        };
        assert_eq!(record.mark, console_mark, "case {k}: the console's mark");
        assert_eq!(run(k), (record.status, record.npc, record.mark), "case {k}");
    }
}

/// A raw SPU that reads a write channel or writes a read channel latches
/// no class 0 status, and class 2 holds B and the stop-and-signal S, as
/// on the console.
#[test]
fn a_raw_spu_wrong_direction_access_latches_no_spu_error() {
    for (k, record) in console().into_iter().enumerate() {
        assert_eq!(
            record.class0, 0,
            "case {k}: the console latched no SPU error"
        );
        assert_eq!(
            record.class2,
            raw_spu::INT_STAT_MAILBOX_THRESHOLD | raw_spu::INT_STAT_STOP_AND_SIGNAL,
            "case {k}: the console's B and S"
        );
        let base = u64::from(k as u32 * CASE_STRIDE);
        let mut ops = create_ops();
        ops.extend(
            case_words(k)
                .iter()
                .enumerate()
                .map(|(i, &w)| Op::Store(WINDOW + base + 4 * i as u64, w)),
        );
        ops.extend([
            Op::Store(register(SPU_NPC_OFFSET), base as u32),
            Op::Store(register(SPU_RUN_CNTL_OFFSET), 1),
            poll_stopped(),
            Op::Call([
                RAW_SPU_GET_INT_STAT,
                0,
                u64::from(raw_spu::CLASS_ERROR),
                OUT_PTR,
            ]),
            Op::Call([
                RAW_SPU_GET_INT_STAT,
                0,
                u64::from(raw_spu::CLASS_APPLICATION),
                OUT_PTR + 8,
            ]),
        ]);
        let n = ops.len();
        let (mut rt, ppu) = runtime(ops);
        run_ops(&mut rt, ppu, n);
        let dword = |at: u64| u64::from(read(&rt, at)) << 32 | u64::from(read(&rt, at + 4));
        assert_eq!(
            read(&rt, register(SPU_STATUS_OFFSET)),
            record.status,
            "case {k}"
        );
        assert_eq!(read(&rt, register(SPU_NPC_OFFSET)), record.npc, "case {k}");
        assert_eq!(dword(OUT_PTR), record.class0, "case {k}: class 0");
        assert_eq!(dword(OUT_PTR + 8), record.class2, "case {k}: class 2");
    }
}
