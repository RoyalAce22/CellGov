//! Cell Broadband Engine PPU constants: timebase, cache geometry,
//! register-file sizes and the real-address limit.
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
/// stride PS3 atomic primitives assume.
// [CBE-Handbook p:135 s:6.1] The coherence block equals the 128-byte cache-line
// size for every PPE cache, so the reservation granule is 128 bytes.
pub const RESERVATION_LINE_BYTES: u64 = 128;

/// `dcbz` block size on the Cell PPU: the implementation's data cache
/// line, which here equals [`RESERVATION_LINE_BYTES`].
// [PPC-Book2 p:20 s:3.2 Cache Management Instructions] dcbz block is implementation-defined.
// [CBE-Handbook p:135 s:6.1] PPE L1 DCache cache-line size is 128 bytes; coherence block matches.
pub const DCBZ_BLOCK_BYTES: usize = 128;

/// Number of PPU general-purpose registers (r0..r31).
// [PPC-Book1 p:31 s:3.2.1] The Fixed-Point Processor's principal internal
// storage is 32 General Purpose Registers, each 64 bits wide.
pub const GPR_COUNT: usize = 32;

/// Number of PPU floating-point registers (f0..f31).
// [PPC-Book1 p:86 s:4.2.1] Implementations provide 32 floating-point registers
// numbered 0-31, each holding 64 bits.
pub const FPR_COUNT: usize = 32;

/// Number of PPU vector (AltiVec / VMX) registers (v0..v31).
// [AltiVec-PEM p:2-4 s:2.3.1] VRF: 32 vector registers, each 128 bits wide.
pub const VR_COUNT: usize = 32;

/// Highest guest address that can be backed by real storage (2^42 - 1).
///
/// Not a bound on address arithmetic: a program's effective addresses
/// span the full 64-bit range. Callers use this to reject an address
/// no PS3 storage mapping could ever satisfy.
// [CBE-Handbook p:51 s:2.1] PPE MMU address-space sizes: real address 2^42
// bytes, effective address 2^64 bytes, virtual address 2^65 bytes.
pub const CELL_EA_LIMIT: u64 = 0x0000_03FF_FFFF_FFFF;
