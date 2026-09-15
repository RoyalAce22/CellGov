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
    /// LV2 out-parameter, or a DMA payload the unit issued. A mechanism
    /// with no unit behind it, such as the RSX, passes `None` and
    /// exempts nobody.
    /// [PPC-Book2 p:10 s:1.7.3.1] a store by another processor
    /// or mechanism into the granule loses the reservation.
    ///
    /// A sibling-view replication carries whatever its own writer
    /// carries, because every view of a segment names one granule: a
    /// store replicates with `None`, a DMA landing with its issuer.
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
        let (mem, reservations, _dma_memory) = super::spaces::resolve_commit_targets(
            &mut self.memory,
            &mut self.reservations,
            &mut self.spaces,
            space,
        );
        mem.apply_commit(range, bytes)?;
        let (addr, len) = (range.start().raw(), range.length());
        let cleared = reservations.clear_covering(addr, len, exempt);
        // The push precedes the mode gate below: a FaultDriven run
        // writes no trace record and still publishes this entry.
        self.last_host_writes.push((writer, space, range));
        if let Some(tap) = self.tap.as_deref_mut() {
            tap.write(addr, bytes);
        }
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

    /// Places `bytes` over `range` in `space`, and returns the count of
    /// reservations the clear sweep dropped.
    ///
    /// This is the write the program driving the runtime makes on its
    /// own account, and the trace names it
    /// [`HostWriter::Placement`]. No unit is behind it, so the clear
    /// sweep exempts nobody.
    ///
    /// # Errors
    ///
    /// Returns the [`MemError`] that `GuestMemory::apply_commit`
    /// rejects the write with. A refused placement changes nothing.
    ///
    /// # Panics
    ///
    /// Panics if `space` is not a live address space.
    ///
    /// Panics in a debug build when the bytes land inside a shared
    /// view. A release build logs
    /// `runtime.place_bytes_targets_shared_view` instead.
    pub fn place_bytes(
        &mut self,
        space: AddressSpaceId,
        range: ByteRange,
        bytes: &[u8],
    ) -> Result<usize, MemError> {
        let cleared = self.host_write(HostWriter::Placement, space, range, bytes, None)?;
        // A placement skips the commit pipeline's shared-view fanout,
        // so bytes that land inside a shared view leave the sibling
        // views incoherent. `commit_bytes_at` and the LV2
        // `SharedWriteIntent` path in `apply_lv2_effects` guard the
        // same way.
        if self.range_intersects_shared_view(space, range) {
            self.lv2_host.log_invariant_break(
                "runtime.place_bytes_targets_shared_view",
                format_args!(
                    "placement at 0x{:x}+0x{:x} targets a shared view in space {}; \
                     cross-space replication of a placement is not modeled, sibling \
                     views are now incoherent",
                    range.start().raw(),
                    range.length(),
                    space.raw(),
                ),
            );
            debug_assert!(
                false,
                "placement at {:#x}+{:#x} targets a shared view in space {}; \
                 cross-space replication of a placement is not modeled",
                range.start().raw(),
                range.length(),
                space.raw(),
            );
        }
        Ok(cleared)
    }
}

#[cfg(test)]
#[path = "tests/host_write_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/place_bytes_tests.rs"]
mod place_bytes_tests;

#[cfg(test)]
#[path = "tests/published_step_records_tests.rs"]
mod published_step_records_tests;
