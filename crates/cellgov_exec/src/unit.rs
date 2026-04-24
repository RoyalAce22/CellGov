//! The `ExecutionUnit` trait and `UnitStatus` enum.

use crate::context::ExecutionContext;
use crate::step_result::ExecutionStepResult;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_time::Budget;

/// Coarse runnability state queried by the scheduler.
///
/// Finer-grained reasons for the most recent yield live in
/// [`crate::YieldReason`]; internal arch state lives on the unit
/// itself.
///
/// Discriminants are part of the binary trace format: do not reorder
/// or renumber.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UnitStatus {
    /// Eligible to be scheduled.
    Runnable = 0,
    /// Parked, waiting on an external event. The guest-semantic
    /// reason is owned by whichever subsystem parked the unit
    /// (mailbox / signal / barrier / DMA waiter lists in
    /// `cellgov_mailbox` / `cellgov_signal` / `cellgov_dma`, PPU
    /// thread `join` waiters in `cellgov_lv2::ppu_thread`). The
    /// scheduler sees only the opaque state and skips the unit.
    Blocked = 1,
    /// Has raised a fault; kept out of the runnable set. Return to
    /// `Runnable` is architecture-specific.
    Faulted = 2,
    /// Terminal. Must be removed from the runnable set after the
    /// runtime observes this; snapshots may still be retained for
    /// trace purposes.
    Finished = 3,
}

/// A resumable execution unit: something that can take a budget, run
/// for some guest time, and return a step result.
///
/// Implementations communicate with the runtime through
/// `ExecutionContext` input and `Effect` output only. They do not
/// import scheduler types and do not mutate guest-visible state
/// directly.
///
/// **Snapshot rule (required for replay).** `Self::Snapshot` must be
/// pure deterministic data: no raw pointers, no host handles, no
/// allocator-dependent internals, no mutex guards, no references
/// into runtime-owned memory. A snapshot must be reconstructible
/// into an equivalent unit state on a different host. The rule is
/// architectural; the associated type is unbounded so implementations
/// have freedom of representation.
pub trait ExecutionUnit {
    /// Pure deterministic state capture used for replay and assertions.
    type Snapshot;

    /// Stable identifier assigned at registration time.
    fn unit_id(&self) -> UnitId;

    /// Coarse runnability state queried by the scheduler.
    fn status(&self) -> UnitStatus;

    /// Run until the unit yields, consuming up to `budget` and
    /// observing only the readonly state in `ctx`.
    ///
    /// Effects are pushed into `effects` in emission order. The
    /// runtime relies on stable intra-step ordering for validation,
    /// conflict diagnostics, fault attribution, and trace
    /// reconstruction.
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult;

    /// Capture current state as deterministic data per the snapshot
    /// rule on the trait.
    fn snapshot(&self) -> Self::Snapshot;

    /// Drain `(pc, state_hash)` pairs retired during the most recent
    /// `run_until_yield`, in retirement order. The default returns an
    /// empty vec and allocates nothing.
    ///
    /// The caller assigns monotonic step indices; the unit does not
    /// know its own position in the global step sequence.
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        Vec::new()
    }

    /// Drain full-register snapshots collected during the most recent
    /// `run_until_yield` inside the unit's configured zoom-in window.
    /// Each entry is `(pc, gpr, lr, ctr, xer, cr)` in retirement
    /// order. Step indices pair with
    /// [`Self::drain_retired_state_hashes`].
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        Vec::new()
    }

    /// Drain instruction-variant frequency data from profiling mode.
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    /// Drain adjacent-pair frequency data from profiling mode.
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        Vec::new()
    }

    /// Notify the unit that guest memory in `[addr, addr+len)` was
    /// written by the commit pipeline. Units with a predecoded
    /// shadow override this to mark affected slots stale.
    fn invalidate_code(&mut self, _addr: u64, _len: u64) {}

    /// Return `(shadow_hits, shadow_misses)` for units with a
    /// predecoded instruction shadow; others report `(0, 0)`. A
    /// high miss ratio indicates fetches outside the shadowed
    /// region (e.g. PRX bodies) falling back to decode-on-fetch.
    fn shadow_stats(&self) -> (u64, u64) {
        (0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yield_reason::YieldReason;
    use crate::LocalDiagnostics;
    use cellgov_mem::GuestMemory;

    struct CountingUnit {
        id: UnitId,
        steps: u64,
        max_steps: u64,
    }

    impl ExecutionUnit for CountingUnit {
        type Snapshot = u64;

        fn unit_id(&self) -> UnitId {
            self.id
        }

        fn status(&self) -> UnitStatus {
            if self.steps >= self.max_steps {
                UnitStatus::Finished
            } else {
                UnitStatus::Runnable
            }
        }

        fn run_until_yield(
            &mut self,
            budget: Budget,
            _ctx: &ExecutionContext<'_>,
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            self.steps += 1;
            let yield_reason = if self.steps >= self.max_steps {
                YieldReason::Finished
            } else {
                YieldReason::BudgetExhausted
            };
            effects.push(Effect::TraceMarker {
                marker: self.steps as u32,
                source: self.id,
            });
            ExecutionStepResult {
                yield_reason,
                consumed_budget: budget,
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            }
        }

        fn snapshot(&self) -> u64 {
            self.steps
        }
    }

    #[test]
    fn unit_status_discriminants_locked() {
        assert_eq!(UnitStatus::Runnable as u8, 0);
        assert_eq!(UnitStatus::Blocked as u8, 1);
        assert_eq!(UnitStatus::Faulted as u8, 2);
        assert_eq!(UnitStatus::Finished as u8, 3);
    }

    #[test]
    fn counting_unit_runs_to_completion() {
        let mem = GuestMemory::new(16);
        let ctx = ExecutionContext::new(&mem);
        let mut unit = CountingUnit {
            id: UnitId::new(7),
            steps: 0,
            max_steps: 3,
        };
        assert_eq!(unit.unit_id(), UnitId::new(7));
        assert_eq!(unit.status(), UnitStatus::Runnable);

        let mut effects = Vec::new();
        let r1 = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
        assert_eq!(r1.yield_reason, YieldReason::BudgetExhausted);
        assert_eq!(r1.consumed_budget, Budget::new(10));
        assert_eq!(effects.len(), 1);
        assert_eq!(unit.snapshot(), 1);
        assert_eq!(unit.status(), UnitStatus::Runnable);

        effects.clear();
        let _ = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
        effects.clear();
        let r3 = unit.run_until_yield(Budget::new(10), &ctx, &mut effects);
        assert_eq!(r3.yield_reason, YieldReason::Finished);
        assert_eq!(unit.snapshot(), 3);
        assert_eq!(unit.status(), UnitStatus::Finished);
    }

    #[test]
    fn snapshot_is_value_data() {
        let mut unit = CountingUnit {
            id: UnitId::new(0),
            steps: 5,
            max_steps: 10,
        };
        let snap_before = unit.snapshot();
        let mem = GuestMemory::new(8);
        let ctx = ExecutionContext::new(&mem);
        let mut effects = Vec::new();
        let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut effects);
        let snap_after = unit.snapshot();
        assert_eq!(snap_before, 5);
        assert_eq!(snap_after, 6);
    }
}
