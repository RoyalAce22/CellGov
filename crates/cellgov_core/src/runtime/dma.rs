//! DMA completion handling. [`Runtime::fire_dma_completions`] runs per
//! commit and wakes issuers whose modeled-latency window arrived.
//! [`Runtime::drain_pending_dma`] runs at scenario termination and
//! forces every outstanding transfer into the final memory snapshot.

use cellgov_dma::DmaCompletion;
use cellgov_exec::UnitStatus;
use cellgov_time::GuestTicks;

use super::Runtime;

impl Runtime {
    /// Commit one DMA completion's payload to its destination.
    ///
    /// Infallible by construction: `pre_validate`'s DmaEnqueue arm
    /// proved the destination is mapped and `ReadWrite` at enqueue,
    /// regions are add-only with immutable access, and snapshots
    /// co-capture queue and memory -- so the destination remains
    /// writable at completion.
    fn apply_dma_transfer(&mut self, c: &DmaCompletion, payload: &Option<Vec<u8>>) {
        let bytes = if let Some(data) = payload {
            data.clone()
        } else {
            self.memory
                .read(c.source())
                .expect("DMA source range mapped and readable at enqueue")
                .to_vec()
        };
        self.memory
            .apply_commit(c.destination(), &bytes)
            .expect("DMA destination validated as ReadWrite at enqueue");
    }

    /// Pop and apply DMA completions whose modeled time has arrived;
    /// returns the fired list for trace recording.
    ///
    /// Each completion sweeps overlapping cross-unit reservations:
    /// DMA commits independently of `SharedWriteIntent`, so without
    /// this sweep a cross-unit MFC_PUT would leave a stale reservation
    /// a later `stwcx` / `putllc` could spuriously read as still held.
    pub(super) fn fire_dma_completions(&mut self) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        let due = self.dma_queue.pop_due(self.time);
        for (c, payload) in &due {
            self.apply_dma_transfer(c, payload);
            let dst = c.destination();
            // [PPC-Book2 p:10 s:1.7.3.1] DMA completion: the issuer's own
            // reservation is preserved; only other processors' reservations
            // are invalidated.
            self.reservations
                .clear_covering(dst.start().raw(), dst.length(), Some(c.issuer()));
            self.registry
                .set_status_override(c.issuer(), UnitStatus::Runnable);
            if let Some(tag_id) = c.request().tag_id() {
                *self.pending_tag_completions.entry(c.issuer()).or_insert(0) |= 1u32 << tag_id;
            }
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
