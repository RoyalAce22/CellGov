//! Thin helpers around `GuestMemory::apply_commit`.
//!
//! The pattern
//!
//! ```ignore
//! if let Some(range) = cellgov_mem::ByteRange::new(
//!     cellgov_mem::GuestAddr::new(ptr as u64),
//!     bytes.len() as u64,
//! ) {
//!     let _ = self.memory.apply_commit(range, &bytes);
//! }
//! ```
//!
//! shows up across LV2 dispatch, PPU thread creation, and sync
//! wake resolution -- anywhere the runtime writes a small
//! continuation payload through a caller-supplied pointer. The
//! helper on this module collapses it to a single call so the
//! surrounding logic stays readable.

use super::Runtime;

impl Runtime {
    /// Commit `bytes` to `ptr` via the runtime's guest-memory view.
    ///
    /// Fails silently if `ptr..(ptr + bytes.len())` is not a valid
    /// guest range, matching the prior inline pattern. Callers that
    /// need to distinguish "bad pointer" from "committed" must go
    /// through `GuestMemory::apply_commit` directly.
    pub(super) fn commit_bytes_at(&mut self, ptr: u64, bytes: &[u8]) {
        if let Some(range) =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(ptr), bytes.len() as u64)
        {
            let _ = self.memory.apply_commit(range, bytes);
        }
    }
}
