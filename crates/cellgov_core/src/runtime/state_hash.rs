//! [`Runtime::sync_state_hash`] -- fold every sync / committed-state
//! source the runtime owns into a single FNV-1a hash, in fixed order.

use crate::runtime::state::Runtime;

impl Runtime {
    /// FNV-1a merge over every sync / committed-state source the runtime
    /// owns (mailboxes, signal registers, LV2 host, syscall responses,
    /// reservations, RSX cursor / semaphore offset / flip state) in a
    /// fixed order. Replay tooling compares pairs via the `SyncState`
    /// checkpoint emitted at every commit boundary.
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
        ] {
            hasher.write(&source.to_le_bytes());
        }
        hasher.finish()
    }
}
