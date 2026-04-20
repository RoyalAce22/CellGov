//! DMA completion handling extracted from `runtime.rs`.
//!
//! Two drain paths live here:
//!
//! - [`Runtime::fire_dma_completions`] runs during every commit and
//!   wakes issuers whose modeled-latency window has arrived.
//! - [`Runtime::drain_pending_dma`] runs at scenario termination and
//!   forces every outstanding transfer visible in the final memory
//!   snapshot.
//!
//! Both paths apply transfers through `GuestMemory::apply_commit` via
//! the shared [`Runtime::apply_dma_transfer`] helper; the commit
//! path additionally sweeps overlapping reservations and transitions
//! the issuer back to `Runnable`.

use cellgov_dma::DmaCompletion;
use cellgov_exec::UnitStatus;
use cellgov_time::GuestTicks;

use super::Runtime;

impl Runtime {
    /// Resolve one DMA completion's payload and commit it to the
    /// destination range. Returns `true` if the transfer was
    /// applied, `false` if the source read failed (missing payload
    /// and no committed source bytes) so the caller can skip any
    /// post-apply bookkeeping.
    fn apply_dma_transfer(&mut self, c: &DmaCompletion, payload: &Option<Vec<u8>>) -> bool {
        let bytes = if let Some(data) = payload {
            data.clone()
        } else if let Some(src) = self.memory.read(c.source()) {
            src.to_vec()
        } else {
            return false;
        };
        let _ = self.memory.apply_commit(c.destination(), &bytes);
        true
    }

    /// Pop and apply DMA completions whose modeled time has arrived.
    /// Returns the list of fired completions for trace recording.
    ///
    /// Each completion runs the reservation clear-sweep against the
    /// destination range. DMA is a separate commit path from
    /// ordinary `SharedWriteIntent`, so the sweep must be invoked
    /// explicitly here; without it a cross-unit MFC_PUT would
    /// commit bytes that overlap another unit's reserved line
    /// without clearing the reservation, leaving a stale entry
    /// that a later `stwcx` / `putllc` would spuriously read as
    /// "still held." The `dma_completion_clears_overlapping_
    /// reservation` regression test pins the invariant.
    pub(super) fn fire_dma_completions(&mut self) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        let due = self.dma_queue.pop_due(self.time);
        for (c, payload) in &due {
            if !self.apply_dma_transfer(c, payload) {
                continue;
            }
            let dst = c.destination();
            self.reservations
                .clear_covering(dst.start().raw(), dst.length());
            self.registry
                .set_status_override(c.issuer(), UnitStatus::Runnable);
        }
        due
    }

    /// Drain all pending DMA completions regardless of their scheduled
    /// time, applying each transfer to committed memory. Used at
    /// scenario termination to ensure all in-flight transfers become
    /// visible in the final memory snapshot.
    pub fn drain_pending_dma(&mut self) {
        let due = self.dma_queue.pop_due(GuestTicks::new(u64::MAX));
        for (c, payload) in &due {
            self.apply_dma_transfer(c, payload);
        }
    }
}
