//! Two edges of the problem-state operations: an NPC write after a
//! mailbox read took a message returns the message, and a unit CellGov
//! refused reports itself stopped and refuses an NPC write.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{ProblemStateError, UnitStatus};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{SPU_IN_MBOX_DEPTH, SPU_RD_IN_MBOX, SPU_STATUS_R};
use cellgov_spu::SpuExecutionUnit;
use cellgov_sync::MailboxId;
use cellgov_time::Budget;

/// `rdch r5, SPU_RdInMbox`, `stop`, `il r6, 1`, `stop`.
fn program() -> [u32; 4] {
    [
        (0x00D << 21) | (u32::from(SPU_RD_IN_MBOX) << 7) | 5,
        0,
        (0x081 << 23) | (1 << 7) | 6,
        0,
    ]
}

fn runtime_with_spu() -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let mailbox = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        assert_eq!(id.raw(), mailbox.raw(), "the SPU's mailbox shares its id");
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program().iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });
    (rt, unit)
}

fn step(rt: &mut Runtime) {
    let step = rt.step().expect("the SPU runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
}

fn spu(rt: &Runtime, unit: UnitId) -> &SpuExecutionUnit {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
}

fn occupancy(rt: &mut Runtime, unit: UnitId) -> usize {
    rt.mailbox_registry_mut()
        .get_mut(MailboxId::new(unit.raw()))
        .expect("the SPU's mailbox")
        .len()
}

/// [CBEA p:95 s:8.5.3] a restart resumes at SPU_NPC, which a write while the SPU is stopped replaces.
#[test]
fn an_npc_write_returns_the_message_an_abandoned_read_took() {
    let (mut rt, unit) = runtime_with_spu();
    step(&mut rt); // the rdch parks on the empty mailbox
    rt.write_unit_in_mbox(unit, 9)
        .expect("the SPU has a mailbox");
    step(&mut rt); // the rdch runs again and its commit takes the message
    assert_eq!(occupancy(&mut rt, unit), 0, "the commit took the message");
    rt.write_unit_in_mbox(unit, 10).expect("a second message");

    rt.request_unit_stop(unit).expect("problem state");
    rt.write_unit_npc(unit, 8).expect("the SPU is stopped");
    let queued: Vec<u32> = rt
        .mailbox_registry_mut()
        .get_mut(MailboxId::new(unit.raw()))
        .expect("the SPU's mailbox")
        .iter()
        .copied()
        .collect();
    assert_eq!(
        queued,
        [9, 10],
        "the message is back, ahead of the one sent after it"
    );

    rt.restart_unit(unit).expect("restart");
    step(&mut rt);
    let state = spu(&rt, unit).state();
    assert_eq!(state.reg_word(6), 1, "the SPU ran the word at the new NPC");
    assert_eq!(state.reg_word(5), 0, "the abandoned read wrote nothing");
    assert_eq!(state.stop.map(|stop| stop.npc), Some(0x10));
}

/// [CBEA p:94 s:8.5.2] R is 1 only while the SPU runs.
#[test]
fn a_refused_spu_reports_itself_stopped_and_refuses_an_npc_write() {
    let (mut rt, unit) = runtime_with_spu();
    assert_eq!(rt.unit_spu_status(unit), Some(SPU_STATUS_R));
    rt.set_unit_status_override(unit, UnitStatus::Faulted);
    assert_eq!(rt.unit_spu_status(unit), Some(0));
    assert_eq!(rt.write_unit_npc(unit, 8), Err(ProblemStateError::Refused));
}

#[test]
fn an_spu_that_faulted_itself_reports_stopped_and_refuses_an_npc_write() {
    use cellgov_exec::{ExecutionContext, ExecutionUnit};
    // `wrch 7, r3`: a channel the model does not implement.
    let mut spu = SpuExecutionUnit::new(UnitId::new(0));
    spu.state_mut().ls[..4].copy_from_slice(&((0x10D_u32 << 21) | (7 << 7) | 3).to_be_bytes());
    let mem = GuestMemory::new(0x1000);
    let mut effects = Vec::new();
    spu.run_until_yield(Budget::new(10), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(spu.status(), UnitStatus::Faulted);
    assert_eq!(spu.spu_status(), Some(0));
    assert_eq!(spu.write_npc(8), Err(ProblemStateError::Refused));
}
