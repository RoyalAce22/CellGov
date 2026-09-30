//! The runtime's problem-state operations reach a registered SPU: the
//! inbound mailbox wakes a parked read, a stop request holds the SPU
//! stopped across that wake, and a unit without problem state refuses.

use cellgov_core::{Runtime, StepError};
use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, ProblemStateError, SignalNotifier};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    SPU_IN_MBOX_DEPTH, SPU_RD_IN_MBOX, SPU_STATUS_P, SPU_STATUS_R, SPU_STATUS_W, SPU_WR_OUT_MBOX,
};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `rdch r5, SPU_RdInMbox` then `stop`.
fn read_mailbox_program() -> [u32; 2] {
    [(0x00D << 21) | (u32::from(SPU_RD_IN_MBOX) << 7) | 5, 0]
}

fn runtime_with_spu() -> (Runtime, UnitId) {
    runtime_running(&read_mailbox_program())
}

fn runtime_running(program: &[u32]) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let mailbox = rt
        .mailbox_registry_mut()
        .register(SPU_IN_MBOX_DEPTH as usize);
    let unit = rt.register_unit_with(|id| {
        assert_eq!(id.raw(), mailbox.raw(), "the SPU's mailbox shares its id");
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu
    });
    (rt, unit)
}

fn step(rt: &mut Runtime) -> Result<(), StepError> {
    let step = rt.step()?;
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    Ok(())
}

/// Step until the SPU stops itself; its `rdch` receives on one step and
/// writes the register on the next.
fn run_to_stop(rt: &mut Runtime, unit: UnitId) {
    for _ in 0..4 {
        if rt.unit_spu_status(unit) != Some(SPU_STATUS_R) {
            return;
        }
        step(rt).expect("the SPU runs");
    }
    panic!("the SPU did not stop");
}

fn spu(rt: &Runtime, unit: UnitId) -> &SpuExecutionUnit {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
}

fn r5(rt: &Runtime, unit: UnitId) -> u32 {
    spu(rt, unit).state().reg_word(5)
}

/// [CBEA p:99 s:8.6.2] an MMIO write to SPU_In_Mbox is available to the SPU's rdch of SPU_RdInMbox.
#[test]
fn an_inbound_mailbox_write_wakes_the_spu_parked_on_it() {
    let (mut rt, unit) = runtime_with_spu();
    step(&mut rt).expect("the rdch parks");
    assert_eq!(rt.unit_spu_status(unit), Some(SPU_STATUS_R));
    rt.write_unit_in_mbox(unit, 7)
        .expect("the SPU has a mailbox");
    run_to_stop(&mut rt, unit);
    assert_eq!(r5(&rt, unit), 7);
    assert_eq!(rt.unit_spu_status(unit), Some(SPU_STATUS_P));
}

/// [CBE-Handbook p:541 s:19.6.6.2] a PPE write to a full inbound mailbox does not stall; a message is lost.
/// [CBE-Handbook p:535 s:19.6.2] the write overwrites the last value written to the mailbox.
#[test]
fn a_write_to_a_full_inbound_mailbox_overwrites_its_newest_message() {
    let rdch = |rt: u32| (0x00D << 21) | (u32::from(SPU_RD_IN_MBOX) << 7) | rt;
    let (mut rt, unit) = runtime_running(&[rdch(5), rdch(6), rdch(7), rdch(8), 0]);
    for message in [0x11, 0x22, 0x33, 0x44, 0x55] {
        rt.write_unit_in_mbox(unit, message)
            .expect("the write does not stall");
    }
    for _ in 0..16 {
        if rt.unit_spu_status(unit) != Some(SPU_STATUS_R) {
            break;
        }
        step(&mut rt).expect("the SPU runs");
    }
    assert_eq!(rt.unit_spu_status(unit), Some(SPU_STATUS_P));
    let state = spu(&rt, unit).state();
    assert_eq!(
        [5, 6, 7, 8].map(|r| state.reg_word(r)),
        [0x11, 0x22, 0x33, 0x55],
        "the fifth write replaced the fourth message",
    );
}

/// [CBEA p:92 s:8.5.1] a stop request stops instruction issue until a run request.
/// [CBEA p:94 s:8.5.2] an SPU stopped while waiting on a blocked channel reports W.
#[test]
fn a_stop_request_holds_a_parked_spu_stopped_until_it_restarts() {
    let (mut rt, unit) = runtime_with_spu();
    step(&mut rt).expect("the rdch parks");
    rt.request_unit_stop(unit)
        .expect("the SPU has problem state");
    assert_eq!(rt.unit_spu_status(unit), Some(SPU_STATUS_W));

    rt.write_unit_in_mbox(unit, 9)
        .expect("the SPU has a mailbox");
    assert!(
        step(&mut rt).is_err(),
        "a stopped SPU is not scheduled by the mailbox wake"
    );

    rt.restart_unit(unit).expect("a run request restarts it");
    run_to_stop(&mut rt, unit);
    assert_eq!(r5(&rt, unit), 9);
}

#[test]
fn the_signal_npc_and_outbound_mailbox_operations_reach_the_spu_through_the_runtime() {
    // `il r4, 42`, `wrch SPU_WrOutMbox, r4`, then `stop`.
    let (mut rt, unit) = runtime_running(&[
        (0x081 << 23) | (42 << 7) | 4,
        (0x10D << 21) | (u32::from(SPU_WR_OUT_MBOX) << 7) | 4,
        0,
    ]);
    rt.write_unit_signal(unit, SignalNotifier::Two, 3)
        .expect("the SPU has problem state");
    assert_eq!(rt.write_unit_npc(unit, 4), Err(ProblemStateError::Running));
    run_to_stop(&mut rt, unit);

    let signals = spu(&rt, unit).state().signals;
    assert_eq!(
        signals.map(|r| (r.word, r.pending)),
        [(0, false), (3, true)]
    );
    assert_eq!(rt.read_unit_out_mbox(unit), Ok(Some(42)));
    assert_eq!(rt.read_unit_out_mbox(unit), Ok(None));
    rt.write_unit_npc(unit, 0x20).expect("the SPU is stopped");
    assert_eq!(rt.unit_stop_registers(unit).map(|r| r.npc), Some(0x20));
}

#[test]
fn a_unit_without_problem_state_and_an_unknown_id_are_refused() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let fake = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![]));
    assert_eq!(rt.unit_spu_status(fake), None);
    assert_eq!(
        rt.request_unit_stop(fake),
        Err(ProblemStateError::NoProblemState)
    );
    assert_eq!(
        rt.write_unit_in_mbox(fake, 1),
        Err(ProblemStateError::NoProblemState)
    );
    assert_eq!(
        rt.write_unit_signal(UnitId::new(99), SignalNotifier::One, 1),
        Err(ProblemStateError::UnknownUnit)
    );
}
