//! The boot-state seeds, the committed-effect witness and the pending region installs.

use std::collections::BTreeMap;

use crate::host::mmapper::SystemStateSeed;
use crate::host::system_ipc_witness::SystemIpcMapping;

use super::model::Lv2Host;

impl Lv2Host {
    /// Register a boot-state seed; a duplicate `shm_ipc_key` replaces
    /// the prior entry (last-write-wins). Boot-only: registering
    /// after the matching shm has been mapped has no effect.
    pub fn register_system_seed(&mut self, seed: SystemStateSeed) {
        self.derived
            .system_state_seeds
            .insert(seed.shm_ipc_key, seed);
    }

    /// Boot-registered seeds keyed by `shm_ipc_key`.
    pub fn system_state_seeds(&self) -> &BTreeMap<u64, SystemStateSeed> {
        &self.derived.system_state_seeds
    }

    /// `true` once the seed registered under `shm_ipc_key` has been
    /// applied by a 334 / 337 map.
    pub fn system_seed_applied(&self, shm_ipc_key: u64) -> bool {
        self.derived.system_seeds_applied.contains(&shm_ipc_key)
    }

    /// Mapped guest base of the seeded shm, once applied.
    pub fn system_seed_base(&self, shm_ipc_key: u64) -> Option<u32> {
        self.derived.system_seed_bases.get(&shm_ipc_key).copied()
    }

    /// Count of event queues registered under an ipc key.
    pub fn keyed_event_queue_count(&self) -> usize {
        self.derived.event_queue_ipc.len()
    }

    /// Count committed writes that land in a namespace-keyed shm.
    ///
    /// # Cross-module contract
    ///
    /// The runtime must call this once per successful commit, with the
    /// same effect slice the commit pipeline applied. Calling it before
    /// the commit succeeds would count writes a fault discarded.
    ///
    /// O(writes * mappings), and returns on the first line for a boot
    /// that mapped no namespace shm at all.
    pub fn note_committed_effects(&mut self, effects: &[cellgov_effects::Effect]) {
        if self.obs.system_ipc_mappings.is_empty() {
            return;
        }
        for effect in effects {
            let cellgov_effects::Effect::SharedWriteIntent { range, .. } = effect else {
                continue;
            };
            let start = range.start().raw();
            let end = start.saturating_add(range.length());
            let hit = self
                .obs
                .system_ipc_mappings
                .values()
                .find(|m| {
                    let m_start = u64::from(m.base);
                    start < m_start + u64::from(m.size) && m_start < end
                })
                .map(|m| m.ipc_key);
            if let Some(ipc_key) = hit {
                self.obs.system_ipc_witness.shm_writes += 1;
                self.obs.system_ipc_witness.note_key(ipc_key);
            }
        }
    }

    /// Record a namespace-keyed shm mapping and bump the map witness.
    pub(in crate::host) fn note_system_ipc_map(&mut self, mem_id: u32, base: u32, size: u32) {
        let Some((ipc_key, _)) = self.state.mmapper_ipc.iter().find(|&(_, &id)| id == mem_id)
        else {
            return;
        };
        if !crate::host::is_system_ipc_key(ipc_key) {
            return;
        }
        self.obs.system_ipc_mappings.insert(
            base,
            SystemIpcMapping {
                ipc_key,
                base,
                size,
            },
        );
        self.obs.system_ipc_witness.shm_maps += 1;
        self.obs.system_ipc_witness.note_key(ipc_key);
    }

    /// Read-only `pending_region_installs` snapshot used by sibling
    /// dispatch-arm tests; the runtime remains the drain consumer.
    #[cfg(all(test, debug_assertions))]
    pub(in crate::host) fn drain_pending_region_installs_inspect(
        &self,
    ) -> &[crate::host::mmapper::PendingRegionInstall] {
        &self.derived.pending_region_installs
    }
}
