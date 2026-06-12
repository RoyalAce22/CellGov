//! Deterministic scheduler: picks the next runnable [`UnitId`] from a
//! [`UnitRegistry`]. Does not run the unit; the runtime loop composes
//! the selection with commit, trace, and time advance.
//!
//! A scheduler implementation is a pure function of (its own state,
//! registry contents). No host time, no `HashMap` iteration order,
//! no host-thread scheduling input.

use crate::registry::UnitRegistry;
use cellgov_event::UnitId;
use cellgov_exec::{UnitStatus, YieldReason};

/// Pluggable scheduling policy.
pub trait Scheduler {
    /// Select the next runnable unit, or `None` if none is runnable.
    ///
    /// May mutate scheduler-internal state; must not mutate the registry.
    /// Must be a deterministic function of scheduler state plus the
    /// sequence of registry-status snapshots observed.
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId>;

    /// Notify the scheduler that the just-completed step ended with
    /// `yield_reason` on `unit`. `woke_others` is `true` iff the
    /// step's dispatch transitioned at least one other unit into
    /// `Runnable`. `holds_critical_section` is `true` iff `unit`
    /// owns at least one lwmutex (or analogous primitive) at the
    /// end of the step. Default: ignored. Implementations that want
    /// time-slice-style stickiness use this hook.
    fn notify_yielded(
        &mut self,
        _unit: UnitId,
        _yield_reason: YieldReason,
        _woke_others: bool,
        _holds_critical_section: bool,
    ) {
    }
}

/// Round-robin scheduler with wake-aware and critical-section-aware
/// stickiness: walks the registry in id order from the position
/// after `last_scheduled`, returns the first `Runnable` unit, wraps
/// around. Skips `Blocked`, `Faulted`, `Finished`.
///
/// Stickiness exceptions, in priority order:
///
/// 1. The previous step's unit holds at least one lwmutex (i.e. is
///    in a critical section) and the yield did not break it.
/// 2. The previous step ended with [`YieldReason::Syscall`] and did
///    not wake any other unit.
/// 3. Otherwise: normal round-robin advance.
///
/// Relies on two [`UnitRegistry`] contracts:
///
/// 1. `registry.iter()` yields ids in ascending order; the two-pass
///    `id > cursor` / `id <= cursor` scan depends on it.
/// 2. `UnitId`s are monotonic and stable across the scheduler's
///    lifetime (no id recycling).
#[derive(Debug, Default)]
pub struct RoundRobinScheduler {
    /// Cursor: id of the most recently selected unit; `None` at start.
    last_scheduled: Option<UnitId>,
    /// Set when the previous step yielded on `Syscall` without waking
    /// any other unit; cleared otherwise.
    sticky: bool,
    /// Consecutive sticky yields. Forces a rotation when it crosses
    /// the starvation guard threshold.
    sticky_streak: u32,
}

impl RoundRobinScheduler {
    /// Forces rotation after this many consecutive sticky-eligible
    /// yields; starvation guard.
    const STICKY_STREAK_LIMIT: u32 = 64;

    /// Construct a fresh scheduler.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Id of the most recently selected unit, if any.
    ///
    /// Only meaningful while the registry's id-stability contract
    /// holds; unit removal would leave the cursor dangling. The
    /// disappearance case is debug-asserted on the next `select_next`;
    /// id recycling is not detected at this layer.
    #[inline]
    pub fn last_scheduled(&self) -> Option<UnitId> {
        self.last_scheduled
    }
}

impl Scheduler for RoundRobinScheduler {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        if registry.is_empty() {
            return None;
        }
        if let Some(c) = self.last_scheduled {
            debug_assert!(
                registry.get(c).is_some(),
                "scheduler cursor {c:?} names an id not present in the registry \
                 (does not detect id recycling, only disappearance)"
            );
        }
        // Snapshot-once: reading `effective_status` twice could
        // diverge if a future refactor makes it stateful.
        let runnables: Vec<UnitId> = registry
            .iter()
            .filter(|(id, _)| registry.effective_status(*id) == Some(UnitStatus::Runnable))
            .map(|(id, _)| id)
            .collect();

        // Ascending order is the invariant the two-pass scan below
        // relies on; bounded size catches a runaway registry.
        debug_assert!(
            runnables.windows(2).all(|w| w[0] < w[1]),
            "scheduler runnables snapshot is not ascending: {runnables:?}"
        );
        debug_assert!(
            runnables.len() < 65_536,
            "scheduler runnables snapshot exceeded 65536; registry is likely broken"
        );

        let chosen = match runnables.len() {
            0 => None,
            1 => Some(runnables[0]),
            _ => match self.last_scheduled {
                // Sticky after a non-waking syscall: real PS3 does
                // not preempt on syscall return when no other unit
                // became runnable, so reselect the same unit.
                Some(c) if self.sticky && runnables.contains(&c) => Some(c),
                Some(c) => runnables
                    .iter()
                    .copied()
                    .find(|&id| id > c)
                    .or_else(|| runnables.iter().copied().find(|&id| id <= c)),
                None => Some(runnables[0]),
            },
        };

        if let Some(id) = chosen {
            self.last_scheduled = Some(id);
        }
        chosen
    }

    fn notify_yielded(
        &mut self,
        _unit: UnitId,
        yield_reason: YieldReason,
        woke_others: bool,
        holds_critical_section: bool,
    ) {
        let in_critical = holds_critical_section && !yield_reason.breaks_critical_section();
        let non_waking_syscall = matches!(yield_reason, YieldReason::Syscall) && !woke_others;
        let want_sticky = in_critical || non_waking_syscall;
        if want_sticky {
            self.sticky_streak = self.sticky_streak.saturating_add(1);
            self.sticky = self.sticky_streak < Self::STICKY_STREAK_LIMIT;
        } else {
            self.sticky_streak = 0;
            self.sticky = false;
        }
    }
}

#[cfg(test)]
#[path = "tests/scheduler_tests.rs"]
mod tests;
