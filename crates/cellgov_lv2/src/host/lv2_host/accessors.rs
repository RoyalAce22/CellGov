//! The filesystem, content and registry accessors.

use cellgov_event::UnitId;

use crate::fs_store::{FsMountTable, FsStore};
use crate::image::ContentStore;
use crate::ppu_thread::{PpuThread, PpuThreadAttrs, PpuThreadId, PpuThreadTable};
use crate::prx_registry::LoadedPrxRegistry;
use crate::sync_primitives::{
    CondTable, EventFlagTable, EventQueueTable, LwMutexTable, MutexTable, SemaphoreTable,
};
use crate::thread_group::ThreadGroupTable;

use super::model::Lv2Host;

impl Lv2Host {
    /// In-memory filesystem store.
    pub fn fs_store(&self) -> &FsStore {
        &self.state.fs_store
    }

    /// Mutable view of [`Self::fs_store`].
    pub fn fs_store_mut(&mut self) -> &mut FsStore {
        &mut self.state.fs_store
    }

    /// Guest-path to host-path mount table.
    pub fn fs_mounts(&self) -> &FsMountTable {
        &self.derived.fs_mounts
    }

    /// Mutable view; written by boot only.
    pub fn fs_mounts_mut(&mut self) -> &mut FsMountTable {
        &mut self.derived.fs_mounts
    }

    /// Per-title content manifest store.
    pub fn content_store(&self) -> &ContentStore {
        &self.state.content
    }

    /// Mutable view of [`Self::content_store`].
    pub fn content_store_mut(&mut self) -> &mut ContentStore {
        &mut self.state.content
    }

    /// Loaded-PRX registry.
    pub fn prx_registry(&self) -> &LoadedPrxRegistry {
        &self.state.prx_registry
    }

    /// Mutable view of [`Self::prx_registry`].
    pub fn prx_registry_mut(&mut self) -> &mut LoadedPrxRegistry {
        &mut self.state.prx_registry
    }

    /// SPU thread-group table.
    pub fn thread_groups(&self) -> &ThreadGroupTable {
        &self.state.groups
    }

    /// Mutable view of [`Self::thread_groups`].
    pub fn thread_groups_mut(&mut self) -> &mut ThreadGroupTable {
        &mut self.state.groups
    }

    /// PPU thread table.
    pub fn ppu_threads(&self) -> &PpuThreadTable {
        &self.state.ppu_threads
    }

    /// Mutable view of [`Self::ppu_threads`].
    pub fn ppu_threads_mut(&mut self) -> &mut PpuThreadTable {
        &mut self.state.ppu_threads
    }

    /// Call exactly once after the primary PPU unit is registered.
    pub fn seed_primary_ppu_thread(&mut self, unit_id: UnitId, attrs: PpuThreadAttrs) {
        self.state.ppu_threads.insert_primary(unit_id, attrs);
    }

    /// Alias a transient unit (e.g. a per-module module_start unit)
    /// to the primary thread so sync-syscall dispatch resolves the
    /// caller. Mirrors real LV2's "module_start runs on the calling
    /// thread" contract. See
    /// [`PpuThreadTable::alias_unit`][crate::ppu_thread::PpuThreadTable::alias_unit].
    pub fn alias_unit_to_primary(&mut self, unit_id: UnitId) -> bool {
        self.state
            .ppu_threads
            .alias_unit(unit_id, PpuThreadId::PRIMARY)
    }

    /// Alias a transient unit (a spawned child's module_start unit) to
    /// the PPU thread `owner` runs as; see
    /// [`Self::alias_unit_to_primary`]. `false` when `owner` has no
    /// thread record or `unit_id` is already mapped.
    pub fn alias_unit_to_thread_of(&mut self, unit_id: UnitId, owner: UnitId) -> bool {
        let Some(thread) = self.state.ppu_threads.thread_id_for_unit(owner) else {
            return false;
        };
        self.state.ppu_threads.alias_unit(unit_id, thread)
    }

    /// Drop an alias previously installed via [`Self::alias_unit_to_primary`]
    /// or [`Self::alias_unit_to_thread_of`].
    pub fn drop_ppu_thread_alias(&mut self, unit_id: UnitId) -> bool {
        self.state.ppu_threads.drop_alias(unit_id)
    }

    /// PPU thread record bound to `unit_id`, if any.
    pub fn ppu_thread_for_unit(&self, unit_id: UnitId) -> Option<&PpuThread> {
        self.state.ppu_threads.get_by_unit(unit_id)
    }

    /// PPU thread id bound to `unit_id`, if any.
    pub fn ppu_thread_id_for_unit(&self, unit_id: UnitId) -> Option<PpuThreadId> {
        self.state.ppu_threads.thread_id_for_unit(unit_id)
    }

    /// `false` when `unit_id` has no PPU mapping.
    pub fn is_ppu_thread_finished_for_unit(&self, unit_id: UnitId) -> bool {
        match self.state.ppu_threads.get_by_unit(unit_id) {
            Some(thread) => thread.state.is_finished(),
            None => false,
        }
    }

    /// Lightweight mutex table.
    pub fn lwmutexes(&self) -> &LwMutexTable {
        &self.state.lwmutexes
    }

    /// Mutable view of [`Self::lwmutexes`].
    pub fn lwmutexes_mut(&mut self) -> &mut LwMutexTable {
        &mut self.state.lwmutexes
    }

    /// Mutex table.
    pub fn mutexes(&self) -> &MutexTable {
        &self.state.mutexes
    }

    /// Mutable view of [`Self::mutexes`].
    pub fn mutexes_mut(&mut self) -> &mut MutexTable {
        &mut self.state.mutexes
    }

    /// Semaphore table.
    pub fn semaphores(&self) -> &SemaphoreTable {
        &self.state.semaphores
    }

    /// Mutable view of [`Self::semaphores`].
    pub fn semaphores_mut(&mut self) -> &mut SemaphoreTable {
        &mut self.state.semaphores
    }

    /// Event-queue table.
    pub fn event_queues(&self) -> &EventQueueTable {
        &self.state.event_queues
    }

    /// Mutable view of [`Self::event_queues`].
    pub fn event_queues_mut(&mut self) -> &mut EventQueueTable {
        &mut self.state.event_queues
    }

    /// Event-flag table.
    pub fn event_flags(&self) -> &EventFlagTable {
        &self.state.event_flags
    }

    /// Condition-variable table.
    pub fn conds(&self) -> &CondTable {
        &self.state.conds
    }

    /// Mutable view of [`Self::conds`].
    pub fn conds_mut(&mut self) -> &mut CondTable {
        &mut self.state.conds
    }

    /// Mutable view of [`Self::event_flags`].
    pub fn event_flags_mut(&mut self) -> &mut EventFlagTable {
        &mut self.state.event_flags
    }
}
