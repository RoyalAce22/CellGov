//! Shared `commit_bytes_at` helper: writes a small payload through a
//! pointer the guest supplied in its own syscall arguments.

use cellgov_event::UnitId;
use cellgov_trace::HostWriter;

use super::Runtime;

impl Runtime {
    /// Commit `bytes` to `ptr` in `unit`'s space on behalf of `writer`.
    ///
    /// The pointer came from that unit's own syscall arguments, so its
    /// space is the target.
    ///
    /// A bad pointer never commits, but it is loud: the range-overflow
    /// and validation-failure arms each log a named invariant break.
    /// Every caller returns success (or a staged wake code) to the
    /// guest before this runs, so a dropped payload would otherwise
    /// read as fabricated success. Callers that need to branch on a
    /// bad pointer must use `GuestMemory::apply_commit`.
    pub(super) fn commit_bytes_at(
        &mut self,
        writer: HostWriter,
        unit: UnitId,
        ptr: u64,
        bytes: &[u8],
    ) {
        let space = self.spaces.space_of(unit);
        let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ptr), bytes.len() as u64)
        else {
            self.lv2_host.log_invariant_break(
                "runtime.commit_bytes_at_range_overflow",
                format_args!(
                    "continuation payload at 0x{ptr:x}+0x{:x} overflows the guest \
                     address range; payload dropped, the guest already observed success",
                    bytes.len(),
                ),
            );
            return;
        };
        // Wake payloads bypass the commit pipeline's shared-view
        // fanout; one landing inside a shared view would leave
        // sibling views incoherent (same guard as the LV2
        // SharedWriteIntent path in apply_lv2_effects). The
        // invariant break keeps the witness loud in release
        // builds, where the debug_assert compiles out.
        if self.range_intersects_shared_view(space, range) {
            self.lv2_host.log_invariant_break(
                "runtime.commit_bytes_at_targets_shared_view",
                format_args!(
                    "wake payload at 0x{ptr:x}+0x{:x} targets a shared view in \
                     space {}; cross-space replication of direct commits is not \
                     modeled, sibling views are now incoherent",
                    bytes.len(),
                    space.raw(),
                ),
            );
            debug_assert!(
                false,
                "wake payload at {ptr:#x}+{:#x} targets a shared view in space {}; \
                 cross-space replication of direct commits is not modeled",
                bytes.len(),
                space.raw(),
            );
        }
        if let Err(err) = self.host_write(writer, space, range, bytes, Some(unit)) {
            self.lv2_host.log_invariant_break(
                "runtime.commit_bytes_at_write_failed",
                format_args!(
                    "continuation payload at 0x{ptr:x}+0x{:x} in space {} failed to \
                     commit: {err}; payload dropped, the guest already observed success",
                    bytes.len(),
                    space.raw(),
                ),
            );
        }
    }
}
