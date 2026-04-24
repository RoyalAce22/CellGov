//! DMA completion handling. [`Runtime::fire_dma_completions`] runs per
//! commit and wakes issuers whose modeled-latency window arrived.
//! [`Runtime::drain_pending_dma`] runs at scenario termination and
//! forces every outstanding transfer into the final memory snapshot.

use cellgov_dma::DmaCompletion;
use cellgov_exec::UnitStatus;
use cellgov_time::GuestTicks;

use super::Runtime;

impl Runtime {
    /// Commit one DMA completion's payload to its destination. Returns
    /// `false` if the source read failed (missing payload and no
    /// committed source bytes) so the caller can skip post-apply work.
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

    /// Pop and apply DMA completions whose modeled time has arrived;
    /// returns the fired list for trace recording.
    ///
    /// Each completion explicitly sweeps overlapping reservations:
    /// DMA commits independently of `SharedWriteIntent`, so without
    /// this sweep a cross-unit MFC_PUT would leave a stale reservation
    /// a later `stwcx` / `putllc` could spuriously read as still held.
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

    /// Drain all pending DMA completions regardless of scheduled time;
    /// used at scenario termination to flush in-flight transfers into
    /// the final memory snapshot.
    pub fn drain_pending_dma(&mut self) {
        let due = self.dma_queue.pop_due(GuestTicks::new(u64::MAX));
        for (c, payload) in &due {
            self.apply_dma_transfer(c, payload);
        }
    }
}
