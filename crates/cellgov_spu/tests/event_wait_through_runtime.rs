//! An SPU waits on an event through `SPU_RdEventStat`, and each event
//! source the model raises ends the wait: the inbound mailbox, a signal
//! notification, the tag-group status update, and the multisource
//! synchronization.

// [CBEA p:147 s:9.11.1] a read of SPU_RdEventStat with count 0 stalls until an enabled event occurs.
// [CBEA p:149 s:9.11.1], [CBEA p:150 s:9.11.1] each event is the edge of its source channel's count from 0 to nonzero.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{SignalNotifier, StallWake, UnitStatus, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    event, MFC_CMD, MFC_EAL, MFC_GET, MFC_LSA, MFC_SIZE, MFC_TAG_ID, MFC_TAG_UPDATE_ALL,
    MFC_WR_MSSYNC_REQ, MFC_WR_TAG_MASK, MFC_WR_TAG_UPDATE, SPU_IN_MBOX_DEPTH, SPU_RD_EVENT_STAT,
    SPU_WR_EVENT_MASK,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `wrch $ch<channel>, rt`.
const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

/// `rdch rt, $ch<channel>`.
const fn rdch(channel: u8, rt: u32) -> u32 {
    (0x00D << 21) | ((channel as u32) << 7) | rt
}

/// A runtime holding one SPU that runs `program` with `regs` set, and
/// an inbound mailbox beside it.
fn runtime(program: &[u32], regs: &[(u8, u32)]) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 200);
    let mailbox = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        assert_eq!(id.raw(), mailbox.raw(), "the SPU's mailbox shares its id");
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        for &(reg, value) in regs {
            state.set_reg_word_splat(reg, value);
        }
        spu
    });
    (rt, unit)
}

fn step(rt: &mut Runtime) {
    let step = rt.step().expect("a unit runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
}

/// Steps until the SPU stops and returns its register 5.
fn finish(rt: &mut Runtime, unit: UnitId) -> u32 {
    for _ in 0..40 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Finished)
    );
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
        .state()
        .reg_word(5)
}

fn parked_on_an_event(rt: &Runtime, unit: UnitId) -> bool {
    rt.registry().effective_status(unit) == Some(UnitStatus::Blocked)
        && rt
            .registry()
            .get(unit)
            .and_then(|unit| unit.channel_stall())
            .map(|stall| stall.wake)
            == Some(StallWake::Event)
}

/// Enable the events in register 10, wait, stop.
const WAIT: [u32; 3] = [wrch(SPU_WR_EVENT_MASK, 10), rdch(SPU_RD_EVENT_STAT, 5), 0];

/// [CBEA p:150 s:9.11.1] Mb is set when the SPU_RdInMbox count changes from 0 to nonzero.
#[test]
fn a_mailbox_write_ends_a_wait_on_mb() {
    let (mut rt, unit) = runtime(&WAIT, &[(10, event::MB)]);
    step(&mut rt);
    assert!(parked_on_an_event(&rt, unit));
    rt.write_unit_in_mbox(unit, 7).expect("problem state");
    assert_eq!(finish(&mut rt, unit), event::MB);
}

/// [CBEA p:150 s:9.11.1] S1 is set when the SPU Signal Notification 1 count changes from 0 to nonzero.
#[test]
fn a_signal_write_ends_a_wait_on_s1_and_a_masked_one_does_not() {
    let (mut rt, unit) = runtime(&WAIT, &[(10, event::S1)]);
    step(&mut rt);
    rt.write_unit_signal(unit, SignalNotifier::Two, 5)
        .expect("problem state");
    for _ in 0..3 {
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert!(
        parked_on_an_event(&rt, unit),
        "S2 is masked, so the SPU waits again"
    );
    rt.write_unit_signal(unit, SignalNotifier::One, 9)
        .expect("problem state");
    assert_eq!(finish(&mut rt, unit), event::S1);
}

/// [CBEA p:149 s:9.11.1] Tg is set when the MFC_RdTagStat count changes from 0 to nonzero.
#[test]
fn a_tag_status_update_ends_a_wait_on_tg() {
    let program = [
        wrch(MFC_LSA, 11),
        wrch(MFC_EAL, 12),
        wrch(MFC_SIZE, 13),
        wrch(MFC_TAG_ID, 14),
        wrch(MFC_CMD, 15),
        wrch(MFC_WR_TAG_MASK, 16),
        wrch(MFC_WR_TAG_UPDATE, 17),
        wrch(SPU_WR_EVENT_MASK, 10),
        rdch(SPU_RD_EVENT_STAT, 5),
        0,
    ];
    let (mut rt, unit) = runtime(
        &program,
        &[
            (10, event::TG),
            (11, 0x800),
            (12, 0x100),
            (13, 16),
            (14, 3),
            (15, MFC_GET),
            (16, 1 << 3),
            (17, MFC_TAG_UPDATE_ALL),
        ],
    );
    let mut parked = false;
    for _ in 0..40 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        parked |= parked_on_an_event(&rt, unit);
        let Ok(step) = rt.step() else { break };
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert!(parked, "the SPU waited for the get");
    assert_eq!(finish(&mut rt, unit), event::TG);
}

/// [CBEA p:148 s:9.11.1] a MFC_WrMSSyncReq write with no transfer pending triggers Ms at once.
#[test]
fn a_multisource_request_with_nothing_in_flight_ends_a_wait_on_ms_at_once() {
    let program = [
        wrch(SPU_WR_EVENT_MASK, 10),
        wrch(MFC_WR_MSSYNC_REQ, 0),
        rdch(SPU_RD_EVENT_STAT, 5),
        0,
    ];
    let (mut rt, unit) = runtime(&program, &[(10, event::MS)]);
    let first = rt.step().expect("the SPU runs");
    assert_ne!(first.result.yield_reason, YieldReason::ChannelStall);
    rt.commit_step(&first.result, &first.effects)
        .expect("the step commits");
    assert_eq!(finish(&mut rt, unit), event::MS);
}
