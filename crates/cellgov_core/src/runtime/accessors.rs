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

    /// Mutable view of the unit registry, for test setup.
    #[cfg(test)]
    #[inline]
    pub(crate) fn registry_mut(&mut self) -> &mut UnitRegistry {
        &mut self.registry
    }

    /// Registers the unit `factory` builds from the id the registry
    /// assigns. See [`UnitRegistry::register_with`] for the id
    /// contract.
    ///
    /// # Panics
    ///
    /// Panics if the new unit reports an id other than the assigned
    /// one.
    pub fn register_unit_with<U, F>(&mut self, factory: F) -> UnitId
    where
        U: cellgov_exec::ExecutionUnit + Clone + 'static,
        F: FnOnce(UnitId) -> U,
    {
        self.registry.register_with(factory)
    }

    /// Iterates every registered unit mutably, in id order.
    pub fn units_mut(&mut self) -> impl Iterator<Item = (UnitId, &mut dyn RegisteredUnit)> + '_ {
        self.registry.iter_mut()
    }

    /// Holds `unit` at `status` until
    /// [`Runtime::clear_unit_status_override`] clears it. Does nothing
    /// for an unregistered id.
    pub fn set_unit_status_override(&mut self, unit: UnitId, status: cellgov_exec::UnitStatus) {
        self.registry.set_status_override(unit, status);
    }

    /// Returns `unit` to the status it reports for itself.
    pub fn clear_unit_status_override(&mut self, unit: UnitId) {
        self.registry.clear_status_override(unit);
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

    /// Cumulative `sys_timer_usleep` / `sys_timer_sleep` dispatches
    /// through the runtime timer path, including zero-interval
    /// requests that yield instead of parking.
    #[inline]
    pub fn timer_sleep_dispatches(&self) -> u64 {
        self.timer_sleep_dispatches
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

    /// Invoked when `Lv2Dispatch::ProcessSpawn` fires; installs a
    /// spawned SELF's image into its fresh child-space memory. Spawn
    /// syscalls fail with `CELL_ENOSYS` while unset.
    pub fn set_process_spawn_loader<F>(&mut self, loader: F)
    where
        F: Fn(
                &[u8],
                &mut cellgov_mem::GuestMemory,
            )
                -> Result<super::types::SpawnedProcessImage, super::types::ProcessSpawnLoadError>
            + 'static,
    {
        self.process_spawn_loader = Some(Box::new(loader));
    }

    /// Install the debug observer the runtime reports writes and steps to.
    pub fn set_tap(&mut self, tap: Box<dyn super::RuntimeTap>) {
        self.tap = Some(tap);
    }

    /// True when a spawned child is parked behind a loader-staged init
    /// pass the host has not run yet.
    #[inline]
    pub fn has_pending_child_init(&self) -> bool {
        !self.pending_child_inits.is_empty()
    }

    /// Take every parked child, in spawn order. The caller runs each
    /// child's init pass and then calls [`Self::release_child_init`];
    /// a child taken and never released stays parked.
    pub fn take_pending_child_inits(&mut self) -> Vec<super::types::PendingChildInit> {
        std::mem::take(&mut self.pending_child_inits)
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

    /// Units runnable when the scheduler made its last choice.
    ///
    /// Empty before the first [`Runtime::step`] and after a restore.
    /// Read it after the call: a step that warps guest time to fire a
    /// due DMA completion or timer wake widens the set the scheduler
    /// chose from. A step that refuses leaves the previous step's set
    /// in place.
    #[inline]
    pub fn last_runnable(&self) -> &[UnitId] {
        &self.last_runnable
    }

    /// DMA completions the last [`Runtime::commit_step`] fired, each
    /// with whether an inline payload carried its bytes.
    ///
    /// Empty before the first commit and after a restore. The commit
    /// that fires a completion applies the write outside every unit's
    /// batch, so this list is where a caller reads which bytes moved.
    /// The all-blocked time warp inside [`Runtime::step`] fires
    /// completions outside every commit, and none of them reaches this
    /// list.
    ///
    /// The flag is what [`DmaQueue::pending`] carries, and for the same
    /// reason: a payloaded transfer reads no source at completion.
    #[inline]
    pub fn last_dma_completions(&self) -> &[(cellgov_dma::DmaCompletion, bool)] {
        &self.last_dma_completions
    }

    /// Guest ranges host-side code wrote since the last
    /// [`Runtime::step`] began, each tagged with the mechanism that
    /// wrote it, in the order they landed.
    ///
    /// These writes land outside every unit's batch, so a step's
    /// effect list alone does not show them: an LV2 handler's out
    /// parameter, a wake continuation, a DMA payload, the RSX mirrors
    /// and the shared-view replications are all here.
    ///
    /// # Cross-module contract
    ///
    /// - [`Runtime::step`] empties the list when it starts, and
    ///   [`Runtime::commit_step`] leaves it alone. A driver reads one
    ///   step's writes after it commits that step.
    /// - `SchedulerNotReinstalled` and `MaxStepsExceeded` are the two
    ///   refusals `step` answers ahead of that clear, so each leaves
    ///   the previous step's record in place. Every later refusal
    ///   clears first and leaves whatever the time warp wrote.
    /// - The all-blocked time warp inside [`Runtime::step`] fires the
    ///   DMA and timer wakes, so what they write belongs to the step
    ///   the warp then picked.
    /// - A restore empties the list.
    /// - A refused write records nothing.
    /// - A [`HostWriter::Placement`] the driver makes between two steps
    ///   stands in the list until the next step clears it.
    /// - Every [`RuntimeMode`] builds the list; the mode gates the
    ///   `HostWrite` trace record alone.
    /// - An entry carries no address space, so a write at one numeric
    ///   address in two spaces gives two entries that read alike.
    ///
    /// [`HostWriter::Placement`]: cellgov_trace::HostWriter::Placement
    #[inline]
    pub fn last_host_writes(&self) -> &[(cellgov_trace::HostWriter, cellgov_mem::ByteRange)] {
        &self.last_host_writes
    }

    /// Effects an LV2 handler applied since the last
    /// [`Runtime::step`] began, in the order they landed.
    ///
    /// A handler applies its effects at dispatch, so no unit's effect
    /// list holds them. The list holds the applied effects alone; a
    /// dropped effect is absent. The guest writes among them also
    /// reach [`Runtime::last_host_writes`], which states the contract
    /// both lists follow.
    #[inline]
    pub fn last_lv2_effects(&self) -> &[cellgov_effects::Effect] {
        &self.last_lv2_effects
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

    /// Mutable view of guest memory, for test setup.
    #[cfg(test)]
    #[inline]
    pub(crate) fn memory_mut(&mut self) -> &mut GuestMemory {
        &mut self.memory
    }

    /// Immutable view of space 0's load-reservation table; child
    /// spaces resolve via [`Runtime::space_reservations`].
    #[inline]
    pub fn reservations(&self) -> &cellgov_sync::ReservationTable {
        &self.reservations
    }

    /// Mutable view of space 0's load-reservation table, for test
    /// setup.
    #[cfg(test)]
    #[inline]
    pub(crate) fn reservations_mut(&mut self) -> &mut cellgov_sync::ReservationTable {
        &mut self.reservations
    }

    // -- RSX --

    /// Seed the base `RsxLabelWrite` effects resolve against.
    ///
    /// Only consulted while the LV2 RSX context has published no
    /// reports base; after `sys_rsx_context_allocate` the context's
    /// base wins over this seed.
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

    /// Honest FIFO consumer opt-in: when enabled, the cursor
    /// projects into MMIO `dma.ref`/`dma.get` after `rsx_advance`
    /// reaches the FIFO tail. Driven by the manifest
    /// `[rsx] consume` flag.
    pub fn set_rsx_consume_fifo(&mut self, enabled: bool) {
        self.rsx_consume_fifo = enabled;
    }

    /// True when the FIFO consumer's MMIO side effects are armed.
    #[inline]
    pub fn rsx_consume_fifo_enabled(&self) -> bool {
        self.rsx_consume_fifo
    }

    /// Immutable view of the RSX FIFO call stack.
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

    /// Number of pending timer wakes.
    #[inline]
    pub fn timer_wakes_pending(&self) -> usize {
        self.timer_wakes.len()
    }

    #[cfg(test)]
    pub(crate) fn timer_wakes_cancel_for_test(&mut self, unit: cellgov_event::UnitId) -> bool {
        self.timer_wakes.cancel(unit)
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
