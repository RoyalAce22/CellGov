//! Runtime constructors.

use cellgov_dma::{DmaQueue, FixedLatency};
use cellgov_lv2::Lv2Host;
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, Epoch, GuestTicks};
use cellgov_trace::TraceWriter;

use crate::commit::CommitPipeline;
use crate::registry::UnitRegistry;
use crate::scheduler::RoundRobinScheduler;
use crate::syscall_table::SyscallResponseTable;

use super::{Runtime, RuntimeMode};

/// Default DMA completion latency in guest ticks under the
/// workspace [`FixedLatency`] model. Folds into per-tag completion
/// timing and the cross-runner parity verdict.
pub const DEFAULT_DMA_LATENCY_TICKS: GuestTicks = GuestTicks::new(10);

impl Runtime {
    /// Construct a runtime with a default [`TraceWriter`].
    ///
    /// Time, epoch, and `steps_taken` start at zero; no units are
    /// registered.
    ///
    /// # Non-obvious defaults
    ///
    /// - DMA latency: a fixed default via [`FixedLatency`].
    /// - Runtime mode: [`RuntimeMode::FullTrace`]; the trivial-step
    ///   fast path is `FaultDriven`-only.
    /// - NV method roster:
    ///   [`crate::rsx::method::NvMethodTable::with_default_handlers`].
    ///
    /// # Zero values
    ///
    /// `max_steps == 0` makes the first [`Runtime::step`] return
    /// `Err(StepError::MaxStepsExceeded)`. `Budget::ZERO` stalls
    /// without retiring work. Reject zero at the CLI boundary if a
    /// subcommand requires non-zero.
    pub fn new(memory: GuestMemory, budget_per_step: Budget, max_steps: usize) -> Self {
        Self::with_trace_writer(memory, budget_per_step, max_steps, TraceWriter::new())
    }

    /// Like [`Runtime::new`] but takes a caller-supplied
    /// [`TraceWriter`]. The zoom trace inherits the main trace's
    /// level filter via [`TraceWriter::clone`].
    pub fn with_trace_writer(
        memory: GuestMemory,
        budget_per_step: Budget,
        max_steps: usize,
        trace: TraceWriter,
    ) -> Self {
        let zoom_trace = {
            let mut t = trace.clone();
            t.clear();
            t
        };
        Self {
            registry: UnitRegistry::new(),
            mailbox_registry: MailboxRegistry::new(),
            signal_registry: SignalRegistry::new(),
            reservations: cellgov_sync::ReservationTable::new(),
            rsx_cursor: crate::rsx::RsxFifoCursor::new(),
            rsx_sem_offset: 0,
            rsx_mirror_writes: false,
            rsx_flip: crate::rsx::flip::RsxFlipState::new(),
            rsx_methods: crate::rsx::method::NvMethodTable::with_default_handlers(),
            pending_rsx_effects: Vec::new(),
            dma_queue: DmaQueue::new(),
            dma_latency: Box::new(FixedLatency::new(DEFAULT_DMA_LATENCY_TICKS.raw())),
            lv2_host: Lv2Host::new(),
            syscall_responses: SyscallResponseTable::new(),
            spu_factory: None,
            ppu_factory: None,
            scheduler: Box::new(RoundRobinScheduler::new()),
            commit_pipeline: CommitPipeline::new(),
            memory,
            time: GuestTicks::ZERO,
            epoch: Epoch::ZERO,
            budget_per_step,
            steps_taken: 0,
            max_steps,
            trace,
            last_scheduled_unit: None,
            step_woke_others: false,
            effects_buf: Vec::new(),
            rsx_label_base: 0,
            mode: RuntimeMode::FullTrace,
            per_step_index: 0,
            zoom_trace,
            scheduler_dirty_after_restore: false,
            pending_tag_completions: std::collections::BTreeMap::new(),
            rsx_label_writes_committed: 0,
            rsx_set_reference_dispatches: 0,
            rsx_call_stack: crate::rsx::RsxCallStack::new(),
            rsx_consume_fifo: false,
            lv2_direct_committed_writes: 0,
        }
    }
}

#[cfg(test)]
#[path = "tests/construction_tests.rs"]
mod tests;
