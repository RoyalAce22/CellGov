//! `RuntimeSnapshot`: data-only capture of every cloneable
//! [`Runtime`] field, used by [`Runtime::snapshot`] /
//! [`Runtime::restore_into`].
//!
//! # Excluded fields
//!
//! Set-once or caller-replaced; the host `Runtime` retains them
//! across restores:
//!
//! - `spu_factory`, `ppu_factory`, [`cellgov_dma::DmaLatencyModel`]
//! - [`Box<dyn Scheduler>`](crate::scheduler::Scheduler) -- caller
//!   replaces via [`Runtime::set_scheduler`] (see contract 3 below)
//! - [`cellgov_trace::TraceWriter`] (main + zoom) -- but cleared
//!   on `restore_into` (see contract 2 below)
//!
//! Construction params (`budget_per_step`, `max_steps`, `mode`) are
//! NOT restored; their values at snapshot time are captured under
//! `cfg(debug_assertions)` and asserted unchanged inside
//! [`Runtime::restore_into`].
//!
//! # Cost
//!
//! Dominated by [`cellgov_mem::GuestMemory`] (COW: cheap until a
//! post-snapshot write forks the touched region) and each
//! `PpuExecutionUnit`'s predecoded shadow (deep-cloned per
//! branching point).
//!
//! # Cross-module contracts
//!
//! 1. `restore_into` rewinds [`cellgov_time::Epoch`]. Stale `Epoch`
//!    values held outside the runtime no longer index into anything
//!    live (DMA completion times, join wakes, RSX flip waiters).
//!
//! 2. `restore_into` rewinds `per_step_index` AND clears both
//!    `TraceWriter`s via [`TraceWriter::clear`]. Without the clear,
//!    post-restore records would collide with pre-snapshot records
//!    on the same `per_step_index`. Callers that want the
//!    pre-restore trace must capture via [`TraceWriter::take_bytes`]
//!    before `restore_into`.
//!
//! 3. The scheduler is NOT in the snapshot, but the runtime-side
//!    fields it consumes (`last_scheduled_unit`, `step_woke_others`)
//!    ARE restored. The caller MUST install a scheduler whose
//!    internal state is consistent with `snap`; the runtime sets
//!    `scheduler_dirty_after_restore` and `Runtime::step`
//!    debug-panics if no [`Runtime::set_scheduler`] intervened.
//!
//! 4. [`cellgov_effects::Effect`] payloads deep-clone through
//!    `Vec<u8>`; no `Arc` aliasing.

use cellgov_dma::DmaQueue;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_lv2::Lv2Host;
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, ReservationTable, SignalRegistry};
use cellgov_time::{Epoch, GuestTicks};

use crate::commit::CommitPipeline;
use crate::hle::HleState;
use crate::registry::UnitRegistry;
use crate::rsx::flip::RsxFlipState;
use crate::rsx::method::NvMethodTable;
use crate::rsx::RsxFifoCursor;
use crate::syscall_table::SyscallResponseTable;

#[cfg(debug_assertions)]
use cellgov_time::Budget;

use super::Runtime;
#[cfg(debug_assertions)]
use super::RuntimeMode;

/// Cloneable capture of a [`Runtime`]'s mutable state at a single
/// step boundary. See module doc for excluded fields.
#[derive(Clone)]
pub struct RuntimeSnapshot {
    pub(super) registry: UnitRegistry,
    pub(super) mailbox_registry: MailboxRegistry,
    pub(super) signal_registry: SignalRegistry,
    pub(super) reservations: ReservationTable,
    pub(super) rsx_cursor: RsxFifoCursor,
    pub(super) rsx_sem_offset: u32,
    pub(super) rsx_mirror_writes: bool,
    pub(super) rsx_flip: RsxFlipState,
    pub(super) rsx_methods: NvMethodTable,
    pub(super) pending_rsx_effects: Vec<Effect>,
    pub(super) dma_queue: DmaQueue,
    pub(super) lv2_host: Lv2Host,
    pub(super) syscall_responses: SyscallResponseTable,
    pub(super) commit_pipeline: CommitPipeline,
    pub(super) memory: GuestMemory,
    pub(super) time: GuestTicks,
    pub(super) epoch: Epoch,
    pub(super) steps_taken: usize,
    pub(super) last_scheduled_unit: Option<UnitId>,
    pub(super) step_woke_others: bool,
    pub(super) hle: HleState,
    pub(super) per_step_index: u64,

    /// Asserted unchanged in [`Runtime::restore_into`], not restored.
    #[cfg(debug_assertions)]
    captured_budget_per_step: Budget,
    #[cfg(debug_assertions)]
    captured_max_steps: usize,
    #[cfg(debug_assertions)]
    captured_mode: RuntimeMode,
}

impl Runtime {
    /// Capture a deep clone of this runtime's mutable state. See
    /// the [module doc](self) for excluded fields and cost notes.
    ///
    /// # Panics
    ///
    /// Debug-asserts `effects_buf` is empty: snapshots taken
    /// mid-batch would diverge from a fresh-built runtime on restore.
    pub fn snapshot(&self) -> RuntimeSnapshot {
        debug_assert!(
            self.effects_buf.is_empty(),
            "Runtime::snapshot taken with non-empty effects_buf ({} effects); \
             snapshots must be taken at step boundaries",
            self.effects_buf.len()
        );
        RuntimeSnapshot {
            registry: self.registry.clone(),
            mailbox_registry: self.mailbox_registry.clone(),
            signal_registry: self.signal_registry.clone(),
            reservations: self.reservations.clone(),
            rsx_cursor: self.rsx_cursor,
            rsx_sem_offset: self.rsx_sem_offset,
            rsx_mirror_writes: self.rsx_mirror_writes,
            rsx_flip: self.rsx_flip,
            rsx_methods: self.rsx_methods.clone(),
            pending_rsx_effects: self.pending_rsx_effects.clone(),
            dma_queue: self.dma_queue.clone(),
            lv2_host: self.lv2_host.clone(),
            syscall_responses: self.syscall_responses.clone(),
            commit_pipeline: self.commit_pipeline.clone(),
            memory: self.memory.clone(),
            time: self.time,
            epoch: self.epoch,
            steps_taken: self.steps_taken,
            last_scheduled_unit: self.last_scheduled_unit,
            step_woke_others: self.step_woke_others,
            hle: self.hle.clone(),
            per_step_index: self.per_step_index,

            #[cfg(debug_assertions)]
            captured_budget_per_step: self.budget_per_step,
            #[cfg(debug_assertions)]
            captured_max_steps: self.max_steps,
            #[cfg(debug_assertions)]
            captured_mode: self.mode,
        }
    }

    /// Overwrite this runtime's mutable state from `snap`. See
    /// the [module doc](self) for the cross-module contracts:
    /// epoch rewind, trace-writer clear, mandatory scheduler
    /// reinstall before the next step.
    pub fn restore_into(&mut self, snap: &RuntimeSnapshot) {
        #[cfg(debug_assertions)]
        {
            assert_eq!(
                self.budget_per_step, snap.captured_budget_per_step,
                "restore_into: budget_per_step changed since snapshot; \
                 rebuild the runtime instead of restoring"
            );
            assert_eq!(
                self.max_steps, snap.captured_max_steps,
                "restore_into: max_steps changed since snapshot; \
                 rebuild the runtime instead of restoring"
            );
            assert_eq!(
                self.mode, snap.captured_mode,
                "restore_into: mode changed since snapshot; \
                 rebuild the runtime instead of restoring"
            );
        }

        self.registry = snap.registry.clone();
        self.mailbox_registry = snap.mailbox_registry.clone();
        self.signal_registry = snap.signal_registry.clone();
        self.reservations = snap.reservations.clone();
        self.rsx_cursor = snap.rsx_cursor;
        self.rsx_sem_offset = snap.rsx_sem_offset;
        self.rsx_mirror_writes = snap.rsx_mirror_writes;
        self.rsx_flip = snap.rsx_flip;
        self.rsx_methods = snap.rsx_methods.clone();
        self.pending_rsx_effects = snap.pending_rsx_effects.clone();
        self.dma_queue = snap.dma_queue.clone();
        self.lv2_host = snap.lv2_host.clone();
        self.syscall_responses = snap.syscall_responses.clone();
        self.commit_pipeline = snap.commit_pipeline.clone();
        self.memory = snap.memory.clone();
        self.time = snap.time;
        self.epoch = snap.epoch;
        self.steps_taken = snap.steps_taken;
        self.last_scheduled_unit = snap.last_scheduled_unit;
        self.step_woke_others = snap.step_woke_others;
        self.hle = snap.hle.clone();
        self.per_step_index = snap.per_step_index;
        // clear() not clone(): preserves allocator capacity across
        // repeated restores.
        self.effects_buf.clear();
        self.trace.clear();
        self.zoom_trace.clear();
        self.scheduler_dirty_after_restore = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::RoundRobinScheduler;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn make_runtime_with_two_writers() -> Runtime {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xAA),
                    FakeOp::SharedStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0xBB),
                    FakeOp::SharedStore { addr: 8, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt
    }

    fn drive(rt: &mut Runtime, n: usize) {
        for _ in 0..n {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                }
                Err(_) => break,
            }
        }
    }

    /// Catches init-time-only fields missing from `RuntimeSnapshot`.
    /// Drift-during-execution coverage:
    /// [`snapshot_after_execution_restores_byte_identical_state`].
    #[test]
    fn snapshot_then_restore_replays_to_same_terminal_state() {
        let mut rt = make_runtime_with_two_writers();
        let snap = rt.snapshot();

        drive(&mut rt, 50);
        let original_hash = rt.memory().content_hash();

        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());
        drive(&mut rt, 50);
        let restored_hash = rt.memory().content_hash();

        assert_eq!(
            original_hash, restored_hash,
            "memory hash differs after snapshot/restore replay -- \
             a Runtime field is missing from RuntimeSnapshot"
        );
    }

    /// Catches counter-style fields that drift during execution
    /// being missed from `RuntimeSnapshot` -- the construction-time
    /// replay test above re-derives such fields from a shared
    /// starting point and cannot see them.
    #[test]
    fn snapshot_after_execution_restores_byte_identical_state() {
        let mut rt = make_runtime_with_two_writers();
        drive(&mut rt, 3);

        let snap = rt.snapshot();
        let pre_mem = rt.memory().content_hash();
        let pre_steps = rt.steps_taken();
        let pre_epoch_raw = rt.epoch();
        let pre_per_step = rt.per_step_index_for_tests();

        drive(&mut rt, 5);
        assert_ne!(
            rt.memory().content_hash(),
            pre_mem,
            "test setup: post-snapshot driving must mutate state",
        );

        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());

        assert_eq!(
            rt.memory().content_hash(),
            pre_mem,
            "memory drifted across restore"
        );
        assert_eq!(
            rt.steps_taken(),
            pre_steps,
            "steps_taken drifted across restore"
        );
        assert_eq!(rt.epoch(), pre_epoch_raw, "epoch drifted across restore");
        assert_eq!(
            rt.per_step_index_for_tests(),
            pre_per_step,
            "per_step_index drifted across restore -- snapshot missed it",
        );
    }

    /// Per-unit predecoded shadow non-aliasing is pinned in
    /// `crates/cellgov_ppu/tests/snapshot_shadow_independence.rs`.
    #[test]
    fn snapshot_memory_is_independent_of_post_snapshot_mutation() {
        let mut rt = make_runtime_with_two_writers();
        let snap = rt.snapshot();
        let snap_hash_before = snap.memory.content_hash();

        drive(&mut rt, 5);
        assert_ne!(
            rt.memory().content_hash(),
            snap_hash_before,
            "test setup: original must mutate to validate snapshot independence",
        );

        assert_eq!(
            snap.memory.content_hash(),
            snap_hash_before,
            "snapshot memory aliased the original -- post-snapshot mutation leaked",
        );
    }

    /// Pins that `restore_into` clears `effects_buf` rather than
    /// cloning-and-replacing, so repeated restores don't churn
    /// the allocator.
    #[test]
    fn repeated_restore_preserves_effects_buf_capacity() {
        let mut rt = make_runtime_with_two_writers();
        rt.effects_buf_mut_for_tests().reserve(128);
        let pre_capacity = rt.effects_buf_capacity_for_tests();
        assert!(pre_capacity >= 128);

        let snap = rt.snapshot();
        for _ in 0..4 {
            rt.restore_into(&snap);
            rt.set_scheduler(RoundRobinScheduler::new());
        }
        let post_capacity = rt.effects_buf_capacity_for_tests();
        assert_eq!(
            pre_capacity, post_capacity,
            "effects_buf capacity changed across restores ({pre_capacity} -> {post_capacity})",
        );
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "between restore_into and set_scheduler")]
    fn step_after_restore_without_set_scheduler_panics() {
        let mut rt = make_runtime_with_two_writers();
        let snap = rt.snapshot();
        rt.restore_into(&snap);
        let _ = rt.step();
    }

    #[test]
    fn set_scheduler_after_restore_clears_dirty_flag() {
        let mut rt = make_runtime_with_two_writers();
        let snap = rt.snapshot();
        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());
        let _ = rt.step();
    }

    /// Without the clear, post-restore records would collide with
    /// pre-snapshot records on the same `per_step_index`.
    #[test]
    fn restore_into_clears_trace_writers() {
        let mut rt = make_runtime_with_two_writers();
        drive(&mut rt, 1);
        let snap = rt.snapshot();
        drive(&mut rt, 2);
        assert!(
            rt.trace().record_count() > 0,
            "test setup: post-snapshot stepping must produce trace records",
        );

        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());

        assert_eq!(
            rt.trace().record_count(),
            0,
            "main trace writer was not cleared on restore_into",
        );
        assert_eq!(
            rt.zoom_trace().record_count(),
            0,
            "zoom trace writer was not cleared on restore_into",
        );
    }
}
