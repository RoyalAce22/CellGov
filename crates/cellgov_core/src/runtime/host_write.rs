//! The one path host-side code writes guest memory through.
//!
//! An execution unit reaches guest memory only through the commit
//! pipeline. The host -- LV2 dispatch, wake resolution, DMA completion,
//! the RSX mirrors and shared-view fanout -- has its own writes to
//! land, outside any unit's batch. [`Runtime::host_write`] gives them
//! the pipeline's three guest-visible steps:
//!
//! - the same write validation,
//! - the same reservation clear sweep,
//! - a trace record.
//!
//! It does not give them the pipeline's atomic-batch contract. Each
//! host write lands on its own, so a caller with several writes to
//! land all-or-none must validate the set first.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, MemError};
use cellgov_trace::{HostWriter, TraceRecord};

use super::spaces::AddressSpaceId;
use super::types::RuntimeMode;
use super::Runtime;

impl Runtime {
    /// Writes `bytes` to `range` in `space` on behalf of `writer`, and
    /// returns the count of reservations the clear sweep dropped.
    ///
    /// `exempt` names the one unit that keeps its reservation. The
    /// host passes it for a write it makes on that unit's behalf -- an
    /// LV2 out-parameter, or a DMA payload the unit issued. A
    /// mechanism with no unit behind it (the RSX, a sibling-view
    /// replication) passes `None` and exempts nobody.
    /// [PPC-Book2 p:10 s:1.7.3.1] a store by another processor
    /// or mechanism into the granule loses the reservation.
    ///
    /// # Errors
    ///
    /// Returns the [`MemError`] that `GuestMemory::apply_commit`
    /// rejects the write with. A refused write changes nothing: it
    /// lands no bytes, sweeps no reservation, and leaves no trace
    /// record.
    pub(super) fn host_write(
        &mut self,
        writer: HostWriter,
        space: AddressSpaceId,
        range: ByteRange,
        bytes: &[u8],
        exempt: Option<UnitId>,
    ) -> Result<usize, MemError> {
        let (mem, reservations) = super::spaces::resolve_commit_targets(
            &mut self.memory,
            &mut self.reservations,
            &mut self.spaces,
            space,
        );
        mem.apply_commit(range, bytes)?;
        let (addr, len) = (range.start().raw(), range.length());
        let cleared = reservations.clear_covering(addr, len, exempt);
        if self.mode != RuntimeMode::FaultDriven {
            // Both casts narrow to the record's u32 fields:
            // `apply_commit` proved the range lies inside one region,
            // a region spans a 32-bit guest address space, and the
            // table holds one entry per registered unit.
            debug_assert!(
                len <= u64::from(u32::MAX) && cleared <= u32::MAX as usize,
                "host write at {addr:#x}+{len:#x} clearing {cleared} reservations \
                 exceeds the trace record's u32 fields",
            );
            self.trace.record(&TraceRecord::HostWrite {
                writer,
                space: space.raw(),
                addr,
                len: len as u32,
                reservations_cleared: cleared as u32,
            });
        }
        Ok(cleared)
    }
}

#[cfg(test)]
#[path = "tests/host_write_tests.rs"]
mod tests;
