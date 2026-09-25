//! [`Runtime::sync_state_hash`] -- the runtime's committed-state
//! fingerprint.

use cellgov_mem::lanes;

use crate::runtime::state::Runtime;

/// Which partial each source reports: the one it keeps, or one rebuilt
/// from every entry.
#[derive(Clone, Copy)]
enum Partials {
    Kept,
    FromScratch,
}

impl Runtime {
    /// Multilinear-128 hash of the committed sync state: the high half
    /// of `key(Y, 0)` plus every source's partial.
    ///
    /// The lane space and its collision bound are in
    /// [`cellgov_mem::lanes`]. The sources enter the sum as follows:
    ///
    /// - The runtime-owned tables keep their partials in lane maps.
    /// - The RSX scalars compute their terms on read.
    /// - The LV2 host's tables add the host's partial.
    /// - The rest of the LV2 host's state enters as one transitional lane.
    ///
    /// Replay tooling compares pairs via the `SyncState` checkpoint
    /// emitted at every commit boundary.
    pub fn sync_state_hash(&self) -> u64 {
        (self.sync_sum(Partials::Kept) >> 64) as u64
    }

    /// [`Self::sync_state_hash`] computed from every entry of every
    /// source, without the partials the tables keep.
    pub fn sync_state_hash_from_scratch(&self) -> u64 {
        (self.sync_sum(Partials::FromScratch) >> 64) as u64
    }

    /// The sum of the additive key, every source's partial and the
    /// transitional lane.
    fn sync_sum(&self, partials: Partials) -> u128 {
        // A rebuild walks every entry, so the kept path does not call it.
        macro_rules! partial {
            ($source:expr) => {
                match partials {
                    Partials::Kept => $source.sync_partial(),
                    Partials::FromScratch => $source.sync_partial_from_scratch(),
                }
            };
        }
        let mut sum = lanes::additive_key();
        for term in [
            partial!(self.mailbox_registry),
            partial!(self.signal_registry),
            partial!(self.reservations),
            partial!(self.syscall_responses),
            partial!(self.timer_wakes),
            partial!(self.spaces),
            partial!(self.lv2_host),
            self.rsx_cursor.sync_term(),
            self.rsx_flip.sync_term(),
            lanes::value_term(
                lanes::source::RSX_SEM_OFFSET,
                0,
                &u64::from(self.rsx_sem_offset),
            ),
            lanes::contribution(
                lanes::LaneIndex::new(lanes::source::TRANSITIONAL, 0, 0, 0),
                self.lv2_host.state_hash(),
            ),
        ] {
            sum = sum.wrapping_add(term);
        }
        for table in self.spaces.extra_reservations.values() {
            sum = sum.wrapping_add(partial!(table));
        }
        sum
    }
}
