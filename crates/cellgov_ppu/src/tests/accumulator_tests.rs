//! The state-hash accumulator stays equal to a full rehash through every
//! setter, a clone, a batch rollback and a runtime snapshot restore.

use super::*;
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;
use cellgov_trace::{TraceReader, TraceRecord};

fn assert_current(s: &PpuState, what: &str) {
    assert!(s.hash_is_current(), "{what}: accumulator out of date");
    assert_eq!(s.state_hash(), s.state_hash_from_scratch(), "{what}");
}

#[test]
fn a_new_state_starts_current() {
    assert_current(&PpuState::new(), "new");
    assert_current(&PpuState::default(), "default");
}

#[test]
fn every_hashed_setter_keeps_the_accumulator_current() {
    let mut s = PpuState::new();
    s.set_gpr(0, u64::MAX);
    assert_current(&s, "set_gpr r0");
    s.set_gpr(31, 0x8000_0000_0000_0000);
    assert_current(&s, "set_gpr r31");
    s.set_gpr_all([0x0123_4567_89ab_cdef; GPR_COUNT]);
    assert_current(&s, "set_gpr_all");
    s.set_lr(0xdead_beef);
    assert_current(&s, "set_lr");
    s.set_ctr(u64::MAX);
    assert_current(&s, "set_ctr");
    s.set_xer(1 << 63);
    assert_current(&s, "set_xer");
    s.set_cr(0xffff_ffff);
    assert_current(&s, "set_cr");
    s.set_cr_field(3, 0b0101);
    assert_current(&s, "set_cr_field");
    s.set_cr_bit(31, false);
    assert_current(&s, "set_cr_bit");
    s.set_xer_ca(true);
    assert_current(&s, "set_xer_ca");
    s.set_xer_ov(true);
    assert_current(&s, "set_xer_ov");
    s.set_cr0_from_result(0);
    assert_current(&s, "set_cr0_from_result");
    s.set_reservation(Some(ReservedLine::containing(0x3000_1080)));
    assert_current(&s, "set_reservation Some");
    s.set_reservation(Some(ReservedLine::containing(0)));
    assert_current(&s, "set_reservation line 0");
    s.set_reservation(None);
    assert_current(&s, "set_reservation None");
}

#[test]
fn writing_a_lane_back_restores_its_hash() {
    let mut s = PpuState::new();
    s.set_gpr(7, 42);
    let h = s.state_hash();
    s.set_gpr(7, 42);
    assert_eq!(s.state_hash(), h, "a same-value write changes nothing");
    s.set_gpr(7, 43);
    assert_ne!(s.state_hash(), h);
    s.set_gpr(7, 42);
    assert_eq!(s.state_hash(), h);
}

#[test]
fn unhashed_writes_leave_the_accumulator_alone() {
    let mut s = PpuState::new();
    s.set_gpr(3, 9);
    let h = s.state_hash();
    s.pc = 0x1000;
    s.tb = 77;
    s.vrsave = 1;
    s.set_fpr(1, 2);
    s.set_vr(1, 3);
    s.set_fpr_all([4; 32]);
    s.set_vr_all([5; 32]);
    assert_eq!(s.state_hash(), h);
    assert_current(&s, "unhashed writes");
}

#[test]
fn a_clone_carries_the_accumulator() {
    let mut s = PpuState::new();
    s.set_gpr(5, 0xabc);
    s.set_cr(0x2400_0042);
    let c = s.clone();
    assert_current(&c, "clone");
    assert_eq!(c.state_hash(), s.state_hash());
}

/// SplitMix64 over `state`.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[test]
fn a_long_random_sequence_of_writes_stays_current() {
    let mut rng = 0x5eed;
    let mut s = PpuState::new();
    for i in 0..20_000 {
        let v = next(&mut rng);
        match v % 11 {
            0..=3 => s.set_gpr((v >> 8) as usize % GPR_COUNT, next(&mut rng)),
            4 => s.set_lr(next(&mut rng)),
            5 => s.set_ctr(next(&mut rng)),
            6 => s.set_xer(next(&mut rng)),
            7 => s.set_cr(next(&mut rng) as u32),
            8 => s.set_cr_field((v >> 8) as u8 % 8, (v >> 16) as u8),
            9 => s.set_xer_ca(v & 0x100 != 0),
            _ => s.set_reservation(
                (v & 0x100 != 0)
                    .then(|| ReservedLine::containing(next(&mut rng) % 0x400_0000_0000)),
            ),
        }
        if i % 97 == 0 {
            assert_current(&s, "random sequence");
        }
    }
    assert_current(&s, "random sequence end");
}

fn place_insn(mem: &mut GuestMemory, offset: usize, raw: u32) {
    let range = ByteRange::new(GuestAddr::new(offset as u64), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
}

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

#[test]
fn a_rolled_back_batch_leaves_the_accumulator_current() {
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0, addi_inc(3));
    place_insn(&mut mem, 4, addi_inc(4));
    // The all-zero word at 8 fails to decode, so the batch rewinds.
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(3, 10);
    let entry_hash = unit.state().state_hash();
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.state().gpr[3], 10, "the batch rewound");
    assert_current(unit.state(), "after rollback");
    assert_eq!(unit.state().state_hash(), entry_hash);
}

fn per_step_hashes(bytes: &[u8]) -> Vec<(u64, u64)> {
    TraceReader::new(bytes)
        .map(Result::unwrap)
        .filter_map(|r| match r {
            TraceRecord::PpuStateHash { pc, hash, .. } => Some((pc, hash.raw())),
            _ => None,
        })
        .collect()
}

fn drive(rt: &mut cellgov_core::Runtime, n: usize) {
    for _ in 0..n {
        let s = rt.step().expect("a step");
        rt.commit_step(&s.result, &s.effects).expect("a commit");
    }
}

#[test]
fn a_restored_snapshot_replays_the_same_state_hashes() {
    // r3 and r4 count up forever; each step retires one instruction.
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0, addi_inc(3));
    place_insn(&mut mem, 4, addi_inc(4));
    place_insn(&mut mem, 8, (18 << 26) | (0x3ff_fff8 & 0x3ff_fffc));
    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 1_000);
    rt.set_mode(cellgov_core::RuntimeMode::DeterminismCheck);
    rt.register_unit_with(PpuExecutionUnit::new);

    drive(&mut rt, 10);
    let snap = rt.snapshot();
    let mark = rt.trace().bytes().len();
    drive(&mut rt, 30);
    let first = per_step_hashes(&rt.trace().bytes()[mark..]);

    // The restore clears the trace, so the replay's records start at 0.
    rt.restore_into(&snap);
    rt.set_scheduler(cellgov_core::scheduler::RoundRobinScheduler::new());
    drive(&mut rt, 30);
    let replay = per_step_hashes(rt.trace().bytes());

    assert_eq!(first.len(), 30);
    assert_eq!(replay, first);
    let distinct: std::collections::BTreeSet<u64> = first.iter().map(|&(_, h)| h).collect();
    // The branch writes only the PC, which no lane holds.
    assert_eq!(distinct.len(), 20, "each addi moved the state");
}
