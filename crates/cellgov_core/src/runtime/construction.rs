//! Runtime constructors extracted from `runtime.rs`.
//!
//! [`Runtime::new`] and [`Runtime::with_trace_writer`] set the default
//! field values for every registry, pipeline, and bookkeeping slot
//! the orchestrator owns. The body is mechanical but verbose --
//! moving it here keeps the facade focused on the step/commit
//! pipeline.

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

impl Runtime {
    /// Construct a runtime over the given memory, with the given
    /// per-step budget grant and the given max-steps cap. The
    /// scheduler starts at the beginning of the registry; time and
    /// epoch start at zero; no units are registered.
    ///
    /// Use [`Runtime::registry_mut`] to register units before stepping.
    pub fn new(memory: GuestMemory, budget_per_step: Budget, max_steps: usize) -> Self {
        Self::with_trace_writer(memory, budget_per_step, max_steps, TraceWriter::new())
    }

    /// Construct a runtime with a caller-supplied [`TraceWriter`].
    ///
    /// Used by tests and the testkit runner to install a writer with a
    /// specific level filter (for example, commits + hashes only) so the
    /// high-volume categories can be filtered, exercising that contract
    /// end-to-end. Behaviorally identical to [`Runtime::new`] otherwise.
    pub fn with_trace_writer(
        memory: GuestMemory,
        budget_per_step: Budget,
        max_steps: usize,
        trace: TraceWriter,
    ) -> Self {
        Self {
            registry: UnitRegistry::new(),
            mailbox_registry: MailboxRegistry::new(),
            signal_registry: SignalRegistry::new(),
            reservations: cellgov_sync::ReservationTable::new(),
            rsx_cursor: crate::rsx::RsxFifoCursor::new(),
            rsx_sem_offset: 0,
            rsx_mirror_writes: false,
            rsx_flip: crate::rsx::flip::RsxFlipState::new(),
            rsx_methods: {
                let mut t = crate::rsx::method::NvMethodTable::new();
                crate::rsx::method::register_nv406e_label_handlers(&mut t)
                    .expect("fresh NvMethodTable cannot collide");
                crate::rsx::method::register_nv406e_reference_handler(&mut t)
                    .expect("fresh NvMethodTable cannot collide");
                crate::rsx::method::register_nv4097_flip_handler(&mut t)
                    .expect("fresh NvMethodTable cannot collide");
                crate::rsx::method::register_nv4097_report_handler(&mut t)
                    .expect("fresh NvMethodTable cannot collide");
                crate::rsx::method::register_nv4097_back_end_semaphore_handlers(&mut t)
                    .expect("fresh NvMethodTable cannot collide");
                t
            },
            pending_rsx_effects: Vec::new(),
            dma_queue: DmaQueue::new(),
            dma_latency: Box::new(FixedLatency::new(10)),
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
            effects_buf: Vec::new(),
            hle: crate::hle::HleState::new(),
            mode: RuntimeMode::FullTrace,
            per_step_index: 0,
            zoom_trace: TraceWriter::new(),
        }
    }
}
