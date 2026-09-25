//! [`Runtime::sync_state_hash`] -- the runtime's committed-state
//! fingerprint.

use cellgov_mem::lanes;

use crate::runtime::state::Runtime;

impl Runtime {
    /// Multilinear-128 hash of the committed sync state: the high half
    /// of `key(Y, 0)` plus every source's partial.
    ///
    /// The lane space and its collision bound are in
    /// [`cellgov_mem::lanes`]. The mailbox, signal and reservation
    /// tables keep their partials current; the sources that keep no
    /// partial yet enter as one transitional lane.
    ///
    /// Replay tooling compares pairs via the `SyncState` checkpoint
    /// emitted at every commit boundary.
    pub fn sync_state_hash(&self) -> u64 {
        let sum = self.sync_partials(
            self.mailbox_registry.sync_partial(),
            self.signal_registry.sync_partial(),
            |t| t.sync_partial(),
        );
        (sum >> 64) as u64
    }

    /// [`Self::sync_state_hash`] computed from every entry of every
    /// source, without the partials the tables keep.
    pub fn sync_state_hash_from_scratch(&self) -> u64 {
        let sum = self.sync_partials(
            self.mailbox_registry.sync_partial_from_scratch(),
            self.signal_registry.sync_partial_from_scratch(),
            |t| t.sync_partial_from_scratch(),
        );
        (sum >> 64) as u64
    }

    /// The sum of the additive key, the given registry partials, every
    /// reservation table's partial and the transitional lane.
    fn sync_partials(
        &self,
        mailboxes: u128,
        signals: u128,
        reservations: impl Fn(&cellgov_sync::ReservationTable) -> u128,
    ) -> u128 {
        let mut sum = lanes::additive_key()
            .wrapping_add(mailboxes)
            .wrapping_add(signals)
            .wrapping_add(reservations(&self.reservations));
        for table in self.spaces.extra_reservations.values() {
            sum = sum.wrapping_add(reservations(table));
        }
        sum.wrapping_add(lanes::contribution(
            lanes::LaneIndex::new(lanes::source::TRANSITIONAL, 0, 0, 0),
            self.transitional_fold(),
        ))
    }

    /// FNV-1a fold of the sources that keep no partial yet, in a fixed
    /// order.
    fn transitional_fold(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for source in [
            self.lv2_host.state_hash(),
            self.syscall_responses.state_hash(),
            self.rsx_cursor.state_hash(),
            self.rsx_sem_offset as u64,
            self.rsx_flip.state_hash(),
            self.timer_wakes.state_hash(),
        ] {
            hasher.write(&source.to_le_bytes());
        }
        if !self.spaces.is_empty() {
            hasher.write(&self.spaces.metadata_hash().to_le_bytes());
        }
        hasher.finish()
    }
}
