//! The `ExecutionUnit` trait and the `UnitStatus` enum.
//!
//! `ExecutionUnit` is the interface every translated PPU/SPU execution
//! unit (and every fake unit used for testing) implements. The method
//! set is:
//!
//! ```text
//! pub trait ExecutionUnit {
//!     type Snapshot;
//!     fn unit_id(&self) -> UnitId;
//!     fn status(&self) -> UnitStatus;
//!     fn run_until_yield(
//!         &mut self,
//!         budget: Budget,
//!         ctx: &ExecutionContext,
//!         effects: &mut Vec<Effect>,
//!     ) -> ExecutionStepResult;
//!     fn snapshot(&self) -> Self::Snapshot;
//!     fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)>;
//!     fn drain_retired_state_full(
//!         &mut self,
//!     ) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)>;
//! }
//! ```
//!
//! Implementations communicate with the runtime through `ExecutionContext`
//! input and `Effect` output only. They do not import scheduler types,
//! they do not mutate guest-visible state directly, and their
//! `Snapshot` type must be pure deterministic data with no host
//! handles, raw pointers, allocator-dependent internals, mutex guards,
//! or references into runtime-owned memory.

use crate::context::ExecutionContext;
use crate::step_result::ExecutionStepResult;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_time::Budget;

/// Coarse runnability state of an execution unit.
///
/// `UnitStatus` is what the scheduler queries to decide whether a unit
/// belongs in the runnable set. It is a small total enum;
/// finer-grained reasons live in [`crate::YieldReason`] (the most recent
/// yield) and on the unit itself (its internal state machine).
///
/// Discriminants are locked because the trace format is binary and
/// the scheduler may store unit status in trace records. Reordering
/// or renumbering would break replay against any existing trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum UnitStatus {
    /// Eligible to be scheduled. The default for a freshly registered
    /// unit and the state most units are in between yields.
    Runnable = 0,
    /// Parked, waiting on an external event. Not eligible for
    /// scheduling until the runtime transitions the unit back to
    /// `Runnable`. The guest-semantic reason is owned by whichever
    /// subsystem parked the unit -- mailbox / signal / barrier /
    /// DMA waiter lists in `cellgov_mailbox` / `cellgov_signal` /
    /// `cellgov_dma`, and PPU thread `join` waiters in
    /// `cellgov_lv2::ppu_thread::PpuThreadTable`. The scheduler
    /// never branches on the reason; it sees only the opaque
    /// `Blocked` state and skips the unit.
    Blocked = 1,
    /// Has raised a fault and is in fault-handling state. Whether and
    /// how it returns to `Runnable` is architecture-specific. Currently
    /// `Faulted` units are kept out of the runnable set.
    Faulted = 2,
    /// Terminal. The unit has finished its work and will not be
    /// scheduled again. The runtime may keep its snapshot for trace
    /// purposes but must remove it from the runnable set.
    Finished = 3,
}

/// A resumable execution unit.
///
/// Implementations are anything that can take a budget, run for some
/// guest time, and return a step result describing what happened. The
/// runtime owns construction (via the unit registry in
/// `cellgov_core`) and scheduling; implementations own only their own
/// internal state machine.
///
/// **Snapshot rule (hard requirement for replay).** `Self::Snapshot` must
/// be pure deterministic data: no raw pointers, no host handles, no
/// allocator-dependent internals, no mutex guards, no references into
/// runtime-owned memory. A snapshot must be reconstructible into an
/// equivalent unit state on a different host without any environmental
/// dependency. The associated type is unbounded so that implementations
/// have freedom of representation; the determinism rule is architectural
/// and enforced at code review, not by trait bounds.
pub trait ExecutionUnit {
    /// Pure deterministic state capture used for replay and assertions.
    type Snapshot;

    /// Stable identifier assigned at registration time.
    fn unit_id(&self) -> UnitId;

    /// Coarse runnability state queried by the scheduler.
    fn status(&self) -> UnitStatus;

    /// Run the unit until it yields, consuming up to `budget` worth of
    /// progress and observing only the readonly state in `ctx`.
    ///
    /// Effects are pushed into the caller-supplied `effects` vec.
    /// Implementations must preserve the order in which they emit
    /// effects -- the runtime relies on stable intra-step ordering for
    /// validation, conflict diagnostics, fault attribution, and trace
    /// reconstruction.
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult;

    /// Capture the unit's current state as deterministic data. Must
    /// satisfy the snapshot rule documented on the trait.
    fn snapshot(&self) -> Self::Snapshot;

    /// Drain per-instruction state fingerprints retired during the most
    /// recent `run_until_yield`. Returns an empty vector by default.
    ///
    /// Used by the per-step divergence trace. Units opt in by
    /// overriding this method and a corresponding "per-step trace"
    /// setter of their own. Unit implementations that never opt in pay
    /// nothing: the default empty vector allocates no heap.
    ///
    /// Contract: each pair is `(pc, state_hash)` for one retired
    /// instruction, in retirement order. The caller is responsible for
    /// assigning monotonic step indices; the unit does not know its
    /// own position in the global step sequence.
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        Vec::new()
    }

    /// Drain full-register snapshots collected during the most recent
    /// `run_until_yield` inside the unit's configured zoom-in window.
    /// Returns an empty vector by default.
    ///
    /// Each entry is `(pc, gpr, lr, ctr, xer, cr)` for one retired
    /// instruction whose retirement index fell inside the window. The
    /// caller assigns monotonic step indices, matching the indices
    /// used for `drain_retired_state_hashes`.
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        Vec::new()
    }

    /// Drain instruction-variant frequency data collected during
    /// profiling mode. Returns an empty vec by default.
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    /// Drain adjacent-pair frequency data collected during profiling
    /// mode. Returns an empty vec by default.
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        Vec::new()
    }

    /// Notify the unit that guest memory in `[addr, addr+len)` was
    /// written by the commit pipeline. Units that cache decoded
    /// instructions (predecoded shadow) override this to mark the
    /// affected slots stale. Default is a no-op.
    fn invalidate_code(&mut self, _addr: u64, _len: u64) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yield_reason::YieldReason;
    use crate::LocalDiagnostics;
    use cellgov_mem::GuestMemory;

    /// A minimal fake unit that increments a tick counter on every
    /// step, emits one trace marker, and finishes after `max_steps`.
    /// Exists only to prove the trait shape compiles and behaves as
    /// expected; the full fake unit lives in [`crate::fake_isa`].
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
        // The Snapshot type is u64 here; the test exists to demonstrate
        // that snapshots are values, not borrows. A snapshot lives past
        // any reference to the unit.
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
