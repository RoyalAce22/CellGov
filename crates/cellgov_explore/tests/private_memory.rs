//! A divergence confined to a unit's private memory is a divergence.
//!
//! An SPU computes in its local store, and a schedule can leave that
//! store different with committed memory the same. The observable the
//! verdict compares folds every unit's private memory beside the
//! committed memory, so such a pair of schedules is schedule-sensitive.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_explore::{explore_window, ExplorationConfig, OutcomeClass};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

/// The word the writer stores and the reader copies.
const WORD: u64 = 0x40;

fn word() -> ByteRange {
    ByteRange::new(GuestAddr::new(WORD), 4).unwrap()
}

/// Stores one value to [`WORD`] and finishes.
#[derive(Clone)]
struct Writer {
    id: UnitId,
    done: bool,
}

impl ExecutionUnit for Writer {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.done {
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
        self.done = true;
        effects.push(Effect::shared_write(
            word(),
            WritePayload::new(vec![0xAB; 4]),
            self.id,
            GuestTicks::ZERO,
        ));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// Copies [`WORD`] into its private memory and finishes. It writes no
/// committed memory, so only its private memory says which order ran.
#[derive(Clone)]
struct Reader {
    id: UnitId,
    done: bool,
    local: [u8; 4],
    /// Whether the unit reports its private memory to the observable.
    reports: bool,
}

impl ExecutionUnit for Reader {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.done {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.done = true;
        let bytes = ctx.memory().read(word()).unwrap();
        self.local.copy_from_slice(bytes);
        effects.push(Effect::SharedReadIntent {
            range: word(),
            source: self.id,
        });
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn local_memory_hash(&self) -> Option<u64> {
        self.reports.then(|| {
            let mut hasher = cellgov_mem::Fnv1aHasher::new();
            hasher.write(&self.local);
            hasher.finish()
        })
    }

    fn snapshot(&self) {}
}

fn workload(reader_reports: bool) -> impl FnMut() -> Runtime {
    move || {
        let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 16);
        rt.register_unit_with(|id| Writer { id, done: false });
        rt.register_unit_with(move |id| Reader {
            id,
            done: false,
            local: [0; 4],
            reports: reader_reports,
        });
        rt
    }
}

fn config() -> ExplorationConfig {
    ExplorationConfig {
        max_schedules: 16,
        max_steps_per_run: 100,
    }
}

/// The premise: with the reader's private memory outside the
/// observable, the two orders leave the same committed memory and the
/// window reads stable.
#[test]
fn a_reader_that_reports_no_private_memory_reads_stable() {
    let result = explore_window(workload(false), &config());
    assert_eq!(result.total_branching_points, 1, "the two units race once");
    assert_eq!(result.outcome, OutcomeClass::ScheduleStable);
    assert_eq!(result.classes_explored, Some(2), "both orders ran");
}

#[test]
fn a_divergence_confined_to_private_memory_is_schedule_sensitive() {
    let result = explore_window(workload(true), &config());
    assert_eq!(result.classes_explored, Some(2), "both orders ran");
    assert_eq!(result.outcome, OutcomeClass::ScheduleSensitive);
}
