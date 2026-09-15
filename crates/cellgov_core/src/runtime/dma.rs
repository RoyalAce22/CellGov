//! DMA completion handling. [`Runtime::fire_dma_completions`] runs per
//! commit and wakes issuers whose modeled-latency window arrived.
//! [`Runtime::drain_pending_dma`] runs at scenario termination and
//! forces every outstanding transfer into the final memory snapshot.

use cellgov_dma::DmaCompletion;
use cellgov_exec::UnitStatus;
use cellgov_time::GuestTicks;
use cellgov_trace::HostWriter;

use super::spaces::AddressSpaceId;
use super::Runtime;

impl Runtime {
    /// Commit one DMA completion's payload to its destination.
    ///
    /// Infallible by construction: `pre_validate`'s DmaEnqueue arm
    /// proved the destination is mapped and `ReadWrite` at enqueue,
    /// regions are add-only with immutable access, and snapshots
    /// co-capture queue and memory -- so the destination remains
    /// writable at completion.
    ///
    /// The write sweeps overlapping reservations. DMA commits
    /// independently of `SharedWriteIntent`. Without the sweep, a
    /// cross-unit MFC_PUT leaves a stale reservation, and a later
    /// `stwcx` / `putllc` reads it as still held.
    /// [PPC-Book2 p:10 s:1.7.3.1] the issuer's own reservation is
    /// preserved; only other processors' reservations are invalidated.
    /// [CBE-Handbook p:479 s:18.6.4] names the Cell half of that split:
    /// the atomic unit loses a lock-line reservation when another
    /// processor element or device modifies the line, and the issuer's
    /// own MFC transfer is a local SPE action rather than that outside
    /// entity.
    fn apply_dma_transfer(&mut self, c: &DmaCompletion, payload: &Option<Vec<u8>>) {
        let bytes = if let Some(data) = payload {
            data.clone()
        } else {
            self.memory
                .read(c.source())
                .expect("DMA source range mapped and readable at enqueue")
                .to_vec()
        };
        // DMA stays in space 0 end to end (see the `spaces` module docs).
        self.host_write(
            HostWriter::DmaCompletion,
            AddressSpaceId::BOOT,
            c.destination(),
            &bytes,
            Some(c.issuer()),
        )
        .expect("DMA destination validated as ReadWrite at enqueue");
    }

    /// Pop and apply DMA completions whose modeled time has arrived;
    /// returns the fired list for trace recording.
    pub(super) fn fire_dma_completions(&mut self) -> Vec<(DmaCompletion, Option<Vec<u8>>)> {
        let due = self.dma_queue.pop_due(self.time);
        for (c, payload) in &due {
            self.apply_dma_transfer(c, payload);
            // The transfer still commits, so the terminal memory
            // snapshot holds the payload. The `Runnable` override below
            // would replace either of these issuer states:
            // - `Finished`: process-exit residue (see the `runtime`
            //   module docs), or an SPU that stopped before its
            //   transfer completed. An MFC put does not block the
            //   issuer.
            // - `Faulted`: the commit pipeline's `pre_validate` refused
            //   a later DmaEnqueue from the issuer, and that mark keeps
            //   the unit off the scheduler.
            if matches!(
                self.registry.effective_status(c.issuer()),
                Some(UnitStatus::Finished | UnitStatus::Faulted)
            ) {
                continue;
            }
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

#[cfg(test)]
#[path = "tests/dma_space_tests.rs"]
mod dma_space_tests;
