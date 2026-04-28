//! PS3 hardware constants (Cell Broadband Engine PPU).
//!
//! Reported to titles via `sys_time_get_timebase_frequency` and used
//! internally by reservation tracking, dcbz, and PowerPC atomic ops.
//! Behaviour (the timebase scheduler hooks, the reservation table)
//! lives in `cellgov_time` / `cellgov_sync`; this module is data only.

/// PPU timebase register frequency in Hz. Reported by
/// `sys_time_get_timebase_frequency` and used to convert between
/// guest-visible timebase ticks and microseconds.
pub const CELL_PPU_TIMEBASE_HZ: u64 = 79_800_000;

/// Cell BE PPU L1/L2 cache line size in bytes. Reservation
/// granularity for `lwarx`/`stwcx.`, dcbz target alignment, and the
/// stride PS3 atomic primitives assume. Architecturally fixed at 128
/// bytes by the CBE PPU specification.
pub const RESERVATION_LINE_BYTES: u64 = 128;
