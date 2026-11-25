//! The runtime hands the hypervisor the decrementer of a mediated take at
//! the run's time base, where a sibling's read put that time base past
//! the taking unit's clock.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_lv1::HypercallRefusal;
use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_time::{Budget, InstructionCost, TICKS_PER_TB_TICK};

use super::{call, run_to_error, runtime, setup, spawn, EE_OFF_MSR, PLUG_BIT, PORT, SEND_SC};
use crate::runtime::types::StepError;

/// The taking thread's decrementer at time base 0: near 0, and below
/// [`SIBLING_TIME_BASE`].
const DEC_AT_ZERO: u32 = 0x10;

/// The time base that the sibling reads at the end of its one step.
const SIBLING_TIME_BASE: u64 = 0x20;

/// PPE hardware thread 0 with no supervisor registers. Its one step ends
/// at the tick of [`SIBLING_TIME_BASE`], and it reads that time base.
#[derive(Clone)]
struct Sibling {
    id: UnitId,
    stepped: bool,
}

impl ExecutionUnit for Sibling {
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
    fn ppe_thread(&self) -> Option<u8> {
        Some(0)
    }
    fn time_base_read(&self) -> Option<u64> {
        self.stepped.then_some(SIBLING_TIME_BASE)
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
            consumed_cost: InstructionCost::new(SIBLING_TIME_BASE * TICKS_PER_TB_TICK),
            local_diagnostics: cellgov_exec::LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

/// [PPC-Book3 p:81 s:6.3] The decrementer counts down at the time base
/// rate. At the take, the decrementer is positive at the thread's own
/// clock and negative at the sibling's later read. The thread's next read
/// gets the later time base, so lv1's handler finds the decrementer
/// negative [OFW-4.93 lv1 a:0x2C632C-0x2C6334].
#[test]
fn a_take_after_a_sibling_read_a_later_time_base_reads_the_decrementer_there() {
    let mut rt = runtime(PLUG_BIT);
    rt.register_unit_with(|id| Sibling { id, stepped: false });
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let (unit, seen) = spawn(&mut rt, 1, script, None, EE_OFF_MSR, SEND_SC + 4);
    seen.borrow_mut().decrementer = DEC_AT_ZERO;
    assert_eq!(run_to_error(&mut rt), StepError::HypercallRefused);
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::MediatedExternal {
            unit,
            pc: SEND_SC + 4,
            what: "a take whose decrementer is negative, which lv1's handler resets and can \
                   send to its trampoline slot 0x28B8",
        })
    );
    assert_eq!(seen.borrow().mediated_takes, 1);
}
