//! The PPE's issue timing as a retail console measures it.
//!
//! Every value is the `ppu_issue_timing` capture's (CECH-2001A, 4.93 CEX,
//! `tests/micro/ppu_issue_timing/ps3/cech20-cex-493/`). The probe times
//! each loop with `mftb` five times, and a value here is the fastest of
//! the five at [`PPU_CYCLES_PER_TB_TICK`], rounded to the nearest cycle.
//! The test `crates/cellgov_ppu/tests/it/console_ppu_issue_timing.rs`
//! derives each one from the capture again and replays the loops.

/// PPE core cycles per time-base tick.
///
/// At this ratio the fastest run of the empty `bdnz` loop reads 8.015
/// cycles an iteration, eight independent `addi` 12.04, and eight
/// dependent L1 loads 40.04: whole cycles to half a percent. A ratio of
/// 39 or 41 puts the empty loop at 7.8 or 8.2. The time base runs at
/// 79.8 MHz, so the core clock is 3.192 GHz.
pub const PPU_CYCLES_PER_TB_TICK: u64 = 40;

/// The cycles of one iteration of a loop that holds only its `bdnz`.
pub const PPU_LOOP_BRANCH_CYCLES: u64 = 8;

/// The cycles from a fixed-point `add` to a dependent one.
pub const PPU_FX_LATENCY_CYCLES: u64 = 2;

/// The cycles from a `mulld` to a dependent one.
pub const PPU_MULLD_LATENCY_CYCLES: u64 = 14;

/// The cycles from a `divd` of a 33-bit dividend by 1 to a dependent one.
///
/// The probe divides by 1 only, so this is the cost of that one operand
/// pair.
pub const PPU_DIVD_LATENCY_CYCLES: u64 = 48;

/// The cycles from a double-precision `fadd` or `fmadd` to a dependent one.
pub const PPU_FP_LATENCY_CYCLES: u64 = 10;

/// The cycles from a `vaddfp` to a dependent one.
pub const PPU_VMX_FLOAT_LATENCY_CYCLES: u64 = 12;

/// The cycles from a load that hits the L1 data cache to a load that uses
/// its result as an address.
pub const PPU_L1_LOAD_USE_CYCLES: u64 = 5;

/// [`PPU_L1_LOAD_USE_CYCLES`] for a chase over a shuffled 256 KB ring,
/// which the 32 KB L1 cannot hold and the 512 KB L2 can.
///
/// [CBE-Handbook p:135 s:6.1] the L1 data cache is 32 KB.
///
/// [CBE-Handbook p:139 s:6.1.5] the L2 cache is 512 KB.
pub const PPU_L2_LOAD_USE_CYCLES: u64 = 41;

/// [`PPU_L1_LOAD_USE_CYCLES`] for a chase over a shuffled 8 MB ring,
/// which neither cache can hold.
pub const PPU_MEMORY_LOAD_USE_CYCLES: u64 = 443;

/// The cycles a `std` takes with the `ld` of the same doubleword after it,
/// when the next `std` stores what the `ld` loaded.
pub const PPU_LOAD_HIT_STORE_CYCLES: u64 = 49;

/// The cycles an unconditional branch to the next instruction takes.
pub const PPU_TAKEN_BRANCH_CYCLES: u64 = 9;

/// The cycles a mispredicted conditional branch adds.
///
/// The probe runs one LCG step a round, then a branch on the LCG's top
/// bit, beside the same loop with a branch that is always taken. The
/// random loop costs 15.3 cycles more a round. This value assumes that
/// the predictor misses half the random branches: a predictor cannot do
/// better on bits that its history does not predict.
pub const PPU_BRANCH_MISS_CYCLES: u64 = 31;

/// The cycles one LV2 syscall takes from `sc` to the instruction after it:
/// `sys_time_get_timebase_frequency`, the cheapest call.
pub const PPU_SYSCALL_ROUND_TRIP_CYCLES: u64 = 949;

/// Eight independent `addi` and their `bdnz`, in cycles an iteration, with
/// the other hardware thread idle and then busy on independent `addi`.
pub const PPU_SMT_INDEPENDENT_ADDI_CYCLES: (u64, u64) = (12, 19);

/// Eight dependent `add` and their `bdnz`, in cycles an iteration, with
/// the other hardware thread idle and then busy.
pub const PPU_SMT_DEPENDENT_ADD_CYCLES: (u64, u64) = (16, 23);

/// Eight dependent L1 loads and their `bdnz`, in cycles an iteration,
/// with the other hardware thread idle and then busy.
pub const PPU_SMT_L1_CHASE_CYCLES: (u64, u64) = (40, 42);
