//! [`Runtime`] state struct -- the step-loop and commit-pipeline
//! owner. Per-method impls live in sibling submodules.

use cellgov_dma::{DmaLatencyModel, DmaQueue};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_lv2::Lv2Host;
use cellgov_mem::GuestMemory;
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, Epoch, GuestTicks};
use cellgov_trace::TraceWriter;

use crate::commit::CommitPipeline;
use crate::registry::UnitRegistry;
use crate::runtime::types::{PpuFactory, RuntimeMode, SpuFactory};
use crate::scheduler::Scheduler;
use crate::syscall_table::SyscallResponseTable;

/// Deterministic step-loop runtime over guest memory and registered units.
pub struct Runtime {
    pub(crate) registry: UnitRegistry,
    pub(super) mailbox_registry: MailboxRegistry,
    pub(super) signal_registry: SignalRegistry,
    pub(super) reservations: cellgov_sync::ReservationTable,
    pub(super) rsx_cursor: crate::rsx::RsxFifoCursor,
    /// Persists across commit boundaries: an OFFSET / RELEASE pair may
    /// straddle drains and the later RELEASE must read the earlier OFFSET.
    pub(super) rsx_sem_offset: u32,
    /// Host must make the RSX region writable before enabling; otherwise
    /// every put-pointer store reserved-writes and the mirror never runs.
    pub(super) rsx_mirror_writes: bool,
    pub(super) rsx_flip: crate::rsx::flip::RsxFlipState,
    pub(super) rsx_methods: crate::rsx::method::NvMethodTable,
    /// Advance-pass effects produced at the end of commit batch N, queued
    /// for the start of batch N+1. FIFO method parses mutate cursor +
    /// sem_offset in batch N; downstream memory / state effects commit
    /// alongside batch N+1 to preserve the atomic-batch contract.
    pub(super) pending_rsx_effects: Vec<Effect>,
    pub(super) dma_queue: DmaQueue,
    pub(super) dma_latency: Box<dyn DmaLatencyModel>,
    pub(super) lv2_host: Lv2Host,
    pub(super) syscall_responses: SyscallResponseTable,
    pub(super) spu_factory: Option<SpuFactory>,
    pub(super) ppu_factory: Option<PpuFactory>,
    pub(super) scheduler: Box<dyn Scheduler>,
    pub(super) commit_pipeline: CommitPipeline,
    pub(crate) memory: GuestMemory,
    pub(super) time: GuestTicks,
    pub(super) epoch: Epoch,
    pub(super) budget_per_step: Budget,
    pub(super) steps_taken: usize,
    pub(super) max_steps: usize,
    pub(super) trace: TraceWriter,
    /// One commit batch per unit yield; attributes the batch to its source.
    pub(super) last_scheduled_unit: Option<UnitId>,
    /// True when the just-completed step's dispatch transitioned at least
    /// one other unit into `Runnable`. Surfaced via `notify_yielded` so
    /// scheduler policy can distinguish wake-causing syscalls (`sema_post`,
    /// `event_flag_set`) from non-waking ones (`tty_write`,
    /// `ppu_thread_get_id`).
    pub(super) step_woke_others: bool,
    /// Base address used by `RsxLabelWrite` effects when computing the
    /// commit-side guest address (`base + offset`). Zero means RSX has
    /// not allocated label memory yet; synthetic scenarios set it
    /// directly via [`Runtime::set_rsx_label_base`].
    pub(super) rsx_label_base: u32,
    pub(super) effects_buf: Vec<Effect>,
    pub(super) mode: RuntimeMode,
    /// Monotonic over per-instruction state hashes; orthogonal to
    /// `steps_taken`, which counts `run_until_yield` invocations.
    pub(super) per_step_index: u64,
    pub(super) zoom_trace: TraceWriter,
    /// Set by [`Runtime::restore_into`], cleared by
    /// [`Runtime::set_scheduler`]; [`Runtime::step`] debug-panics
    /// if it sees this set. Catches stepping with a scheduler whose
    /// internal sticky-streak / last-position state was carried over
    /// from before the restore.
    pub(super) scheduler_dirty_after_restore: bool,
}
