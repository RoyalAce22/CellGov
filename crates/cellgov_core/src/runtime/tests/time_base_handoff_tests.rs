//! The runtime hands every step the highest time base any earlier step
//! read, and a restore takes that value back with the rest of the state.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::Runtime;
use crate::scheduler::RoundRobinScheduler;

/// Records the time base the runtime hands each step, and reports `read`
/// as the time base each of its steps read.
#[derive(Clone)]
struct Reader {
    id: UnitId,
    read: Option<u64>,
    seen: Rc<RefCell<Vec<u64>>>,
}

impl ExecutionUnit for Reader {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.seen.borrow_mut().push(ctx.time_base());
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(10),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn time_base_read(&self) -> Option<u64> {
        self.read
    }

    fn snapshot(&self) {}
}

/// A runtime with one [`Reader`] per entry of `reads`, in id order, and
/// the time bases the runtime hands each one.
fn build(reads: &[Option<u64>]) -> (Runtime, Vec<Rc<RefCell<Vec<u64>>>>) {
    let mut rt = Runtime::new(GuestMemory::new(16), Budget::new(1), 100);
    let seen: Vec<_> = reads
        .iter()
        .map(|&read| {
            let seen = Rc::new(RefCell::new(Vec::new()));
            let unit_seen = Rc::clone(&seen);
            rt.register_unit_with(move |id| Reader {
                id,
                read,
                seen: unit_seen,
            });
            seen
        })
        .collect();
    (rt, seen)
}

fn run(rt: &mut Runtime, steps: usize) {
    for _ in 0..steps {
        let mut step = rt.step().expect("every unit is runnable");
        rt.commit_step(&mut step).expect("the step commits");
    }
}

#[test]
fn a_step_is_handed_the_highest_time_base_an_earlier_step_read() {
    // Every clock starts at 0 and each step costs the same, so the units
    // run in id order.
    let (mut rt, seen) = build(&[Some(700), Some(300), None]);
    run(&mut rt, 6);
    assert_eq!(*seen[0].borrow(), [0, 700]);
    assert_eq!(*seen[1].borrow(), [700, 700]);
    assert_eq!(*seen[2].borrow(), [700, 700]);
}

#[test]
fn a_restore_takes_back_the_time_base_read_since_the_snapshot() {
    // The snapshot holds a time base above zero, so a restore that
    // resets the value instead of taking the captured one fails too.
    let (mut rt, seen) = build(&[Some(300), Some(700)]);
    run(&mut rt, 1);
    let snap = rt.snapshot();
    run(&mut rt, 2);
    assert_eq!(*seen[1].borrow(), [300]);
    assert_eq!(*seen[0].borrow(), [0, 700]);

    rt.restore_into(&snap);
    rt.set_scheduler(RoundRobinScheduler::new());
    seen[0].borrow_mut().clear();
    run(&mut rt, 1);
    assert_eq!(*seen[0].borrow(), [300]);
}
