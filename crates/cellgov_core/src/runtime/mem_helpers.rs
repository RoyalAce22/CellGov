//! Shared `commit_bytes_at` helper used by LV2 dispatch, PPU thread
//! creation, and sync wake resolution to write small continuation
//! payloads through a caller-supplied pointer.

use super::spaces::AddressSpaceId;
use super::Runtime;

impl Runtime {
    /// Commit `bytes` to `ptr` in `space`'s memory. The pointer came
    /// from some unit's syscall arguments, so the target space is
    /// that unit's -- callers resolve it via `SpaceTable::space_of`.
    /// A bad pointer never commits, but it is loud: the range-overflow
    /// and validation-failure arms each log a named invariant break,
    /// because every caller has already returned success (or a staged
    /// wake code) to the guest and a dropped payload would otherwise
    /// read as fabricated success. Callers that need to branch on
    /// "bad pointer" must use `GuestMemory::apply_commit`.
    pub(super) fn commit_bytes_at(&mut self, space: AddressSpaceId, ptr: u64, bytes: &[u8]) {
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
        {
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
            let mem = super::spaces::resolve_space_memory_for_write(
                &mut self.memory,
                &mut self.spaces,
                space,
            );
            if let Err(err) = mem.apply_commit(range, bytes) {
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
}
