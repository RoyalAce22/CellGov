//! [`Runtime::sync_state_hash`] -- the runtime's committed-state
//! fingerprint.

use crate::runtime::state::Runtime;

impl Runtime {
    /// FNV-1a fold over every sync / committed-state source the runtime
    /// owns, in a fixed order.
    ///
    /// Replay tooling compares pairs via the `SyncState` checkpoint
    /// emitted at every commit boundary.
    pub fn sync_state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [
            self.mailbox_registry.state_hash(),
            self.signal_registry.state_hash(),
            self.lv2_host.state_hash(),
            self.syscall_responses.state_hash(),
            self.reservations.state_hash(),
            self.rsx_cursor.state_hash(),
            self.rsx_sem_offset as u64,
            self.rsx_flip.state_hash(),
            self.timer_wakes.state_hash(),
        ] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.finish()
    }
}
