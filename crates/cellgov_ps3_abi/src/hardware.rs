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

/// `dcbz` block size on the Cell PPU. The dcbz block is the
/// implementation's data cache line; on Cell PPU this matches
/// [`RESERVATION_LINE_BYTES`]. Both are forced equal by the CBE
/// PPU spec but carry separate names so call sites stay
/// semantically clear (cache-zero target vs. reservation granule).
// [PPC-Book2 p:20 s:3.2 Cache Management Instructions] dcbz block is implementation-defined.
// [CBE-Handbook p:135 s:6.1] PPE L1 DCache cache-line size is 128 bytes; coherence block matches.
pub const DCBZ_BLOCK_BYTES: usize = 128;

/// Number of PPU general-purpose registers (r0..r31).
// [PPC-Book1 p:41 s:3.2.1] 32 General Purpose Registers (GPRs).
pub const GPR_COUNT: usize = 32;

/// Number of PPU floating-point registers (f0..f31).
// [PPC-Book1 p:97 s:4.2 Figure 27] 32 Floating-Point Registers (FPRs).
pub const FPR_COUNT: usize = 32;

/// Number of PPU vector (AltiVec / VMX) registers (v0..v31).
// [AltiVec-PEM p:40 s:2.3.1] VRF: 32 vector registers, each 128 bits wide.
pub const VR_COUNT: usize = 32;

/// Cell BE effective-address space upper bound (42 bits).
// [CBE-Handbook p:75 s:4.5.1]
pub const CELL_EA_LIMIT: u64 = 0x0000_03FF_FFFF_FFFF;

/// SPU local store size in bytes (256 KiB).
// [CBEA p:79 s:6.2] Local Store is 256 KB.
pub const SPU_LS_SIZE: usize = 256 * 1024;

/// Number of SPU general-purpose 128-bit registers (r0..r127).
// [SPU-ISA p:5 s:1.4] 128 GPRs, each 128 bits wide.
pub const SPU_REG_COUNT: usize = 128;
