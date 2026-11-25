//! The decrementer wake of a PPE hardware thread that lv1 paused in
//! mode 0.
//!
//! A pause that lets the decrementer wake the caller queues a timer wake
//! at the tick where the caller's decrementer reaches 0. The wake fires as
//! every timer wake does:
//! - at step entry, once every runnable clock reaches the tick;
//! - in the time warp of a run whose units are all blocked.
//!
//! So a run whose threads are all paused moves guest time to the earliest
//! decrementer deadline. A pause that the decrementer does not end queues
//! no wake. A run whose threads all hold such a pause stops at
//! [`crate::StepError::AllBlocked`].

use cellgov_event::UnitId;
use cellgov_lv1::{DecrementerWake, HypercallAnswer, InterruptChange, PausedThread, StateChange};

use crate::runtime::hypercall::saved_msrs;
use crate::runtime::Runtime;
use crate::timer_queue::TimerWakeKind;

impl Runtime {
    /// Queues the decrementer wake of `unit`, which `answer` paused, when
    /// lv1's wake flag lets the decrementer end the pause.
    ///
    /// The decrementer reads at the later of the runtime's time and the
    /// unit's clock, and at the run's time base. A unit that gives no
    /// decrementer queues no wake.
    pub(in crate::runtime) fn queue_decrementer_wake(
        &mut self,
        unit: UnitId,
        answer: &HypercallAnswer,
    ) {
        if !answer.changes.iter().any(pauses_with_decrementer_wake) {
            return;
        }
        let at = self.time.max(self.registry.clock(unit));
        let Some(deadline) = self
            .registry
            .get(unit)
            .and_then(|u| u.decrementer_zero(at, self.time_base))
        else {
            return;
        };
        let displaced = self
            .timer_wakes
            .insert(deadline, unit, TimerWakeKind::Decrementer);
        if let Some(prior) = displaced {
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_decrementer_wake_displaced",
                format_args!(
                    "{unit:?} paused with {prior:?} still live, so the wake that ended its \
                     previous wait failed to cancel the deadline"
                ),
            );
        }
    }

    /// Ends the pause of `unit` at its decrementer deadline, and returns
    /// whether the unit runs again.
    ///
    /// The unit stays parked in these cases:
    /// - the unit runs no PPE hardware thread with supervisor registers;
    /// - the hypervisor holds the thread in no pause that its decrementer
    ///   ends;
    /// - the hypervisor refuses the return. It keeps the refusal, and the
    ///   next step stops the run with it.
    pub(super) fn wake_on_decrementer(&mut self, unit: UnitId) -> bool {
        let Some(registered) = self.registry.get(unit) else {
            return false;
        };
        let (Some(thread), Some(registers)) = (
            registered.ppe_thread(),
            registered.boundary_registers(self.time, self.time_base),
        ) else {
            return false;
        };
        let wake = DecrementerWake {
            unit,
            thread,
            pc: registers.pc,
        };
        let msrs = saved_msrs(&self.registry);
        let space = self.spaces.space_of(unit);
        let storage =
            crate::runtime::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        match self.hypervisor.wake_on_decrementer(&wake, storage, &msrs) {
            Ok(Some(resume)) => {
                self.resume_paused_thread(&resume);
                true
            }
            Ok(None) | Err(_) => false,
        }
    }
}

/// Whether `change` pauses a thread that its decrementer wakes.
fn pauses_with_decrementer_wake(change: &StateChange) -> bool {
    matches!(
        change,
        StateChange::Interrupt(change) if matches!(
            **change,
            InterruptChange::ThreadPaused(PausedThread {
                decrementer_wakes: true,
                ..
            })
        )
    )
}
