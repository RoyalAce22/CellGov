//! An SPU waiting on Le or Me through `SPU_RdEventStat` wakes when its
//! full outbound mailbox, or its full outbound interrupt mailbox, is read.

// [CBEA p:161 s:9.12.6] Me is the SPU_WrOutIntrMbox count changing from 0 (full) to nonzero.
// [CBEA p:162 s:9.12.7] Le is the SPU_WrOutMbox count changing from 0 (full) to nonzero.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{StallWake, UnitStatus};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    event, SPU_IN_MBOX_DEPTH, SPU_RD_EVENT_STAT, SPU_WR_EVENT_ACK, SPU_WR_EVENT_MASK,
    SPU_WR_OUT_INTR_MBOX, SPU_WR_OUT_MBOX,
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

/// A runtime with one SPU whose program does these steps:
///
/// 1. Acknowledge every event.
/// 2. Fill `mailbox` with register 11.
/// 3. Enable the events in register 10.
/// 4. Wait on `SPU_RdEventStat`.
/// 5. Stop.
fn runtime(mailbox: u8, events: u32) -> (Runtime, UnitId) {
    let program = [
        wrch(SPU_WR_EVENT_ACK, 12),
        wrch(mailbox, 11),
        wrch(SPU_WR_EVENT_MASK, 10),
        rdch(SPU_RD_EVENT_STAT, 5),
        0,
    ];
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 200);
    let inbound = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        assert_eq!(id.raw(), inbound.raw(), "the SPU's mailbox shares its id");
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        state.channels.out_intr_mbox_connected = true;
        state.set_reg_word_splat(10, events);
        state.set_reg_word_splat(11, 0x0122_2222);
        state.set_reg_word_splat(12, u32::MAX);
        spu
    });
    (rt, unit)
}

fn step(rt: &mut Runtime) {
    let step = rt.step().expect("a unit runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
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

#[test]
fn reading_the_outbound_mailbox_ends_a_wait_on_le() {
    let (mut rt, unit) = runtime(SPU_WR_OUT_MBOX, event::LE | event::ME);
    step(&mut rt);
    assert!(
        parked_on_an_event(&rt, unit),
        "the full mailbox raised nothing"
    );
    assert_eq!(rt.read_unit_out_mbox(unit), Ok(Some(0x0122_2222)));
    assert_eq!(finish(&mut rt, unit), event::LE);
}

#[test]
fn reading_the_interrupt_mailbox_ends_a_wait_on_me() {
    let (mut rt, unit) = runtime(SPU_WR_OUT_INTR_MBOX, event::LE | event::ME);
    step(&mut rt);
    assert!(
        parked_on_an_event(&rt, unit),
        "the full mailbox raised nothing"
    );
    assert_eq!(rt.read_unit_out_intr_mbox(unit), Ok(Some(0x0122_2222)));
    assert_eq!(finish(&mut rt, unit), event::ME);
}
