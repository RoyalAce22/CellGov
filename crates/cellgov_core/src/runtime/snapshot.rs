//! `RuntimeSnapshot`: data-only capture of every cloneable
//! [`Runtime`] field, used by [`Runtime::snapshot`] /
//! [`Runtime::restore_into`]. Which fields a restore leaves untouched
//! is decided in [`snapshot_field_categories`], the compile-time
//! destructure beside the struct.
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
//! 1. `restore_into` rewinds [`cellgov_time::Epoch`]; `Epoch` values
//!    held outside the runtime no longer index into anything live.
//!
//! 2. `restore_into` rewinds `per_step_index` and clears both
//!    `TraceWriter`s so post-restore records cannot collide with
//!    pre-snapshot records on the same index. Capture via
//!    [`cellgov_trace::TraceWriter::take_bytes`] before `restore_into`
//!    to keep the pre-restore trace.
//!
//! 3. The scheduler is not in the snapshot, but the runtime-side
//!    fields it consumes (`last_scheduled_unit`, `step_woke_others`)
//!    are restored. The caller must install a scheduler consistent
//!    with `snap` via [`Runtime::set_scheduler`]; until then
//!    [`Runtime::step`] returns
//!    [`super::StepError::SchedulerNotReinstalled`]. Only the presence
//!    of a fresh scheduler is checked.

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
/// step boundary; `snapshot_field_categories` lists what it leaves out.
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
    pub(super) timer_wakes: crate::timer_queue::TimerWakeQueue,
    pub(super) lv2_host: Lv2Host,
    pub(super) syscall_responses: SyscallResponseTable,
    pub(super) commit_pipeline: CommitPipeline,
    pub(super) memory: GuestMemory,
    pub(super) spaces: crate::runtime::spaces::SpaceTable,
    pub(super) time: GuestTicks,
    pub(super) epoch: Epoch,
    pub(super) steps_taken: usize,
    pub(super) last_scheduled_unit: Option<UnitId>,
    pub(super) step_woke_others: bool,
    pub(super) per_step_index: u64,
    pub(super) pending_tag_completions: std::collections::BTreeMap<UnitId, u32>,
    pub(super) pending_child_inits: Vec<crate::runtime::types::PendingChildInit>,
    pub(super) rsx_label_base: u32,

    /// Asserted unchanged in [`Runtime::restore_into`] (release-active),
    /// not restored.
    captured_budget_per_step: Budget,
    captured_max_steps: usize,
    captured_mode: RuntimeMode,
}

/// Every [`Runtime`] field, sorted into captured, asserted-unchanged,
/// and left alone by a restore. A field added to `Runtime` fails to
/// compile here until it is placed in one of the three groups.
fn snapshot_field_categories(rt: &Runtime) {
    let Runtime {
        // --- captured and restored ---
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
        timer_wakes: _,
        lv2_host: _,
        syscall_responses: _,
        commit_pipeline: _,
        memory: _,
        spaces: _,
        time: _,
        epoch: _,
        steps_taken: _,
        last_scheduled_unit: _,
        step_woke_others: _,
        per_step_index: _,
        pending_tag_completions: _,
        pending_child_inits: _,
        rsx_call_stack: _,
        rsx_consume_fifo: _,
        rsx_label_base: _,
        // --- captured for assert-unchanged, not restored ---
        budget_per_step: _,
        max_steps: _,
        mode: _,
        // --- left alone by a restore ---
        dma_latency: _,                   // set once at construction
        spu_factory: _,                   // set once at construction
        ppu_factory: _,                   // set once at construction
        process_spawn_loader: _,          // host-installed closure, like the factories
        scheduler: _,                     // caller reinstalls after restore
        trace: _,                         // cleared on restore
        zoom_trace: _,                    // cleared on restore
        effects_buf: _,                   // cleared on restore (per-step scratch)
        scheduler_dirty_after_restore: _, // set true by restore
        rsx_label_writes_committed: _,    // host-side counter only
        rsx_set_reference_dispatches: _,  // host-side counter only
        timer_sleep_dispatches: _,        // host-side counter only
        lv2_direct_committed_writes: _,   // staging-bypass witness, host-side only
    } = rt;
}

const _: fn(&Runtime) = snapshot_field_categories;

impl Runtime {
    /// Capture a deep clone of this runtime's mutable state. See
    /// the module doc for cost notes.
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
            timer_wakes: self.timer_wakes.clone(),
            lv2_host: self.lv2_host.clone(),
            syscall_responses: self.syscall_responses.clone(),
            commit_pipeline: self.commit_pipeline.clone(),
            memory: self.memory.clone(),
            spaces: self.spaces.clone(),
            time: self.time,
            epoch: self.epoch,
            steps_taken: self.steps_taken,
            last_scheduled_unit: self.last_scheduled_unit,
            step_woke_others: self.step_woke_others,
            per_step_index: self.per_step_index,
            pending_tag_completions: self.pending_tag_completions.clone(),
            pending_child_inits: self.pending_child_inits.clone(),
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
        self.timer_wakes = snap.timer_wakes.clone();
        self.lv2_host = snap.lv2_host.clone();
        self.syscall_responses = snap.syscall_responses.clone();
        self.commit_pipeline = snap.commit_pipeline.clone();
        self.memory = snap.memory.clone();
        self.spaces = snap.spaces.clone();
        self.time = snap.time;
        self.epoch = snap.epoch;
        self.steps_taken = snap.steps_taken;
        self.last_scheduled_unit = snap.last_scheduled_unit;
        self.step_woke_others = snap.step_woke_others;
        self.per_step_index = snap.per_step_index;
        self.pending_tag_completions = snap.pending_tag_completions.clone();
        self.pending_child_inits = snap.pending_child_inits.clone();
        self.rsx_label_base = snap.rsx_label_base;
        self.effects_buf.clear();
        self.trace.clear();
        self.zoom_trace.clear();
        self.scheduler_dirty_after_restore = true;
    }
}

#[cfg(test)]
#[path = "tests/snapshot_tests.rs"]
mod tests;
