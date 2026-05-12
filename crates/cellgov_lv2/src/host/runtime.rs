//! The runtime-side contract the host consumes during dispatch.
//!
//! Mirrors the host's outward [`crate::dispatch::Lv2Dispatch`] response
//! direction: `Lv2Runtime` is the input view; `Lv2Dispatch` is the
//! output. Keeping the two on opposite sides of the host's surface
//! keeps the directionality readable at the call site.

use cellgov_time::GuestTicks;

/// Readonly view of runtime state exposed to the host during dispatch.
///
/// `current_tick` stamps LV2-sourced effects so they participate in
/// commit-pipeline ordering at the triggering syscall's tick rather
/// than tick 0.
pub trait Lv2Runtime {
    /// # Contract
    /// `Some(bytes)` must carry exactly `len` bytes; short reads are
    /// a trait violation. `None` means the range is out of bounds.
    fn read_committed(&self, addr: u64, len: usize) -> Option<&[u8]>;

    /// Current guest tick.
    fn current_tick(&self) -> GuestTicks;

    /// Read up to `max_len` bytes from `addr`, returning the prefix
    /// before the first `terminator` byte (terminator excluded).
    ///
    /// # Returns
    /// - `Some(bytes)` with `bytes.len() < max_len` when a terminator
    ///   is found within the first `max_len` mapped bytes.
    /// - `None` when `addr` is unmapped, no terminator appears within
    ///   `max_len` mapped bytes, or the address is in a
    ///   `ReservedStrict` region.
    fn read_committed_until(&self, addr: u64, max_len: usize, terminator: u8) -> Option<&[u8]>;

    /// True iff a `len`-byte write at `addr` lands entirely inside a
    /// single `ReadWrite` region.
    fn writable(&self, addr: u64, len: usize) -> bool;
}
