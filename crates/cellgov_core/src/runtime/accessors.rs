//! Pure getters, setters, and field-shaped accessors over [`Runtime`].
//!
//! No business logic -- entries here are field plumbing only. Methods
//! that compute over multiple fields (e.g. `sync_state_hash`) stay in
//! `mod.rs` next to the struct definition.

use cellgov_dma::DmaQueue;
#[cfg(test)]
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_lv2::{Lv2Host, PpuThreadInitState, SpuInitState};
use cellgov_mem::GuestMemory;
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

    /// Sets the base address used by `RsxLabelWrite` effects when
    /// computing the commit-side guest address. Synthetic test
    /// scenarios call this to wire up label memory without booting
    /// the firmware-set RSX init path.
    pub fn set_rsx_label_base(&mut self, addr: u32) {
        self.rsx_label_base = addr;
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

    /// Last parsed RSX semaphore-write offset.
    #[inline]
    pub fn rsx_sem_offset(&self) -> u32 {
        self.rsx_sem_offset
    }

    /// Mutable reference to the RSX semaphore-write offset.
    #[inline]
    pub fn rsx_sem_offset_mut(&mut self) -> &mut u32 {
        &mut self.rsx_sem_offset
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

    /// Consume the runtime and return its guest memory. Chains execution
    /// stages: run one runtime, extract the initialized memory, seed a
    /// fresh runtime for the next stage.
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
