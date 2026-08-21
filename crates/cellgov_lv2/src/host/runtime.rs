//! The runtime-side contract the host consumes during dispatch.
//!
//! `Lv2Runtime` is the host's input view; [`crate::dispatch::Lv2Dispatch`]
//! is its output.

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

    /// The exclusive end of a committed region intersecting
    /// `[addr, addr + size)` in the CALLER's address space, `None`
    /// when the window is free. When several regions intersect, any
    /// of their ends satisfies the contract; the largest lets a
    /// caller skip furthest.
    ///
    /// `sys_mmapper_map_shared_memory` refuses an occupied window
    /// with `CELL_EBUSY` (RPCS3 sys_mmapper.cpp: the window
    /// allocation fails) and `sys_mmapper_search_and_map` skips it,
    /// so an implementor without a region model (fixed-layout test
    /// doubles) answers `None`.
    ///
    /// # Contract
    /// `Some(end)` is either the exclusive end of a region the window
    /// really intersects or `u64::MAX` when `addr + size` overflows;
    /// both lie above any `addr` a search reaches. Search loops
    /// advance to `end`, so an answer at or below `addr` makes no
    /// progress; `mmapper_search_free_range` treats one as "no
    /// overlap" and names the violation in a debug assertion rather
    /// than spinning.
    fn committed_overlap_end(&self, addr: u64, size: u64) -> Option<u64>;
}
