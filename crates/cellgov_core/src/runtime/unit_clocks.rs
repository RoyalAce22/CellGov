//! Runtime methods that fire DMA completions and timer deadlines against
//! per-unit clocks.
//!
//! Each unit keeps its own clock (see [`crate::registry::UnitRegistry::clock`]).
//! The runtime fires an event due at tick `t` only when no runnable
//! unit's clock is below `t`. That is the conservative rule: an event
//! applies once no unit can still run a step that starts before it
//! [Fujimoto1990 p:33]. Thus a unit behind
//! the event never sees it early, and a unit that the event wakes resumes
//! at `t`.
//!
//! Under the lowest-clock order every step starts at the lowest clock, so
//! the event applies before any step that starts at or after `t`, and
//! after every step that starts before it. A scheduler that runs a unit
//! ahead of the lowest clock runs that unit's steps before the events
//! between the two clocks, and the unit sees them late.

use cellgov_dma::DmaCompletion;
use cellgov_time::GuestTicks;

use crate::timer_queue::TimerWake;

use super::Runtime;

/// The completions and deadlines one firing pass applied, for the trace.
pub(super) type FiredEvents = (Vec<(DmaCompletion, Option<Vec<u8>>)>, Vec<TimerWake>);

impl Runtime {
    /// The tick of the next DMA completion or timer deadline, if any.
    ///
    /// A suspended queue never completes the transfers it holds, so they
    /// are not events.
    pub(super) fn next_event_time(&self) -> Option<GuestTicks> {
        match (
            self.dma_queue.next_event_time(),
            self.timer_wakes.peek_deadline(),
        ) {
            (Some(d), Some(t)) => Some(d.min(t)),
            (d, t) => d.or(t),
        }
    }

    /// Fires each event due by `cap` and by the lowest runnable clock.
    ///
    /// Each pass handles one due tick, in tick order:
    ///
    /// 1. It sets the runtime's time to that tick and fires the events.
    /// 2. It wakes each event waiter whose reservation a landing cleared.
    /// 3. It sets the clock of each unit that it woke to that tick.
    ///
    /// Each pass reads the lowest runnable clock again, because a woken
    /// unit can lower it. With no unit runnable, `cap` alone bounds the
    /// firing. On return, the runtime's time is the same as on entry.
    pub(super) fn fire_due_events(&mut self, cap: GuestTicks) -> FiredEvents {
        let now = self.time;
        let mut due = Vec::new();
        let mut timer_due = Vec::new();
        // The loop reads the next event first: most passes find none due,
        // and the lowest runnable clock costs more to read.
        while let Some(tick) = self.next_event_time().filter(|&tick| tick <= cap) {
            let lowest = self.registry.lowest_runnable_clock();
            if lowest.is_some_and(|lowest| tick > lowest) {
                break;
            }
            self.time = tick;
            due.extend(self.fire_dma_completions());
            timer_due.extend(self.fire_timer_wakes());
            // A landing can clear the reservation of a unit that waits on
            // an event, and that unit also resumes at the landing's tick.
            self.wake_reservation_lost_waiters();
            self.registry.sync_clocks(tick);
        }
        self.time = now;
        (due, timer_due)
    }
}
