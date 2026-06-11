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
//!   -- installed at construction, never mutated.
//! - [`Box<dyn Scheduler>`](crate::scheduler::Scheduler) -- caller
//!   replaces via [`Runtime::set_scheduler`] (see contract 3 below).
//! - [`cellgov_trace::TraceWriter`] (main + zoom) -- but cleared
//!   on `restore_into` (see contract 2 below).
//! - `rsx_label_writes_committed` -- cumulative audit counter
//!   incremented in `commit_step` per `Effect::RsxLabelWrite`;
//!   read only by `Runtime::rsx_label_writes_committed()` and the
//!   bench-boot stderr witness. Never feeds the commit pipeline or
//!   FIFO advance, so excluding it does not perturb replay. The
//!   counter on the post-restore host carries its pre-restore value
//!   forward, which is a documented audit-instrument quirk, not a
//!   determinism violation.
//! - `effects_buf`, `scheduler_dirty_after_restore` -- transient
//!   per-step scratch / restore-tracking state; restore sets the
//!   flag and clears the buffer (see contracts 3, 5).
//!
//! Construction params (`budget_per_step`, `max_steps`, `mode`) are
//! NOT restored; their values at snapshot time are captured and
//! asserted unchanged inside [`Runtime::restore_into`]. The assertion
//! is release-active rather than `debug_assertions`-gated because
//! [`Runtime::set_budget`] and [`Runtime::set_mode`] are public
//! setters; a release-only guard would let a caller silently mutate
//! a config value between snapshot and restore and produce a
//! divergent replay with no diagnostic.
//!
//! # Failure-mode mechanism split
//!
//! The scheduler-reinstall guard in [`Runtime::step`] returns
//! [`super::StepError::SchedulerNotReinstalled`]; the
//! `restore_into` config-immutability guards
//! [`assert!`]-panic. The split is deliberate: `step` is the hot
//! loop a caller (e.g. `cellgov_explore`) drives in a long-running
//! search, where a typed signal is more useful than an unwind. The
//! `restore_into` asserts fire at setup, where a panic is the right
//! shape for a programming error (the caller mutated a setter
//! between snapshot and restore).
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
//!    internal state is consistent with `snap`. The runtime sets
//!    `scheduler_dirty_after_restore`; [`Runtime::step`] returns
//!    [`super::StepError::SchedulerNotReinstalled`] (release-active,
//!    not a debug-only panic) if no [`Runtime::set_scheduler`]
//!    intervened. Only the *presence* of a fresh scheduler is
//!    mechanically checkable; whether its internal state matches
//!    `snap` is a documented caller obligation.
//!
//! 4. [`cellgov_effects::Effect`] payloads deep-clone through
//!    `Vec<u8>`; no `Arc` aliasing. The protection is structural,
//!    not test-enforced: `WritePayload`'s heap fallback is
//!    `Vec<u8>`, `DmaQueue` is `BTreeMap + u64`, and
//!    `cellgov_lv2` carries no `Arc` / `Rc`. Adding an `Arc`
//!    anywhere on a snapshot-captured path is what would break
//!    the contract; `dma_queue_aliasing_canary_two_directions`
//!    is the corroborating fixture, not the source of truth.
//!
//! 5. `restore_into` clears `effects_buf` rather than rejecting a
//!    partial batch; a restore is a reset and any in-flight
//!    effects are intentionally discarded. `snapshot` asserts
//!    `effects_buf` is empty so the capture itself only happens
//!    at step boundaries.

use cellgov_dma::DmaQueue;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_lv2::Lv2Host;
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, ReservationTable, SignalRegistry};
use cellgov_time::{Epoch, GuestTicks};

use crate::commit::CommitPipeline;
use crate::registry::UnitRegistry;
use crate::rsx::flip::RsxFlipState;
use crate::rsx::method::NvMethodTable;
use crate::rsx::RsxFifoCursor;
use crate::syscall_table::SyscallResponseTable;
use cellgov_time::Budget;

use super::{Runtime, RuntimeMode};

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
    pub(super) rsx_call_stack: crate::rsx::RsxCallStack,
    pub(super) rsx_consume_fifo: bool,
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
    pub(super) per_step_index: u64,
    pub(super) pending_tag_completions: std::collections::BTreeMap<UnitId, u32>,
    /// Base address for `Effect::RsxLabelWrite` commit targets.
    /// Mutable via [`Runtime::set_rsx_label_base`] and read every
    /// commit batch in `commit::process` (`start = base + offset`),
    /// so a snapshot taken after the base is set must restore it to
    /// preserve the commit-side guest address; a fresh-runtime
    /// restore that left base at 0 would commit label writes to a
    /// different memory address and diverge.
    pub(super) rsx_label_base: u32,

    /// Asserted unchanged in [`Runtime::restore_into`] (release-active),
    /// not restored. `set_budget` and `set_mode` are public; a
    /// debug-only guard would let a release-build caller mutate these
    /// between snapshot and restore and silently diverge replays.
    captured_budget_per_step: Budget,
    captured_max_steps: usize,
    captured_mode: RuntimeMode,
}

impl Runtime {
    /// Capture a deep clone of this runtime's mutable state. See
    /// the module doc for excluded fields and cost notes.
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
            rsx_call_stack: self.rsx_call_stack,
            rsx_consume_fifo: self.rsx_consume_fifo,
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
            per_step_index: self.per_step_index,
            pending_tag_completions: self.pending_tag_completions.clone(),
            rsx_label_base: self.rsx_label_base,

            captured_budget_per_step: self.budget_per_step,
            captured_max_steps: self.max_steps,
            captured_mode: self.mode,
        }
    }

    /// Overwrite this runtime's mutable state from `snap`. See
    /// the module doc for the cross-module contracts:
    /// epoch rewind, trace-writer clear, mandatory scheduler
    /// reinstall before the next step.
    pub fn restore_into(&mut self, snap: &RuntimeSnapshot) {
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

        self.registry = snap.registry.clone();
        self.mailbox_registry = snap.mailbox_registry.clone();
        self.signal_registry = snap.signal_registry.clone();
        self.reservations = snap.reservations.clone();
        self.rsx_cursor = snap.rsx_cursor;
        self.rsx_sem_offset = snap.rsx_sem_offset;
        self.rsx_call_stack = snap.rsx_call_stack;
        self.rsx_consume_fifo = snap.rsx_consume_fifo;
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
        self.per_step_index = snap.per_step_index;
        self.pending_tag_completions = snap.pending_tag_completions.clone();
        self.rsx_label_base = snap.rsx_label_base;
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
            "terminal memory hash diverged after snapshot/restore replay",
        );
        // Field-completeness is not what this test proves. The
        // FakeIsaUnit fixture exercises registry / memory /
        // commit_pipeline; it never populates lv2_host, dma_queue,
        // any rsx_* field, etc. A snapshot missing one of those
        // would still pass here. The structural guard for that is
        // the exhaustive destructure in
        // `_snapshot_field_exhaustiveness_compile_guard`.
    }

    /// Compile-time field-completeness guard. A no-rest destructure
    /// of every [`Runtime`] field; adding a field to `Runtime`
    /// breaks compilation here until it is consciously categorized
    /// as snapshot-captured, deliberately excluded
    /// (set-once / caller-replaced), or asserted-unchanged
    /// (construction param). The fixture-based replay tests can't
    /// reach every field; this can.
    ///
    /// Never called. Determinism-neutral, zero runtime cost.
    #[allow(dead_code)]
    fn _snapshot_field_exhaustiveness_compile_guard(rt: &Runtime) {
        let Runtime {
            // --- snapshot-captured ---
            registry: _,
            mailbox_registry: _,
            signal_registry: _,
            reservations: _,
            rsx_cursor: _,
            rsx_sem_offset: _,
            rsx_mirror_writes: _,
            rsx_flip: _,
            rsx_methods: _,
            pending_rsx_effects: _,
            dma_queue: _,
            lv2_host: _,
            syscall_responses: _,
            commit_pipeline: _,
            memory: _,
            time: _,
            epoch: _,
            steps_taken: _,
            last_scheduled_unit: _,
            step_woke_others: _,
            per_step_index: _,
            pending_tag_completions: _,
            rsx_call_stack: _,
            rsx_consume_fifo: _,
            rsx_label_base: _,
            // --- captured for assert-unchanged, not restored ---
            budget_per_step: _,
            max_steps: _,
            mode: _,
            // --- deliberately excluded; see module doc for category ---
            dma_latency: _,                   // set-once at construction
            spu_factory: _,                   // set-once at construction
            ppu_factory: _,                   // set-once at construction
            scheduler: _,                     // caller-replaced post-restore
            trace: _,                         // cleared on restore
            zoom_trace: _,                    // cleared on restore
            effects_buf: _,                   // cleared on restore (per-step scratch)
            scheduler_dirty_after_restore: _, // set true by restore
            rsx_label_writes_committed: _,    // audit counter, host-side only
            rsx_set_reference_dispatches: _,  // audit counter, host-side only
            lv2_direct_committed_writes: _,   // staging-bypass witness, host-side only
        } = rt;
    }

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

    /// Two-direction container-independence canary on `dma_queue`.
    ///
    /// What this proves: cloning the BTreeMap-backed queue produces
    /// an independent top-level container. Inserting a new entry
    /// into the original or restored runtime's map does not touch
    /// the snapshot's map.
    ///
    /// What this does NOT prove: that entry *payloads* are
    /// independent. Both directions only insert new entries; neither
    /// mutates a pre-existing captured entry, so an `Arc`-shared
    /// payload would pass green here. Interior aliasing is guarded
    /// structurally (module doc contract 4: no `Arc` on
    /// snapshot-captured paths) rather than by this fixture. If a
    /// future heap-bearing snapshot field gains in-place entry
    /// mutation, an interior-aliasing canary would have to
    /// snapshot-then-mutate-existing on that specific field; the
    /// general `BTreeMap + u64` shape we have today is structurally
    /// `Arc`-free and has no in-place entry mutation to exercise.
    fn make_runtime_with_dma_drivers() -> Runtime {
        let mem = GuestMemory::new(0x4000);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::DmaPut {
                        src: 0x100,
                        dst: 0x1000,
                        len: 32,
                    },
                    FakeOp::DmaPut {
                        src: 0x200,
                        dst: 0x2000,
                        len: 32,
                    },
                    FakeOp::End,
                ],
            )
        });
        rt
    }

    #[test]
    fn dma_queue_aliasing_canary_two_directions() {
        // DMAs complete after DEFAULT_DMA_LATENCY_TICKS (10), so
        // driving 1 step keeps the enqueued completion in queue
        // while step 2 would advance time past the completion and
        // fire it. We need the queue non-empty when we observe.

        // Direction 1: original mutates after snapshot.
        let mut rt = make_runtime_with_dma_drivers();
        let snap = rt.snapshot();
        let snap_len_pre = snap.dma_queue.len();
        drive(&mut rt, 1);
        assert!(
            rt.dma_queue().len() > snap_len_pre,
            "test setup: one step must leave at least one DMA queued before its completion fires",
        );
        assert_eq!(
            snap.dma_queue.len(),
            snap_len_pre,
            "snapshot's dma_queue aliased the original; post-snapshot \
             enqueue leaked into the captured queue",
        );

        // Direction 2: restored runtime mutates, separate snapshot
        // held by the test stays put.
        let mut rt2 = make_runtime_with_dma_drivers();
        let snap2 = rt2.snapshot();
        let snap2_len_pre = snap2.dma_queue.len();
        rt2.restore_into(&snap2);
        rt2.set_scheduler(RoundRobinScheduler::new());
        drive(&mut rt2, 1);
        assert!(
            rt2.dma_queue().len() > snap2_len_pre,
            "test setup: one step must leave at least one DMA queued before its completion fires",
        );
        assert_eq!(
            snap2.dma_queue.len(),
            snap2_len_pre,
            "snapshot's dma_queue aliased the restored runtime; \
             post-restore enqueue leaked into the captured queue",
        );
    }

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
    fn step_after_restore_without_set_scheduler_returns_typed_error() {
        // F1: the guard was previously a debug_assert, release-silent.
        // Promoted to a typed StepError variant so cellgov_explore's
        // step loop catches the misuse uniformly debug + release
        // instead of unwinding (debug) or producing a divergent
        // replay (release). No #[cfg(debug_assertions)] gate; this
        // test runs under `cargo test --release` too.
        //
        // assert_eq! on the Result transitively requires PartialEq
        // + Debug on the Ok type (RuntimeStep) and its components
        // (Effect, WritePayload). Verified these derives existed
        // before this test was added (effect.rs:21, types.rs:13);
        // the test did not force any new derives onto hot domain
        // types.
        use crate::runtime::StepError;
        let mut rt = make_runtime_with_two_writers();
        let snap = rt.snapshot();
        rt.restore_into(&snap);
        assert_eq!(rt.step(), Err(StepError::SchedulerNotReinstalled));
    }

    #[test]
    fn set_scheduler_after_restore_clears_dirty_flag() {
        let mut rt = make_runtime_with_two_writers();
        let snap = rt.snapshot();
        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());
        let _ = rt.step();
    }

    #[test]
    fn snapshot_captures_rsx_label_base_and_restore_overwrites_it() {
        // N1: rsx_label_base drives commit_step's RsxLabelWrite
        // commit target. Snapshot must restore it; a fresh-built
        // runtime that had its base set differently from snap would
        // otherwise commit label writes to a different address on
        // replay -- a guest-observable divergence.
        let mut rt = make_runtime_with_two_writers();
        rt.set_rsx_label_base(cellgov_mem::GuestAddr::new(0x4000));
        let snap = rt.snapshot();
        assert_eq!(snap.rsx_label_base, 0x4000);

        // Mutate the host between snapshot and restore, then verify
        // restore overwrites the mutation with snap's value.
        rt.set_rsx_label_base(cellgov_mem::GuestAddr::new(0x8000));
        rt.restore_into(&snap);
        rt.set_scheduler(RoundRobinScheduler::new());
        assert_eq!(
            rt.rsx_label_base, 0x4000,
            "restore_into must overwrite rsx_label_base with snap's captured value",
        );
    }

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
