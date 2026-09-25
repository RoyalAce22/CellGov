//! [`Runtime::sync_state_hash`] -- the runtime's committed-state
//! fingerprint.

use cellgov_effects::Effect;
use cellgov_mem::lanes::{self, source, LaneValue, ObjectLanes};

use crate::runtime::state::Runtime;

/// Which partial each source reports: the one it keeps, or one rebuilt
/// from every entry.
#[derive(Clone, Copy)]
enum Partials {
    Kept,
    FromScratch,
}

/// One effect the FIFO advance pass queued for the next batch.
///
/// Field 1 is the effect kind, plus 1. The other fields depend on the
/// variant:
///
/// - `RsxLabelWrite`: field 2 is the offset and field 3 is the value.
/// - `RsxFlipRequest`: field 2 is the buffer index.
/// - Any other variant has the kind lane only. The advance pass queues
///   no other variant.
struct DeferredRsxEffect<'a>(&'a Effect);

impl LaneValue for DeferredRsxEffect<'_> {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.0.kind() as u64 + 1);
        match *self.0 {
            Effect::RsxLabelWrite { offset, value } => {
                lanes.lane(2, 0, u64::from(offset));
                lanes.lane(3, 0, u64::from(value));
            }
            Effect::RsxFlipRequest { buffer_index } => {
                lanes.lane(2, 0, u64::from(buffer_index));
            }
            _ => {}
        }
    }
}

impl Runtime {
    /// Multilinear-128 hash of the committed sync state: the high half
    /// of `key(Y, 0)` plus every source's partial.
    ///
    /// The lane space and its collision bound are in
    /// [`cellgov_mem::lanes`]. The sources enter the sum as follows:
    ///
    /// - The runtime-owned tables and the DMA queue keep their partials
    ///   in lane maps.
    /// - These sources compute their terms on read:
    ///   - the RSX scalars
    ///   - the RSX call stack
    ///   - the deferred RSX effects
    ///   - the pending DMA tag bits
    ///   - the pending child inits
    /// - The LV2 host adds its partial.
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

    /// The sum of the additive key and every source's partial.
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
            partial!(self.dma_queue),
            partial!(self.lv2_host),
            self.rsx_cursor.sync_term(),
            self.rsx_flip.sync_term(),
            lanes::value_term(source::RSX_SEM_OFFSET, 0, &u64::from(self.rsx_sem_offset)),
            lanes::value_term(source::RSX_LABEL_BASE, 0, &u64::from(self.rsx_label_base)),
            self.rsx_call_stack.sync_term(),
        ] {
            sum = sum.wrapping_add(term);
        }
        for table in self.spaces.extra_reservations.values() {
            sum = sum.wrapping_add(partial!(table));
        }
        for (i, effect) in self.pending_rsx_effects.iter().enumerate() {
            sum = sum.wrapping_add(lanes::value_term(
                source::RSX_PENDING_EFFECTS,
                i as u64,
                &DeferredRsxEffect(effect),
            ));
        }
        for (unit, &bits) in &self.pending_tag_completions {
            sum = sum.wrapping_add(lanes::value_term(
                source::DMA_TAG_COMPLETIONS,
                unit.raw(),
                &bits,
            ));
        }
        for (i, init) in self.pending_child_inits.iter().enumerate() {
            sum = sum.wrapping_add(lanes::value_term(
                source::PENDING_CHILD_INIT,
                i as u64,
                init,
            ));
        }
        sum
    }
}
