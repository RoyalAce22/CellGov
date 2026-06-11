//! Pure getters, setters, and field-shaped accessors over [`Runtime`].
//!
//! No business logic -- entries here are field plumbing only. Methods
//! that compute over multiple fields (e.g. `sync_state_hash`) live in
//! sibling submodules (`state_hash`, `step`, `commit_step`, etc.).

use cellgov_dma::DmaQueue;
#[cfg(test)]
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_lv2::{Lv2Host, PpuThreadInitState, SpuInitState};
use cellgov_mem::{GuestAddr, GuestMemory};
use cellgov_sync::{MailboxRegistry, SignalRegistry};
use cellgov_time::{Budget, Epoch, GuestTicks};
use cellgov_trace::TraceWriter;

use crate::registry::{RegisteredUnit, UnitRegistry};
use crate::rsx;
use crate::scheduler::Scheduler;
use crate::syscall_table::SyscallResponseTable;

use super::{Runtime, RuntimeMode};

impl Runtime {
    // -- trace --

    /// Main binary trace stream emitted by the runtime.
    #[inline]
    pub fn trace(&self) -> &TraceWriter {
        &self.trace
    }

    /// Empty unless a unit had a zoom-in window configured.
    #[inline]
    pub fn zoom_trace(&self) -> &TraceWriter {
        &self.zoom_trace
    }

    // -- registries --

    /// Immutable view of the unit registry.
    #[inline]
    pub fn registry(&self) -> &UnitRegistry {
        &self.registry
    }

    /// Mutable view of the unit registry.
    #[inline]
    pub fn registry_mut(&mut self) -> &mut UnitRegistry {
        &mut self.registry
    }

    /// Immutable view of the mailbox registry.
    #[inline]
    pub fn mailbox_registry(&self) -> &MailboxRegistry {
        &self.mailbox_registry
    }

    /// Mutable view of the mailbox registry.
    #[inline]
    pub fn mailbox_registry_mut(&mut self) -> &mut MailboxRegistry {
        &mut self.mailbox_registry
    }

    /// Immutable view of the signal registry.
    #[inline]
    pub fn signal_registry(&self) -> &SignalRegistry {
        &self.signal_registry
    }

    /// Mutable view of the signal registry.
    #[inline]
    pub fn signal_registry_mut(&mut self) -> &mut SignalRegistry {
        &mut self.signal_registry
    }

    /// Mutable access to unit and mailbox registries together.
    #[inline]
    pub fn registries_mut(&mut self) -> (&mut UnitRegistry, &mut MailboxRegistry) {
        (&mut self.registry, &mut self.mailbox_registry)
    }

    // -- LV2 --

    /// Immutable view of the LV2 host state.
    #[inline]
    pub fn lv2_host(&self) -> &Lv2Host {
        &self.lv2_host
    }

    /// Mutable view of the LV2 host state.
    #[inline]
    pub fn lv2_host_mut(&mut self) -> &mut Lv2Host {
        &mut self.lv2_host
    }

    /// Cumulative count of `Effect::RsxLabelWrite` entries submitted
    /// to the commit pipeline.
    #[inline]
    pub fn rsx_label_writes_committed(&self) -> u64 {
        self.rsx_label_writes_committed
    }

    /// Cumulative count of `NV406E_SET_REFERENCE` dispatches across
    /// every `rsx_advance` invocation.
    #[inline]
    pub fn rsx_set_reference_dispatches(&self) -> u64 {
        self.rsx_set_reference_dispatches
    }

    /// Cumulative count of `Effect::SharedWriteIntent`s applied via
    /// the `apply_lv2_effects` direct-commit path.
    #[inline]
    pub fn lv2_direct_committed_writes(&self) -> u64 {
        self.lv2_direct_committed_writes
    }

    /// Invoked when `Lv2Dispatch::RegisterSpu` fires during `commit_step`.
    pub fn set_spu_factory<F>(&mut self, factory: F)
    where
        F: Fn(UnitId, SpuInitState) -> Box<dyn RegisteredUnit> + 'static,
    {
        self.spu_factory = Some(Box::new(factory));
    }

    /// Invoked when `Lv2Dispatch::PpuThreadCreate` fires during `commit_step`.
    pub fn set_ppu_factory<F>(&mut self, factory: F)
    where
        F: Fn(UnitId, PpuThreadInitState) -> Box<dyn RegisteredUnit> + 'static,
    {
        self.ppu_factory = Some(Box::new(factory));
    }

    /// Immutable view of the syscall response table.
    #[inline]
    pub fn syscall_responses(&self) -> &SyscallResponseTable {
        &self.syscall_responses
    }

    /// Mutable view of the syscall response table.
    #[inline]
    pub fn syscall_responses_mut(&mut self) -> &mut SyscallResponseTable {
        &mut self.syscall_responses
    }

    // -- DMA --

    /// Immutable view of the in-flight DMA queue.
    #[inline]
    pub fn dma_queue(&self) -> &DmaQueue {
        &self.dma_queue
    }

    // -- scheduler --

    /// Replace the runtime scheduler.
    pub fn set_scheduler<S: Scheduler + 'static>(&mut self, scheduler: S) {
        self.scheduler = Box::new(scheduler);
        self.scheduler_dirty_after_restore = false;
    }

    // -- mode / budget --

    /// Set the runtime trace / fault mode.
    pub fn set_mode(&mut self, mode: RuntimeMode) {
        self.mode = mode;
    }

    /// Current runtime trace / fault mode.
    pub fn mode(&self) -> RuntimeMode {
        self.mode
    }

    /// Takes effect on the next `step()` call. See
    /// [`super::default_budget_for_mode`] for per-mode defaults.
    pub fn set_budget(&mut self, budget: Budget) {
        self.budget_per_step = budget;
    }

    /// Current per-step execution budget.
    pub fn budget(&self) -> Budget {
        self.budget_per_step
    }

    // -- memory --

    /// Immutable view of guest memory.
    #[inline]
    pub fn memory(&self) -> &GuestMemory {
        &self.memory
    }

    /// Mutable view of guest memory.
    #[inline]
    pub fn memory_mut(&mut self) -> &mut GuestMemory {
        &mut self.memory
    }

    /// Immutable view of the load-reservation table.
    #[inline]
    pub fn reservations(&self) -> &cellgov_sync::ReservationTable {
        &self.reservations
    }

    /// Mutable view of the load-reservation table.
    #[inline]
    pub fn reservations_mut(&mut self) -> &mut cellgov_sync::ReservationTable {
        &mut self.reservations
    }

    // -- RSX --

    /// Set the base address used by `RsxLabelWrite` effects to
    /// compute the commit-side guest address. Narrowed to u32 at
    /// this boundary; `debug_assert` traps a 64-bit address whose
    /// upper bits are nonzero.
    pub fn set_rsx_label_base(&mut self, addr: GuestAddr) {
        debug_assert!(
            addr.raw() <= u32::MAX as u64,
            "set_rsx_label_base: addr=0x{:016x} exceeds u32 storage width; \
             RSX label base lives in the 32-bit MMIO window",
            addr.raw(),
        );
        self.rsx_label_base = addr.raw() as u32;
    }

    /// Immutable view of the RSX FIFO cursor.
    #[inline]
    pub fn rsx_cursor(&self) -> &rsx::RsxFifoCursor {
        &self.rsx_cursor
    }

    /// Mutable view of the RSX FIFO cursor.
    #[inline]
    pub fn rsx_cursor_mut(&mut self) -> &mut rsx::RsxFifoCursor {
        &mut self.rsx_cursor
    }

    /// Host must have made the RSX region writable before enabling;
    /// otherwise every put-pointer store reserved-writes and the mirror
    /// never runs.
    pub fn set_rsx_mirror_writes(&mut self, enabled: bool) {
        self.rsx_mirror_writes = enabled;
    }

    /// True when RSX control-register writes mirror into the cursor.
    #[inline]
    pub fn rsx_mirror_writes_enabled(&self) -> bool {
        self.rsx_mirror_writes
    }

    /// 40F honest FIFO consumer opt-in: when enabled, the cursor
    /// projects into MMIO `dma.ref`/`dma.get` after `rsx_advance`
    /// reaches the FIFO tail. Driven by the manifest
    /// `[rsx] consume` flag.
    pub fn set_rsx_consume_fifo(&mut self, enabled: bool) {
        self.rsx_consume_fifo = enabled;
    }

    /// True when the 40F consumer's MMIO side effects are armed.
    #[inline]
    pub fn rsx_consume_fifo_enabled(&self) -> bool {
        self.rsx_consume_fifo
    }

    /// Immutable view of the RSX FIFO call stack (40F).
    #[inline]
    pub fn rsx_call_stack(&self) -> &rsx::RsxCallStack {
        &self.rsx_call_stack
    }

    /// Immutable view of the RSX flip state.
    #[inline]
    pub fn rsx_flip(&self) -> &rsx::flip::RsxFlipState {
        &self.rsx_flip
    }

    /// Mutable view of the RSX flip state.
    #[inline]
    pub fn rsx_flip_mut(&mut self) -> &mut rsx::flip::RsxFlipState {
        &mut self.rsx_flip
    }

    // -- lifecycle --

    /// Drops all runtime state except [`GuestMemory`].
    pub fn into_memory(self) -> GuestMemory {
        self.memory
    }

    /// Current guest time.
    #[inline]
    pub fn time(&self) -> GuestTicks {
        self.time
    }

    /// Advances only at commit boundaries; `step()` never advances it.
    #[inline]
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Number of `step()` calls completed so far.
    #[inline]
    pub fn steps_taken(&self) -> usize {
        self.steps_taken
    }

    /// Step-count cap before `step()` returns `MaxStepsExceeded`.
    #[inline]
    pub fn max_steps(&self) -> usize {
        self.max_steps
    }

    // -- test-only --

    #[cfg(test)]
    pub(crate) fn effects_buf_mut_for_tests(&mut self) -> &mut Vec<Effect> {
        &mut self.effects_buf
    }

    #[cfg(test)]
    pub(crate) fn effects_buf_capacity_for_tests(&self) -> usize {
        self.effects_buf.capacity()
    }

    #[cfg(test)]
    pub(crate) fn per_step_index_for_tests(&self) -> u64 {
        self.per_step_index
    }
}
