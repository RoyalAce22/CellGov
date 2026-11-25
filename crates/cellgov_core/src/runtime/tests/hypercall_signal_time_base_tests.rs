//! The snapshot of the running threads reads each decrementer at the
//! run's time base, where that time base is past the thread's tick.

use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_lv1::{HypercallRefusal, RunningThreads};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    SEND_EVENT_LOCALLY,
};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use super::{
    call, registers_at, registry, running_threads, runtime, Log, Shared, Step, Thread, BITMAP,
    EE_CLEAR_MSR, EE_SET_MSR, PLUG, PORT, RECORD_SC, SEND_SC, STEP_CAP,
};
use crate::runtime::types::StepError;

/// The tick of the signalling call.
const CALL_TICK: u64 = 0x10;

/// The highest time base that a unit read before the call: past
/// [`CALL_TICK`].
const RUN_TIME_BASE: u64 = 0x40;

/// A thread's next read gets no less than the run's time base, so its
/// decrementer in the snapshot reads that time base.
#[test]
fn a_thread_reads_its_decrementer_at_the_run_s_time_base_where_that_one_is_later() {
    let (registry, _) = registry();
    let at_run_time_base = registers_at(RUN_TIME_BASE as u32);
    assert_eq!(
        running_threads(&registry, GuestTicks::new(CALL_TICK), RUN_TIME_BASE),
        RunningThreads([at_run_time_base, at_run_time_base])
    );
}

/// The time base that the reader reads in its one step: past
/// [`super::DEC_AT_ZERO`], so a scripted thread's decrementer there is negative.
const READER_TIME_BASE: u64 = 0x8000_0000;

/// A unit with no PPE hardware thread that reads [`READER_TIME_BASE`] in
/// its one step and then finishes.
#[derive(Clone)]
struct Reader {
    id: UnitId,
    stepped: bool,
}

impl ExecutionUnit for Reader {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.stepped {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn time_base_read(&self) -> Option<u64> {
        self.stepped.then_some(READER_TIME_BASE)
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.stepped = true;
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

/// The runtime hands the signalling hypercall the run's time base. The
/// reader's read puts that time base past the target's decrementer, so
/// the target's next read finds the decrementer negative and lv1's
/// handler resets it [OFW-4.93 lv1 a:0x2C636C-0x2C637C].
#[test]
fn a_send_after_another_unit_read_a_later_time_base_reads_the_target_s_decrementer_there() {
    let mut rt = runtime();
    rt.register_unit_with(|id| Reader { id, stepped: false });
    let thread_1 = Shared::default();
    let seen = Rc::clone(&thread_1);
    rt.register_unit_with(move |id| Thread {
        id,
        thread: 1,
        script: vec![
            call(SEND_SC, CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, BITMAP]),
            call(RECORD_SC, CONSTRUCT_EVENT_RECEIVE_PORT, &[]),
            call(SEND_SC, CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0]),
            Step::Spin,
            Step::Spin,
            Step::Spin,
        ],
        next: 0,
        seen,
        other: None,
        log: Log::default(),
        msr: EE_CLEAR_MSR,
    });
    let sender = rt.register_unit_with(move |id| Thread {
        id,
        thread: 0,
        script: vec![Step::AwaitReady, call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT])],
        next: 0,
        seen: Shared::default(),
        other: Some(thread_1),
        log: Log::default(),
        msr: EE_SET_MSR,
    });
    let mut error = None;
    for _ in 0..=STEP_CAP {
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(e) => {
                error = Some(e);
                break;
            }
        };
        rt.commit_step(&mut step).expect("commit");
    }
    assert_eq!(error, Some(StepError::HypercallRefused));
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::UnmodelledInput {
            number: SEND_EVENT_LOCALLY,
            unit: sender,
            pc: SEND_SC,
            what: "an interprocessor interrupt to a thread whose decrementer is negative, which \
                   lv1's handler resets",
        })
    );
}
