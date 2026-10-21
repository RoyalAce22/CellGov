//! The PPE's issue timing: the measured terms and the documented ones.
//!
//! The measured values come from the `ppu_issue_timing` and
//! `ppu_issue_table` captures (CECH-2001A, 4.93 CEX,
//! `tests/micro/<probe>/ps3/cech20-cex-493/`). Each probe times each loop
//! with `mftb` five times. A measured value is one of:
//!
//! - the fastest of the five at [`PPU_CYCLES_PER_TB_TICK`], rounded to the
//!   nearest cycle;
//! - a term of the PPU's issue model that the item's doc ties to a loop's
//!   fastest run.
//!
//! The test `crates/cellgov_ppu/tests/it/console_ppu_issue_timing.rs`
//! derives the `ppu_issue_timing` values of the first kind again. It also
//! replays the loops of both probes through the issue model. The
//! documented values carry the page that states them.

/// PPE core cycles per time-base tick.
///
/// At this ratio the fastest run of the empty `bdnz` loop reads 8.015
/// cycles an iteration, eight independent `addi` 12.04, and eight
/// dependent L1 loads 40.04: whole cycles to half a percent. A ratio of
/// 39 or 41 puts the empty loop at 7.8 or 8.2. The time base runs at
/// 79.8 MHz, so the core clock is 3.192 GHz.
pub const PPU_CYCLES_PER_TB_TICK: u64 = 40;

/// The cycles from the fetch that holds a predicted-taken branch to the
/// fetch at its target.
///
/// The empty `bdnz` loop measures it: one fetch an iteration.
///
/// [CBE-Handbook p:299 s:10.2.3] starting the fetch of a new sequential
/// stream after a predicted-taken branch requires eight cycles.
pub const PPU_TAKEN_BRANCH_FETCH_CYCLES: u64 = 8;

/// The cycles from the fetch that holds a predicted-not-taken branch to
/// the fetch of the instruction after it.
///
/// Eight compares, each with a `bne` that falls through, and the loop's
/// `bdnz` take 39.1 cycles an iteration: four cycles a branch, and the
/// eight of the loop branch.
pub const PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES: u64 = 4;

/// The cycles from a fixed-point `add` to a dependent one.
pub const PPU_FX_LATENCY_CYCLES: u64 = 2;

/// The cycles from a `mulld` to a dependent one, and the cycles the
/// multiply holds the fixed-point unit.
///
/// The handbook gives 15; the console's dependent `mulld` takes 14.
///
/// [CBE-Handbook p:718 s:A.1.2] `mulld mulhd mulhdu mulldo`: latency 15;
/// the FXU stalls 15 cycles after issue.
pub const PPU_MULLD_LATENCY_CYCLES: u64 = 14;

/// The cycles a dependent instruction waits for a divide beyond the
/// handbook's formula for the divide.
///
/// The probe's dependent `divd` of a 33-bit dividend by 1 takes 48
/// cycles, and the formula gives 38: a dependent instruction refetches
/// while the divide runs.
pub const PPU_DIVIDE_REFETCH_CYCLES: u64 = 10;

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

/// The cycles from a store to the load of the same bytes after it,
/// while the store is still in the store queue.
///
/// Four `std` and `ld` pairs on one doubleword, each `std` storing what
/// the `ld` before it loaded, take 194.5 cycles an iteration.
pub const PPU_LOAD_HIT_STORE_CYCLES: u64 = 49;

/// The bytes of one store-queue entry.
///
/// [CBE-Handbook p:134 s:6.1.2.2] the store subunit contains a store
/// queue (STQ, 8 x 16-byte entries).
pub const PPU_STORE_ENTRY_BYTES: u64 = 16;

/// The cycles between two store-queue entries writing their bytes into
/// the cache.
///
/// Fitted to the `ppu_issue_multiple` capture's `stmw` and `lmw` pairs
/// on one line. The pairs of 4, 8 and 16 words take 85.94, 139.34 and
/// 174.53 cycles an iteration, and with this interval the issue model
/// gives 82, 120 and 196. No one interval fits both the 8- and 16-word
/// pairs, and 30 stays within 22 cycles of each.
pub const PPU_STORE_DRAIN_CYCLES: u64 = 30;

/// The cycles from a mispredicted branch's issue to the earliest issue
/// of the instruction at its correct target.
///
/// The probe runs one LCG step a round, then a branch on the LCG's top
/// bit, beside the same loop with a branch that is always taken. The
/// replay of the random loop through the issue model and its branch
/// history table takes the console's 38.7 cycles an iteration at this
/// penalty.
pub const PPU_BRANCH_MISS_CYCLES: u64 = 25;

/// The cycles from an `sc` to the earliest issue of the instruction
/// after it: the LV2 round trip of `sys_time_get_timebase_frequency`,
/// the cheapest call.
///
/// The probe's loop of these calls takes 949 cycles an iteration. The
/// replay of that loop through the issue model takes the console's
/// cycles at this round trip.
pub const PPU_SYSCALL_ROUND_TRIP_CYCLES: u64 = 929;

/// Eight independent `addi` and their `bdnz`, in cycles an iteration, with
/// the other hardware thread idle and then busy on independent `addi`.
pub const PPU_SMT_INDEPENDENT_ADDI_CYCLES: (u64, u64) = (12, 19);

/// Eight dependent `add` and their `bdnz`, in cycles an iteration, with
/// the other hardware thread idle and then busy.
pub const PPU_SMT_DEPENDENT_ADD_CYCLES: (u64, u64) = (16, 23);

/// Eight dependent L1 loads and their `bdnz`, in cycles an iteration,
/// with the other hardware thread idle and then busy.
pub const PPU_SMT_L1_CHASE_CYCLES: (u64, u64) = (40, 42);

/// The cycles a runnable sibling thread holds issue after each of a
/// thread's dispatch groups.
///
/// A group is the pair of instructions at one 8-byte-aligned address,
/// whether or not the two issue together. An instruction after a
/// microcoded one opens a group of its own: the `ppu_issue_smt`
/// capture's loop of `lha` and `addi` pairs takes 107.66 cycles an
/// iteration beside a busy sibling and 90.26 alone. The 17.4 between
/// is near the nine holds, 18 cycles, that the model takes.
///
/// With this hold, the issue model times the three busy-sibling loops
/// of `ppu_issue_timing` at 19, 23 and 41 cycles an iteration against
/// the console's 19.24, 23.34 and 41.52: each inside the console's
/// spread.
///
/// [CBE-Handbook p:754 s:A.6.1] the threads take dispatch in turn, a
/// thread every other cycle.
pub const PPU_SMT_SIBLING_HOLD_CYCLES: u64 = 2;

/// The cycles between two fetches for one hardware thread.
///
/// [CBE-Handbook p:299 s:10.2.3] instruction fetching strictly
/// alternates between the two threads on successive cycles.
pub const PPU_FETCH_INTERVAL_CYCLES: u64 = 2;

/// The bytes one fetch reads: four instructions.
///
/// [CBE-Handbook p:135 s:6.1.3.2] the PPE can fetch up to four
/// instructions per cycle.
pub const PPU_FETCH_BYTES: u64 = 16;

/// The instructions a thread's fetch can run ahead of its issue.
///
/// The instruction buffer holds four instructions, and each of the seven
/// stages from the buffer's load to issue holds two.
///
/// [CBE-Handbook p:754 s:A.6.1] IB1 loads the instruction buffer and
/// IB2 unloads it.
///
/// [CBE-Handbook p:755 s:A.6.1] decode takes ID1 to ID3, and issue IS1
/// to IS2.
pub const PPU_FETCH_AHEAD_INSTRUCTIONS: u64 = 18;

/// The cycles from a compare to a dependent branch or CR operation.
///
/// [CBE-Handbook p:719 s:A.1.2] `cmpi cmp cmpli cmpl`: latency 1.
pub const PPU_COMPARE_LATENCY_CYCLES: u64 = 1;

/// The cycles from a branch-unit instruction to a dependent one.
///
/// The branch-unit instructions are:
///
/// - a branch's update of CTR or LR;
/// - a CR logical operation;
/// - a move to or from LR or CTR.
///
/// [CBE-Handbook p:715 s:A.1.2] `bc`: latency 1 (lr / ctr).
pub const PPU_BRANCH_UNIT_LATENCY_CYCLES: u64 = 1;

/// The latency of a `mulli`, and the cycles it holds the fixed-point unit.
///
/// The handbook gives 6; eight dependent `mulli` and their `bdnz` take
/// 65.8 cycles an iteration on the console.
///
/// [CBE-Handbook p:718 s:A.1.2] `mulli`: latency 6; the FXU stalls 6
/// cycles after issue.
pub const PPU_MULLI_LATENCY_CYCLES: u64 = 8;

/// The latency of a word multiply, and the cycles it holds the
/// fixed-point unit.
///
/// The handbook gives 9; eight dependent `mullw` and their `bdnz` take
/// 90.8 cycles an iteration on the console.
///
/// [CBE-Handbook p:718 s:A.1.2] `mullw mulhw mulhwu mullwo`: latency 9;
/// the FXU stalls 9 cycles after issue.
pub const PPU_MULLW_LATENCY_CYCLES: u64 = 11;

/// The fewest cycles a divide takes.
///
/// [CBE-Handbook p:718 s:A.1.2] word and doubleword minimum 10; an
/// overflow case completes in 10.
pub const PPU_DIVIDE_MIN_CYCLES: u64 = 10;

/// The most cycles a word divide takes.
///
/// [CBE-Handbook p:718 s:A.1.2] word maximum 38.
pub const PPU_DIVIDE_WORD_MAX_CYCLES: u64 = 38;

/// The most cycles a doubleword divide takes.
///
/// [CBE-Handbook p:718 s:A.1.2] doubleword maximum 70.
pub const PPU_DIVIDE_DOUBLEWORD_MAX_CYCLES: u64 = 70;

/// The cycles from a fixed-point load that hits the L1 to a dependent
/// fixed-point operation.
///
/// [CBE-Handbook p:716 s:A.1.2] 2-cycle latency for a load followed by
/// a dependent FXU operation.
pub const PPU_LOAD_FX_USE_CYCLES: u64 = 2;

/// The cycles from a floating-point load that hits the L1 to a
/// dependent floating-point instruction.
///
/// [CBE-Handbook p:720 s:A.1.2] `lfs lfsx lfd lfdx`: latency 1.
pub const PPU_FP_LOAD_LATENCY_CYCLES: u64 = 1;

/// The cycles a double-precision divide takes and holds the
/// floating-point unit.
///
/// [CBE-Handbook p:721 s:A.1.2] `fdiv fdivs` (IEEE): latency 74.
///
/// [CBE-Handbook p:754 s:A.5] no FPU instruction, loads and stores
/// included, issues after an `fdiv` or `fsqrt` until it completes.
pub const PPU_FDIV_CYCLES: u64 = 74;

/// The cycles a square root takes and holds the floating-point unit.
///
/// [CBE-Handbook p:721 s:A.1.2] `fsqrt fsqrts` (IEEE): latency 84.
pub const PPU_FSQRT_CYCLES: u64 = 84;

/// The cycles a recording floating-point instruction's CR field takes
/// beyond its result.
///
/// The handbook gives 17. Eight dependent `fadd.`, each of which writes
/// CR1 again, and their `bdnz` take 238.9 cycles an iteration on the
/// console: 30 cycles each.
///
/// [CBE-Handbook p:721 s:A.1.2] +17 (cr).
pub const PPU_FP_RECORD_EXTRA_CYCLES: u64 = 20;

/// The cycles from a move from the FPSCR to a dependent instruction.
///
/// Eight `mffs` to one register and their `bdnz` take 248.2 cycles an
/// iteration on the console.
///
/// [CBE-Handbook p:722 s:A.1.2] `mffs`: 11 to 28, stalled until older
/// vector-scalar operations complete.
pub const PPU_MFFS_LATENCY_CYCLES: u64 = 31;

/// The cycles a move from the CR takes and holds the branch unit.
///
/// The handbook gives about 34; eight `mfcr` to one register and their
/// `bdnz` take 212.6 cycles an iteration on the console.
///
/// [CBE-Handbook p:720 s:A.1.2] `mfcr`, `mfocrf`: about 34, one per 34
/// cycles.
pub const PPU_MFCR_CYCLES: u64 = 27;

/// The cycles from a move from the time base to a dependent instruction.
///
/// The handbook gives about 40; eight `mftb` to one register and their
/// `bdnz` take 365.6 cycles an iteration on the console.
///
/// [CBE-Handbook p:720 s:A.1.2] `mftb`: about 40.
pub const PPU_MFTB_LATENCY_CYCLES: u64 = 46;

/// The microwords of an `lha`.
///
/// The handbook gives 7. Four independent `lha` from one L1 line and
/// their `bdnz` take 86.6 cycles an iteration on the console: 21 cycles
/// each, the microcode ROM's 11 and 10 microwords.
///
/// [CBE-Handbook p:729 s:A.1.3.1] `lha`: 7 microwords.
pub const PPU_LHA_MICROWORDS: u8 = 10;

/// The cycles before the first microword of a microcoded instruction
/// issues.
///
/// [CBE-Handbook p:724 s:A.1.2] a minimum of 11 cycles is required
/// before the first instruction is received from the microcode ROM.
pub const PPU_MICROCODE_ROM_CYCLES: u64 = 11;

/// The cycles from a vector load to a dependent instruction.
///
/// [CBE-Handbook p:740 s:A.3.2] vector loads: latency 2.
pub const PPU_VMX_LOAD_LATENCY_CYCLES: u64 = 2;

/// The cycles from a vector permute-class instruction to a dependent
/// instruction.
///
/// The permute class is:
///
/// - permute;
/// - pack and unpack;
/// - merge;
/// - splat.
///
/// [CBE-Handbook p:740 s:A.3.2] permute: latency 4.
pub const PPU_VMX_PERMUTE_LATENCY_CYCLES: u64 = 4;

/// The cycles from a simple vector operation to a dependent instruction.
///
/// The simple operations are:
///
/// - integer add;
/// - logical and select;
/// - rotate and shift;
/// - compare;
/// - minimum and maximum.
///
/// [CBE-Handbook p:741 s:A.3.2] simple: latency 4.
pub const PPU_VMX_SIMPLE_LATENCY_CYCLES: u64 = 4;

/// The cycles from a vector reciprocal estimate to a dependent
/// instruction.
///
/// [CBE-Handbook p:742 s:A.3.2] `vrefp vrsqrtefp`: latency 14.
pub const PPU_VMX_ESTIMATE_LATENCY_CYCLES: u64 = 14;

/// The cycles from a vector multiply or multiply-sum to a dependent
/// instruction.
///
/// [CBE-Handbook p:743 s:A.3.2] complex: latency 9.
pub const PPU_VMX_COMPLEX_LATENCY_CYCLES: u64 = 9;

/// The ways of the L1 data cache.
///
/// [CBE-Handbook p:135 s:6.1] 32 KB, 4-way set-associative, 128-byte
/// lines, write-through; a store that misses allocates no line.
pub const PPU_L1D_WAYS: usize = 4;

/// The sets of the L1 data cache: 32 KB over four ways of 128-byte
/// lines.
pub const PPU_L1D_SETS: usize = 64;

/// The ways of the L2 cache.
///
/// [CBE-Handbook p:139 s:6.1.5] 512 KB, 8-way set-associative, 128-byte
/// lines, allocation on store miss.
pub const PPU_L2_WAYS: usize = 8;

/// The sets of the L2 cache: 512 KB over eight ways of 128-byte lines.
pub const PPU_L2_SETS: usize = 512;

/// The bytes of one cache line.
///
/// [CBE-Handbook p:135 s:6.1] 128-byte cache-line size.
pub const PPU_CACHE_LINE_BYTES: u64 = 128;

/// The entries of the store queue, which the threads share.
///
/// [CBE-Handbook p:135 s:6.1] 16-entry store queue.
pub const PPU_STORE_QUEUE_ENTRIES: usize = 16;

/// The entries of a thread's branch history table.
///
/// [CBE-Handbook p:139 s:6.1.4] 4096 x 2-bit entries and 6 bits of
/// global branch history, and a 4-entry link stack, per thread.
pub const PPU_BHT_ENTRIES: usize = 4096;

/// The bits of a thread's global branch history.
pub const PPU_GLOBAL_HISTORY_BITS: u32 = 6;

/// The entries of a thread's link stack.
pub const PPU_LINK_STACK_ENTRIES: usize = 4;
