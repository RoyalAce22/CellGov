//! Shared `commit_bytes_at` helper used by LV2 dispatch, PPU thread
//! creation, and sync wake resolution to write small continuation
//! payloads through a caller-supplied pointer.

use super::Runtime;

impl Runtime {
    /// Commit `bytes` to `ptr` through guest memory. Fails silently on
    /// an invalid guest range; callers that need to distinguish "bad
    /// pointer" from "committed" must use `GuestMemory::apply_commit`.
    pub(super) fn commit_bytes_at(&mut self, ptr: u64, bytes: &[u8]) {
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ptr), bytes.len() as u64)
        {
            let _ = self.memory.apply_commit(range, bytes);
        }
    }
}
