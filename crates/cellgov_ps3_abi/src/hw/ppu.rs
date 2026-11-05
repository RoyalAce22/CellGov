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
///
/// The reference console's `sys_time_get_timebase_frequency` returns
/// this value, and a one-second `sys_timer_sleep` there takes 79,802,891
/// timebase ticks, which measures the same frequency
/// (`lv2_timer_time_tty`).
pub const CELL_PPU_TIMEBASE_HZ: u64 = 79_800_000;

/// The time base at guest time 0: CellGov's choice.
///
/// [PPC-Book2 p:29 s:4] The time base is a 64-bit count that increments
/// periodically, at an implementation-dependent rate.
///
/// The reference console counts from power-on: a title's first read there
/// held more than ten hours of ticks (`ppu_timebase_reads`). CellGov starts
/// the count when the run starts, so a title reads the time since its own
/// start.
pub const PPU_TIME_BASE_ORIGIN: u64 = 0;

/// Gives the effective-address offset of the PowerPC System Call interrupt vector.
///
/// [PPC-Book3 p:73 s:5.5.13] A System Call interrupt resumes at effective address 0xC00.
pub const SYSTEM_CALL_VECTOR_OFFSET: u64 = 0xC00;

/// The PPE's hardware threads: how many PPU threads run at once.
///
/// [CBE-Handbook p:295 s:10] to software, the PPE appears to provide two independent instruction-processing units.
pub const PPE_HARDWARE_THREADS: usize = 2;

/// Cell BE PPU L1/L2 cache line size in bytes. Reservation
/// granularity for `lwarx`/`stwcx.`, dcbz target alignment, and the
/// stride PS3 atomic primitives assume.
///
/// [CBE-Handbook p:135 s:6.1] The coherence block equals the 128-byte cache-line
/// size for every PPE cache, so the reservation granule is 128 bytes.
///
/// The reference console agrees (`ppu_granule_nan`). Another thread's
/// store 64 bytes past a reserved line's start broke the reservation in
/// each of 32 trials, and a store 128 bytes past it in none.
pub const RESERVATION_LINE_BYTES: u64 = 128;

/// `dcbz` block size on the Cell PPU: the implementation's data cache
/// line, which here equals [`RESERVATION_LINE_BYTES`].
///
/// [PPC-Book2 p:20 s:3.2 Cache Management Instructions] dcbz block is implementation-defined.
/// [CBE-Handbook p:135 s:6.1] PPE L1 DCache cache-line size is 128 bytes; coherence block matches.
///
/// The reference console agrees (`ppu_granule_nan`): one `dcbz` 40 bytes
/// into a 128-byte-aligned block zeroes exactly that block's 128 bytes.
pub const DCBZ_BLOCK_BYTES: usize = 128;

/// Number of PPU general-purpose registers (r0..r31).
///
/// [PPC-Book1 p:31 s:3.2.1] The Fixed-Point Processor's principal internal
/// storage is 32 General Purpose Registers, each 64 bits wide.
pub const GPR_COUNT: usize = 32;

/// Number of PPU floating-point registers (f0..f31).
///
/// [PPC-Book1 p:86 s:4.2.1] Implementations provide 32 floating-point registers
/// numbered 0-31, each holding 64 bits.
pub const FPR_COUNT: usize = 32;

/// Number of PPU vector (AltiVec / VMX) registers (v0..v31).
///
/// [AltiVec-PEM p:2-4 s:2.3.1] VRF: 32 vector registers, each 128 bits wide.
pub const VR_COUNT: usize = 32;

/// VSCR non-Java mode bit (VSCR bit 15).
///
/// [AltiVec-PEM p:2-5 s:2.3.2] The VSCR defines two bits: NJ is VSCR bit 15 and SAT is VSCR bit 31.
pub const VSCR_NJ: u32 = 0x0001_0000;

/// VSCR saturation bit (VSCR bit 31).
///
/// [AltiVec-PEM p:2-5 s:2.3.2] The VSCR defines two bits: NJ is VSCR bit 15 and SAT is VSCR bit 31.
pub const VSCR_SAT: u32 = 0x0000_0001;

/// The VSCR bits the PPE stores. Every other bit is reserved.
///
/// [AltiVec-PEM p:2-5 s:2.3.2] A reserved bit last written as 1 reads back as 0 or 1.
///
/// The reference console reads every reserved bit back as 0, after a
/// write of all ones and after a write of 0xA5A5A5A5 (`ppu_vscr_sat`).
pub const VSCR_DEFINED_BITS: u32 = VSCR_NJ | VSCR_SAT;

/// The VSCR of a PPU thread at its first instruction.
///
/// [AltiVec-PEM p:2-4 s:2.3.2] The VSCR reset value is implementation specific.
///
/// The reference console starts each PPU thread at NJ = 1 and SAT = 0
/// (`ppu_vscr_mode`). The main thread and a created thread both start
/// at this value, and a created thread does not take its creator's NJ.
pub const PPU_THREAD_ENTRY_VSCR: u32 = VSCR_NJ;

/// Every FPR of a PPU thread at its first instruction: CellGov's choice.
///
/// The reference console leaves one value in all 32 FPRs at a thread's
/// first instruction, and the value differs from thread to thread
/// (`ppu_entry_state`). No document names it, so CellGov starts each FPR
/// at 0.
pub const PPU_THREAD_ENTRY_FPR: u64 = 0;

/// The MSR bits a PPU thread's own code can observe, one field per bit.
///
/// [PPC-Book3 p:10 s:2.2.3] The MSR defines the state of the processor.
/// [PPC-Book3 p:23 s:3.4] `mfmsr`, which reads it, is privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserMsr {
    /// 64-bit mode.
    ///
    /// [PPC-Book3 p:10 s:2.2.3] SF = 1: the processor is in 64-bit mode.
    ///
    /// `ppu_user_msr` shows it on the main thread and on a created thread.
    /// `addic` of 0xFFFF_FFFF + 1 leaves CA clear, and an `add.` with the
    /// result 0x8000_0000 sets CR0 GT.
    pub sf: bool,
    /// Problem state.
    ///
    /// [PPC-Book3 p:10 s:2.2.3] PR = 1: the processor is in problem state.
    ///
    /// The console ends a process that runs `mfmsr` or another privileged
    /// instruction (`ppu_may_trap_*`).
    pub pr: bool,
    /// Floating-point available.
    ///
    /// [PPC-Book3 p:10 s:2.2.3] FP = 0: the processor executes no
    /// floating-point instruction, load, store or move.
    ///
    /// A thread's first FP instruction retires (`ppu_user_msr`). The
    /// kernel can start a thread with FP off and set it at the first use.
    /// The thread sees that only as a difference in time.
    pub fp: bool,
    /// AltiVec available.
    ///
    /// [AltiVec-PEM p:2-8 s:2.5.1] The VEC field is the AltiVec available
    /// bit of the MSR.
    ///
    /// A thread's first vector instruction retires (`ppu_user_msr`), with
    /// the same caveat as [`UserMsr::fp`].
    pub vec: bool,
    /// Floating-point exception mode bit 0.
    ///
    /// [PPC-Book3 p:11 s:2.2.3] FE0 = FE1 = 0 is Ignore Exceptions mode.
    ///
    /// With FPSCR VE = 1, an `fdiv` of 0 / 0 on either thread of
    /// `ppu_user_msr` does three things:
    ///
    /// - It sets FEX.
    /// - It leaves its target unchanged.
    /// - It lets the thread continue.
    pub fe0: bool,
    /// Floating-point exception mode bit 1. See [`UserMsr::fe0`].
    pub fe1: bool,
    /// Little-endian mode.
    ///
    /// [PPC-Book3 p:11 s:2.2.3] LE = 0: the processor is in big-endian
    /// mode.
    ///
    /// Every console capture reads a stored word big-endian.
    pub le: bool,
}

/// The MSR a PS3 user thread runs under: 64-bit, problem state, FP and
/// AltiVec available, Ignore Exceptions mode, big-endian.
pub const PPU_USER_MSR: UserMsr = UserMsr {
    sf: true,
    pr: true,
    fp: true,
    vec: true,
    fe0: false,
    fe1: false,
    le: false,
};

/// Highest guest address that can be backed by real storage (2^42 - 1).
///
/// Not a bound on address arithmetic: a program's effective addresses
/// span the full 64-bit range. Callers use this to reject an address
/// no PS3 storage mapping could ever satisfy.
///
/// [CBE-Handbook p:51 s:2.1] PPE MMU address-space sizes: real address 2^42
/// bytes, effective address 2^64 bytes, virtual address 2^65 bytes.
pub const CELL_EA_LIMIT: u64 = 0x0000_03FF_FFFF_FFFF;
