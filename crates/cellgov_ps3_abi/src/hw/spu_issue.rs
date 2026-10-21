//! The SPU's issue timing: each instruction's pipeline, latency and stall
//! from the CBE instruction table, the SPU clock against the time base,
//! and the issue costs a retail console measures.
//!
//! [CBE-Handbook p:763 s:B.1.2] the SPU issues up to two instructions a cycle, one in each of the even and odd pipelines; Table B-1 names each instruction's pipeline, latency and stall cycles.
//!
//! The measured values are the `spu_issue_timing` capture's (CECH-2001A,
//! 4.93 CEX, `tests/micro/spu_issue_timing/ps3/cech20-cex-493/`): loops
//! of 65536 iterations, each timed with the SPU decrementer. The test
//! `crates/cellgov_spu/tests/it/console_issue_timing.rs` holds the model
//! to every loop of the capture.

use super::spu_isa::{row_named, SPU_OPCODE_MAP};

/// SPU clock cycles per time-base tick.
///
/// At this ratio, every loop of the `spu_issue_timing` capture reads a
/// whole number of cycles per iteration, to one part in a thousand.
/// Examples are 4, 21, 28 and 41. The time base runs at 79.8 MHz, so the SPU clock
/// is 3.192 GHz.
pub const SPU_CYCLES_PER_TB_TICK: u64 = 40;

/// An SPU execution pipeline.
///
/// [CBE-Handbook p:763 s:B.1.2] pipeline 0 is the even pipeline, pipeline 1 the odd.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuPipe {
    /// Pipeline 0: fixed point, floating point, byte operations and
    /// word shifts.
    Even,
    /// Pipeline 1: loads and stores, branches and hints, channels,
    /// shuffles and quadword rotates.
    Odd,
}

/// One instruction's row of the CBE instruction table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuIssue {
    /// The pipeline the instruction issues to.
    pub pipe: SpuPipe,
    /// The cycles from issue until the result is available to a later
    /// instruction, or 0 for an instruction with no register result.
    ///
    /// [CBE-Handbook p:764 s:B.1.2 Table B-1] the latency includes the stall cycles.
    pub latency: u8,
    /// The cycles after issue in which no instruction issues.
    ///
    /// [CBE-Handbook p:774 s:B.1.3] no instruction issues for six cycles after a double-precision instruction, and none dual-issues with it.
    pub stall: u8,
}

const fn even(latency: u8) -> SpuIssue {
    SpuIssue {
        pipe: SpuPipe::Even,
        latency,
        stall: 0,
    }
}

const fn odd(latency: u8) -> SpuIssue {
    SpuIssue {
        pipe: SpuPipe::Odd,
        latency,
        stall: 0,
    }
}

/// A double-precision instruction: even pipeline, 13 cycles, 6 stall.
const DOUBLE: SpuIssue = SpuIssue {
    pipe: SpuPipe::Even,
    latency: 13,
    stall: 6,
};

/// Table B-1 by mnemonic.
///
/// A latency of 0 marks an instruction with no register result, or one
/// that the table gives as N/A. These classes have a latency of 0:
///
/// - branches that write no link register, and hints
/// - stores and channel writes
/// - halts and the synchronizing instructions
///
/// [CBE-Handbook p:764 s:B.1.2 Table B-1] sheets 1 to 8, pages 764 to 771.
const TABLE_B1: &[(&str, SpuIssue)] = &[
    ("a", even(2)),
    ("absdb", even(4)),
    ("addx", even(2)),
    ("ah", even(2)),
    ("ahi", even(2)),
    ("ai", even(2)),
    ("and", even(2)),
    ("andbi", even(2)),
    ("andc", even(2)),
    ("andhi", even(2)),
    ("andi", even(2)),
    ("avgb", even(4)),
    ("bg", even(2)),
    ("bgx", even(2)),
    ("bi", odd(0)),
    ("bihnz", odd(0)),
    ("bihz", odd(0)),
    ("binz", odd(0)),
    ("bisl", odd(4)),
    ("bisled", odd(4)),
    ("biz", odd(0)),
    ("br", odd(0)),
    ("bra", odd(0)),
    // Table B-1 gives N/A for brasl's and brsl's latency; the link
    // register takes the branch latency of Table 24-2.
    // [CBE-Handbook p:691 s:24.3 Table 24-2] pipeline 1 branches have a latency of 4.
    ("brasl", odd(4)),
    ("brhnz", odd(0)),
    ("brhz", odd(0)),
    ("brnz", odd(0)),
    ("brsl", odd(4)),
    ("brz", odd(0)),
    ("cbd", odd(4)),
    ("cbx", odd(4)),
    ("cdd", odd(4)),
    ("cdx", odd(4)),
    ("ceq", even(2)),
    ("ceqb", even(2)),
    ("ceqbi", even(2)),
    ("ceqh", even(2)),
    ("ceqhi", even(2)),
    ("ceqi", even(2)),
    ("cflts", even(7)),
    ("cfltu", even(7)),
    ("cg", even(2)),
    ("cgt", even(2)),
    ("cgtb", even(2)),
    ("cgtbi", even(2)),
    ("cgth", even(2)),
    ("cgthi", even(2)),
    ("cgti", even(2)),
    ("cgx", even(2)),
    ("chd", odd(4)),
    ("chx", odd(4)),
    ("clgt", even(2)),
    ("clgtb", even(2)),
    ("clgtbi", even(2)),
    ("clgth", even(2)),
    ("clgthi", even(2)),
    ("clgti", even(2)),
    ("clz", even(2)),
    ("cntb", even(4)),
    ("csflt", even(7)),
    ("cuflt", even(7)),
    ("cwd", odd(4)),
    ("cwx", odd(4)),
    ("dfa", DOUBLE),
    ("dfm", DOUBLE),
    ("dfma", DOUBLE),
    ("dfms", DOUBLE),
    ("dfnma", DOUBLE),
    ("dfnms", DOUBLE),
    ("dfs", DOUBLE),
    ("dsync", odd(0)),
    ("eqv", even(2)),
    ("fa", even(6)),
    ("fceq", even(2)),
    ("fcgt", even(2)),
    ("fcmeq", even(2)),
    ("fcmgt", even(2)),
    ("fesd", DOUBLE),
    ("fi", even(7)),
    ("fm", even(6)),
    ("fma", even(6)),
    ("fms", even(6)),
    ("fnms", even(6)),
    ("frds", DOUBLE),
    ("frest", odd(4)),
    ("frsqest", odd(4)),
    ("fs", even(6)),
    ("fscrrd", DOUBLE),
    ("fscrwr", even(7)),
    ("fsm", odd(4)),
    ("fsmb", odd(4)),
    ("fsmbi", odd(4)),
    ("fsmh", odd(4)),
    ("gb", odd(4)),
    ("gbb", odd(4)),
    ("gbh", odd(4)),
    ("hbr", odd(0)),
    ("hbra", odd(0)),
    ("hbrr", odd(0)),
    ("heq", even(0)),
    ("heqi", even(0)),
    ("hgt", even(0)),
    ("hgti", even(0)),
    ("hlgt", even(0)),
    ("hlgti", even(0)),
    ("il", even(2)),
    ("ila", even(2)),
    ("ilh", even(2)),
    ("ilhu", even(2)),
    ("iohl", even(2)),
    ("iret", odd(0)),
    ("lnop", odd(0)),
    ("lqa", odd(6)),
    ("lqd", odd(6)),
    ("lqr", odd(6)),
    ("lqx", odd(6)),
    ("mfspr", odd(6)),
    ("mpy", even(7)),
    ("mpya", even(7)),
    ("mpyh", even(7)),
    ("mpyhh", even(7)),
    ("mpyhha", even(7)),
    ("mpyhhau", even(7)),
    ("mpyhhu", even(7)),
    ("mpyi", even(7)),
    ("mpys", even(7)),
    ("mpyu", even(7)),
    ("mpyui", even(7)),
    ("mtspr", odd(0)),
    ("nand", even(2)),
    ("nop", even(0)),
    ("nor", even(2)),
    ("or", even(2)),
    ("orbi", even(2)),
    ("orc", even(2)),
    ("orhi", even(2)),
    ("ori", even(2)),
    ("orx", odd(4)),
    ("rchcnt", odd(6)),
    ("rdch", odd(6)),
    ("rot", even(4)),
    ("roth", even(4)),
    ("rothi", even(4)),
    ("rothm", even(4)),
    ("rothmi", even(4)),
    ("roti", even(4)),
    ("rotm", even(4)),
    ("rotma", even(4)),
    ("rotmah", even(4)),
    ("rotmahi", even(4)),
    ("rotmai", even(4)),
    ("rotmi", even(4)),
    ("rotqbi", odd(4)),
    ("rotqbii", odd(4)),
    ("rotqby", odd(4)),
    ("rotqbybi", odd(4)),
    ("rotqbyi", odd(4)),
    ("rotqmbi", odd(4)),
    ("rotqmbii", odd(4)),
    ("rotqmby", odd(4)),
    ("rotqmbybi", odd(4)),
    ("rotqmbyi", odd(4)),
    ("selb", even(2)),
    ("sf", even(2)),
    ("sfh", even(2)),
    ("sfhi", even(2)),
    ("sfi", even(2)),
    ("sfx", even(2)),
    ("shl", even(4)),
    ("shlh", even(4)),
    ("shlhi", even(4)),
    ("shli", even(4)),
    ("shlqbi", odd(4)),
    ("shlqbii", odd(4)),
    ("shlqby", odd(4)),
    ("shlqbybi", odd(4)),
    ("shlqbyi", odd(4)),
    ("shufb", odd(4)),
    ("stop", odd(0)),
    ("stopd", odd(0)),
    ("stqa", odd(0)),
    ("stqd", odd(0)),
    ("stqr", odd(0)),
    ("stqx", odd(0)),
    ("sumb", even(4)),
    ("sync", odd(0)),
    ("wrch", odd(0)),
    ("xor", even(2)),
    ("xorbi", even(2)),
    ("xorhi", even(2)),
    ("xori", even(2)),
    ("xsbh", even(2)),
    ("xshw", even(2)),
    ("xswd", even(2)),
];

const fn build_issue_table() -> [Option<SpuIssue>; SPU_OPCODE_MAP.len()] {
    let mut table = [None; SPU_OPCODE_MAP.len()];
    let mut i = 0;
    while i < TABLE_B1.len() {
        match row_named(TABLE_B1[i].0) {
            Some(row) => table[row] = Some(TABLE_B1[i].1),
            None => panic!("a Table B-1 mnemonic names no opcode-map row"),
        }
        i += 1;
    }
    table
}

/// Table B-1 indexed by [`SPU_OPCODE_MAP`] row: `Some` for every row the
/// CBE provides, `None` for the optional rows it does not.
pub const SPU_ISSUE_TABLE: [Option<SpuIssue>; SPU_OPCODE_MAP.len()] = build_issue_table();

/// The fetch line: the 32 instructions one local-store request loads.
///
/// [CBE-Handbook p:771 s:B.1.3.1] instruction fetches load 32 instructions per LS request.
pub const SPU_FETCH_LINE_BYTES: u32 = 128;

/// The cycles from a mispredicted branch to the issue of the right
/// instruction, when that instruction lies in the first half of its
/// fetch line.
///
/// [CBE-Handbook p:691 s:24.3] a mispredicted branch costs 18 to 19 cycles, depending on the address of the branch target.
///
/// The `spu_issue_timing` capture places a loop of nop, ai and an
/// unhinted brnz at fourteen target addresses. A loop in the first 64
/// bytes of a 128-byte line runs 21 cycles an iteration. A loop in the
/// last 64 bytes runs 22.
pub const SPU_BRANCH_MISS_CYCLES: u64 = 18;

/// [`SPU_BRANCH_MISS_CYCLES`] for an instruction in the second half of
/// its fetch line.
pub const SPU_BRANCH_MISS_LATE_CYCLES: u64 = 19;

/// The cycles from a hinted branch that is not taken to the issue of
/// the instruction after it.
///
/// [CBE-Handbook p:694 s:24.3.3] when a hinted branch is not taken, the speculated path is flushed and refetched.
///
/// The `spu_issue_timing` capture measures 16, for a branch that the
/// hint stall holds.
pub const SPU_HINTED_NOT_TAKEN_CYCLES: u64 = 16;

/// The cycles from a hint's issue until a branch can use it.
///
/// [CBE-Handbook p:695 s:24.3.3.4] a hint should precede its branch by eleven cycles plus four instruction pairs; a branch the hint stall holds waits until the hint is loaded.
///
/// The `spu_issue_timing` capture times a loop that issues its own hint
/// twelve instructions before its branch. The loop runs 19 cycles an
/// iteration, and the hint stall holds the branch until 18 cycles after
/// the hint.
pub const SPU_HINT_READY_CYCLES: u64 = 18;

/// The nearest branch, in instructions after its hint, that the hint
/// stall holds.
///
/// [CBE-Handbook p:695 s:24.3.3.4] hint stall is set up by hints whose branches are at least eight, but less than 32, instructions ahead.
pub const SPU_HINT_STALL_MIN_INSNS: u32 = 8;

/// The first branch distance, in instructions after its hint, that the
/// hint stall no longer holds.
pub const SPU_HINT_STALL_END_INSNS: u32 = 32;

/// The cycle window in which the SPU retries a second channel
/// instruction to the same channel.
///
/// [CBE-Handbook p:764 s:B.1.2 Table B-1] note 4: a rdch, wrch or rchcnt issued within five SPU cycles of one to the same channel, or to SPU_RdEventStat, SPU_WrEventMask or SPU_WrEventAck, is retried, which causes a flush.
///
/// The `spu_issue_timing` capture times two rchcnt in a row at 41 cycles
/// an iteration, and two rchcnt six cycles apart at 16. Each retried
/// instruction issues again one branch-miss refetch later.
pub const SPU_CHANNEL_RETRY_WINDOW_CYCLES: u64 = 5;

/// The fetch lines the SPU holds past the one it issues from, after a
/// branch or a flush.
///
/// [CBE-Handbook p:66 s:3.1.1.3 Table 3-1] inline fetch has the lowest local-store priority, below loads, stores and hint fetches.
///
/// The `spu_issue_fetch` capture times loops of 64 to 224 lqa from a
/// 128-byte-aligned target. A loop of 88 lqa with its ai and brnz, three
/// lines, runs without a runout; 96 lqa run out once, in the fourth line.
pub const SPU_FETCH_LINES_HELD: u32 = 2;

/// The cycles after the issue of the first instruction of a line in which
/// inline prefetch can still find a cycle no load, store or hint takes.
/// The issue cycle itself also counts.
///
/// [CBE-Handbook p:774 s:B.1.4] the prefetcher has nine cycles in which it must succeed in securing a slot; if the LS is busy for that interval, instruction runout can occur.
///
/// The `spu_issue_fetch` capture bounds the window between 1 and 31
/// cycles:
///
/// - The loop of 64 lqa, lnop and 64 lqa runs out once, though the lnop
///   frees a slot 32 cycles into the window.
/// - The loop of four times 32 lqa and lnop does not run out, and one of
///   its lnop frees a slot 1 cycle into its window.
pub const SPU_PREFETCH_WINDOW_CYCLES: u64 = 9;

/// The cycles an instruction runout costs: the issue delay of the first
/// instruction of a line whose prefetch found no free cycle.
///
/// [CBE-Handbook p:774 s:B.1.4] a prefetch late enough to run out delays the next issue by one cycle for every cycle the prefetch is delayed.
///
/// The `spu_issue_fetch` capture times every loop of 96 to 128 lqa at
/// 17 cycles an iteration past its instruction count, against 3 for the
/// loops of 64 to 88; 160 and 192 lqa at 31, and 224 at 45.
pub const SPU_RUNOUT_CYCLES: u64 = 14;

/// The cycles a getllar takes with the read of its atomic status: the
/// `spu_dma_timing` capture's mean, in SPU cycles.
pub const SPU_GETLLAR_CYCLES: u64 = super::mfc_timing::MFC_GETLLAR_TB_SUM as u64
    * SPU_CYCLES_PER_TB_TICK
    / super::mfc_timing::MFC_TIMING_RUNS as u64;

/// The cycles a putllc that keeps its reservation takes with the read of
/// its atomic status: the `spu_dma_timing` capture's mean, in SPU cycles.
pub const SPU_PUTLLC_CYCLES: u64 = super::mfc_timing::MFC_PUTLLC_TB_SUM as u64
    * SPU_CYCLES_PER_TB_TICK
    / super::mfc_timing::MFC_TIMING_RUNS as u64;

/// The cycles a putlluc takes with the read of its atomic status: the
/// `spu_dma_timing` capture's mean, in SPU cycles.
pub const SPU_PUTLLUC_CYCLES: u64 = super::mfc_timing::MFC_PUTLLUC_TB_SUM as u64
    * SPU_CYCLES_PER_TB_TICK
    / super::mfc_timing::MFC_TIMING_RUNS as u64;

/// The branch-miss cycles to an instruction at local-store address
/// `target`: the refetch every flush pays.
///
/// [CBE-Handbook p:770 s:B.1.2 Table B-1] note 5: sync and syncc cause a pipeline flush.
///
/// The `spu_issue_timing` capture times a loop of sync, ai and a hinted
/// brnz at 38 cycles an iteration, and the same loop with syncc at 38.
/// Each iteration pays two refetches:
///
/// - one after the sync
/// - one after the loop branch, which mispredicts because the sync drops
///   its hint
///
/// dsync takes the 4 cycles of a nop.
pub const fn branch_miss_cycles(target: u32) -> u64 {
    if target % SPU_FETCH_LINE_BYTES < SPU_FETCH_LINE_BYTES / 2 {
        SPU_BRANCH_MISS_CYCLES
    } else {
        SPU_BRANCH_MISS_LATE_CYCLES
    }
}

#[cfg(test)]
#[path = "tests/spu_issue_tests.rs"]
mod tests;
