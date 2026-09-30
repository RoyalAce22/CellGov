//! The unit snapshot holds the whole SPU context, and restore is its
//! exact inverse.

use crate::state::{SignalNotifyMode, SpuObservableSnapshot, TagUpdateCondition, SPU_LS_SIZE};
use crate::stop::{SpuStop, SpuStopKind};
use crate::SpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{
    ChannelStall, ExecutionContext, ExecutionUnit, StallWake, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED;
use cellgov_sync::ReservedLine;
use cellgov_time::Budget;

/// xorshift64: a fixed sequence for each seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn word(&mut self) -> u32 {
        self.next() as u32
    }

    fn flag(&mut self) -> bool {
        self.next() & 1 == 1
    }
}

/// A unit with every part of its context set from `seed`.
fn random_unit(seed: u64) -> SpuExecutionUnit {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut unit = SpuExecutionUnit::new(UnitId::new(3));
    let s = unit.state_mut();
    for reg in s.regs.iter_mut() {
        for byte in reg.iter_mut() {
            *byte = r.next() as u8;
        }
    }
    for _ in 0..64 {
        let at = (r.next() % SPU_LS_SIZE as u64) as usize;
        s.ls[at] = r.next() as u8;
    }
    s.pc = r.word() & 0x3_FFFC;
    for signal in s.signals.iter_mut() {
        signal.mode = if r.flag() {
            SignalNotifyMode::LogicalOr
        } else {
            SignalNotifyMode::Overwrite
        };
        signal.word = r.word();
        signal.pending = r.flag();
    }
    let c = &mut s.channels;
    c.mfc_lsa = r.word();
    c.mfc_eah = r.word();
    c.mfc_eal = r.word();
    c.mfc_size = r.word();
    c.mfc_tag_id = r.word();
    c.tag_mask = r.word();
    c.tag_status = r.word();
    c.atomic_status = r.word();
    c.cmd_queue_free = r.word() % 17;
    c.tag_update = [
        None,
        Some(TagUpdateCondition::Any),
        Some(TagUpdateCondition::All),
    ][(r.next() % 3) as usize];
    c.tag_status_read = r.flag().then(|| r.word());
    c.atomic_status_ready = r.flag();
    c.in_mbox = (0..r.next() % 5).map(|_| r.word()).collect();
    c.out_mbox = r.flag().then(|| r.word());
    c.list_stall_status = r.word();
    c.mssync_tracking = r.flag().then(|| r.next());
    c.mssync_horizon = r.flag().then(|| r.next());
    c.pending_events = r.word();
    c.event_mask = r.word();
    c.event_count = r.flag();
    c.event_levels = r.word();
    s.reservation = r
        .flag()
        .then(|| ReservedLine::containing(r.next() & 0xFFFF_FF80));
    s.stop = r
        .flag()
        .then(|| SpuStop::new(SpuStopKind::Stop, r.next() as u16, s.pc, s.lslr));
    s.fpscr = (u128::from(r.next()) << 64 | u128::from(r.next())) & FPSCR_DEFINED;
    s.interrupts_enabled = r.flag();
    s.srr0 = r.word();
    unit.status = [
        UnitStatus::Runnable,
        UnitStatus::Blocked,
        UnitStatus::Finished,
        UnitStatus::Faulted,
    ][(r.next() % 4) as usize];
    unit.stall = r.flag().then_some(ChannelStall {
        channel: 29,
        wake: StallWake::MailboxDelivery,
    });
    unit
}

/// [CBEA p:241 s:17] an implementation supports a full save and restore of an SPE context.
#[test]
fn restoring_a_snapshot_into_a_fresh_unit_reproduces_the_snapshot() {
    for seed in 1..=64 {
        let unit = random_unit(seed);
        let snapshot = unit.snapshot();
        let mut fresh = SpuExecutionUnit::new(UnitId::new(3));
        fresh.restore(snapshot.clone());
        assert_eq!(fresh.snapshot(), snapshot, "seed {seed}");
        assert_eq!(fresh.state(), unit.state(), "seed {seed}");
        assert_eq!(fresh.status(), unit.status(), "seed {seed}");
        assert_eq!(fresh.channel_stall(), unit.channel_stall(), "seed {seed}");
    }
}

#[test]
fn the_comparison_view_and_the_local_store_hash_derive_from_the_snapshot() {
    for seed in 1..=16 {
        let unit = random_unit(seed);
        let snapshot = unit.snapshot();
        assert_eq!(
            snapshot.observable(),
            SpuObservableSnapshot::capture(unit.state())
        );
        assert_eq!(unit.local_memory_hash(), Some(snapshot.local_store_hash()));
    }
}

/// A unit restored mid-program runs the rest of it as the original does.
#[test]
fn a_restored_unit_continues_as_the_original() {
    // il r3, 1; ai r3, r3, 1 (x3); stop.
    let il = (0x081 << 23) | (1 << 7) | 3;
    let ai = (0x1C << 24) | (1 << 14) | (3 << 7) | 3;
    let program: [u32; 5] = [il, ai, ai, ai, 0x0000_0001];
    let mut unit = SpuExecutionUnit::new(UnitId::new(3));
    for (i, word) in program.iter().enumerate() {
        unit.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    let mem = GuestMemory::new(0x100);
    let ctx = ExecutionContext::new(&mem);
    unit.run_until_yield(Budget::new(2), &ctx, &mut Vec::new());
    let mut copy = SpuExecutionUnit::new(UnitId::new(3));
    copy.restore(unit.snapshot());
    for spu in [&mut unit, &mut copy] {
        let result = spu.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
        assert_eq!(result.yield_reason, YieldReason::Finished);
    }
    assert_eq!(unit.state().reg_word(3), 4);
    assert_eq!(copy.snapshot(), unit.snapshot());
}
