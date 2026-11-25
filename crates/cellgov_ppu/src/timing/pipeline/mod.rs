//! The issue model: the core cycle each retired instruction issues at.
//!
//! - **Fetch.** A thread fetches every [`PPU_FETCH_INTERVAL_CYCLES`]
//!   cycles, up to the end of the aligned [`PPU_FETCH_BYTES`] block. A
//!   fetch ends at its first branch. The next fetch follows a
//!   predicted-taken branch by [`PPU_TAKEN_BRANCH_FETCH_CYCLES`] and a
//!   predicted-not-taken one by [`PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES`]. A
//!   mispredicted branch restarts fetch [`PPU_BRANCH_MISS_CYCLES`] after
//!   it issues. Fetch runs at most [`PPU_FETCH_AHEAD_INSTRUCTIONS`] ahead
//!   of issue.
//! - **Dispatch.** Two instructions at an 8-byte-aligned pair form a
//!   group.
//! - **Issue.** Instructions issue in order. The younger of a group
//!   issues with the older when all of these hold:
//!   - Figure A-1 allows the pair of units.
//!   - The younger reads nothing the older writes.
//!   - Neither one is unpipelined, microcoded, a drain, a barrier or an
//!     `sc`.
//!
//!   An instruction issues once all of these hold:
//!   - Its sources are ready.
//!   - Each register it writes has no older write still pending.
//!   - Its units are free.
//!
//! [CBE-Handbook p:755 s:A.6.1] instructions grouped together must be
//! aligned on 8-byte boundaries.
//!
//! [CBE-Handbook p:753 s:A.5] the dual-issue combinations, and the
//! cases that prevent dual issue.
//!
//! [`PPU_FETCH_INTERVAL_CYCLES`]: k::PPU_FETCH_INTERVAL_CYCLES
//! [`PPU_FETCH_BYTES`]: k::PPU_FETCH_BYTES
//! [`PPU_TAKEN_BRANCH_FETCH_CYCLES`]: k::PPU_TAKEN_BRANCH_FETCH_CYCLES
//! [`PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES`]: k::PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES
//! [`PPU_BRANCH_MISS_CYCLES`]: k::PPU_BRANCH_MISS_CYCLES
//! [`PPU_FETCH_AHEAD_INSTRUCTIONS`]: k::PPU_FETCH_AHEAD_INSTRUCTIONS

use super::cache::{Caches, Level, Operand};
use super::op::{is_fpr, is_vr, unit, Barrier, Kind, LoadClass, Op, Target, NO_REG, REGS};
use super::predictor::Predictor;
use cellgov_exec::PpeIssuePressure;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
use cellgov_ps3_abi::hw::ppu_issue as k;
use pressure::{divides, note_divide, note_pressure, sibling_stall, stretch_pending, weigh_over};
use store_queue::{barrier_start, bytes_in, queue_room, queue_store};

mod pressure;
mod store_queue;

#[cfg(test)]
#[path = "tests/mffs_record_wait_tests.rs"]
mod mffs_record_wait_tests;

#[cfg(test)]
#[path = "tests/kernel_share_tests.rs"]
mod kernel_share_tests;

#[cfg(test)]
#[path = "tests/divide_window_tests.rs"]
mod divide_window_tests;

#[cfg(test)]
#[path = "tests/preempted_call_tests.rs"]
mod preempted_call_tests;
#[cfg(test)]
#[path = "tests/resumed_call_tests.rs"]
mod resumed_call_tests;
#[cfg(test)]
#[path = "tests/sized_call_tests.rs"]
mod sized_call_tests;
#[cfg(test)]
#[path = "tests/timed_out_call_tests.rs"]
mod timed_out_call_tests;

/// The length of the ring of latest issue cycles that the fetch-ahead
/// bound reads.
const AHEAD_RING: usize = 32;

/// When an instruction issues, and how much of its wait a sibling added.
struct Scheduled {
    issue: u64,
    /// The cycle the op leaves the VSU issue queue and runs. It is the
    /// issue cycle unless the queue holds the op past IS2.
    exec: u64,
    /// The cycles a drain waits while the VSU issue queue still holds ops
    /// of this thread.
    queued: u64,
    /// The cycles of the wait the sibling's hold and stall added.
    sibling: u64,
    /// The sibling's stall alone.
    stalled: u64,
    /// The sibling's hold alone: the wait for its dispatch group.
    held: u64,
    /// The cycle the sibling's stall began at.
    began: u64,
    /// The VSU issue queue holds the op past IS2 for a divide in progress.
    behind_divide: bool,
    /// [`Core::divide_carry`] after the op.
    divide_carry: u32,
    /// The op opens a dispatch group.
    opens_group: bool,
}

/// The older instruction of an open dispatch group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Older {
    pc: u64,
    issue: u64,
    op: Op,
}

/// The scalar part of the model, which a faulted batch rolls back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Core {
    /// The earliest cycle the next dispatch group can issue at.
    floor: u64,
    /// Instructions retired, which index [`Core::ahead`].
    retired: u64,
    ahead: [u64; AHEAD_RING],
    older: Option<Older>,
    /// Whether a fetch block is open.
    block_open: bool,
    /// The cycle the open block arrives at.
    block_cycle: u64,
    /// The end of the open block's aligned range.
    block_end: u64,
    /// The address the open block reads next.
    next_pc: u64,
    /// The cycle of the next fetch block.
    next_block: u64,
    /// The cycle each register's value is ready for a data use.
    ready: [u64; REGS],
    /// The cycle each register's value is ready for an address use.
    addr_ready: [u64; REGS],
    /// The cycle an outstanding load miss writes each register.
    ///
    /// [CBE-Handbook p:137 s:6.1.3.7] an instruction whose target is the
    /// target of an outstanding load miss waits for the load.
    miss: [u64; REGS],
    /// The cycle each unit is free again, indexed by unit bit.
    unit_free: [u64; 5],
    /// The earliest cycle a vector-scalar instruction other than an
    /// `mtvscr` can leave the VSU issue queue and run at, after the last
    /// `mtvscr`.
    vsu_fence: u64,
    /// The cycle after the last op that the VSU issue queue held runs, and
    /// so the earliest cycle the next held op runs. It stays zero until
    /// the queue holds an op. A drain issues no earlier than one cycle
    /// after it.
    vsu_queue: u64,
    /// The cycle the last divide or square root on the VSU completes.
    divide_done: u64,
    /// The cycle each of the last [`k::PPU_VIQ_HELD_OPS`] stages of the VSU
    /// issue queue behind a divide leaves it, a ring in issue order.
    viq: [u64; k::PPU_VIQ_HELD_OPS],
    /// The ring index of the oldest stage in [`Core::viq`], which the next
    /// stage takes.
    viq_next: usize,
    /// The cycle the youngest stage in [`Core::viq`] leaves.
    viq_tail: u64,
    /// The cycle the last floating-point store's data reaches the store
    /// queue. A drain that uses the VSU, other than an `mffs`, issues no
    /// earlier.
    fp_store_data: u64,
    /// The cycle the last floating-point store completes. A drain that
    /// does not use the VSU issues no earlier.
    fp_store_done: u64,
    /// The cycle the last floating-point store's data is ready for a
    /// younger `mffs`.
    fp_store_read: u64,
    /// The cycle the last vector-scalar instruction other than a load or
    /// store completes, its CR field included, for a younger `mffs`.
    vsu_done: u64,
    /// The cycle the last store completes, for a younger `mfvscr`. Stores
    /// complete in order.
    store_done: u64,
    /// The first and the last doubleword the last store wrote, as their
    /// addresses divided by 8.
    store_dwords: (u64, u64),
    /// The cycle the last `mffs` result is ready, or the last `mfvscr`
    /// completes. A younger floating-point store sends its data no earlier.
    status_read: u64,
    /// The cycle every older vector-scalar instruction other than a load
    /// or store completes, for a younger `mfvscr`. It includes the last
    /// `mfvscr`, which completes `PPU_MFVSCR_LATENCY_CYCLES` after it runs.
    vscr_done: u64,
    /// The address of the instruction right after the last op, when that
    /// op is an `mffs` at an even word, so that the two share a pair. A VSU
    /// op at that address runs no earlier than the `mffs` result
    /// ([`fpscr_floor`]).
    fpscr_partner: Option<u64>,
    /// The earliest cycle a VSU op runs at, when a taken branch lands on it
    /// at an odd word ([`fpscr_floor`]). The last `mffs` sets it.
    odd_target_floor: u64,
    /// The earliest cycle a VSU op runs at, set by the last `mffs`
    /// ([`fpscr_floor`]).
    fpscr_queue_floor: u64,
    /// The earliest cycle an instruction younger than the last `mfvscr`
    /// issues at.
    vscr_hold: u64,
    /// The registers, one bit each, whose last write came from an
    /// instruction whose writes land in issue order
    /// ([`Op::writes_in_order`]).
    in_order_writes: u128,
    /// The share of this thread's recent cycles, out of [`SHARE_ONE`],
    /// spent on divides and square roots. Each instruction weighs by its
    /// cycles, over a window of [`PRESSURE_WINDOW_CYCLES`].
    divide_share: u32,
    /// The share of this thread's recent dispatch cycles, out of
    /// [`SHARE_ONE`], in which its divides stall the shared issue point.
    ///
    /// It averages over a window of [`STALL_WINDOW_CYCLES`]. Dispatch
    /// cycles are the thread's own cycles and its holds for the sibling's
    /// dispatch group.
    stall_share: u32,
    /// The fraction of a cycle, out of 256, of the sibling's divide stalls
    /// this thread has yet to wait ([`PpeIssuePressure::divide_fraction`]).
    divide_carry: u32,
    /// The share of this thread's recent cycles spent on the other
    /// instructions that issue alone for long (`issues_alone_for_long`),
    /// weighed as [`Core::divide_share`] is.
    alone_share: u32,
    /// The share of this thread's recent cycles spent on `mtvscr`
    /// instructions, past their wait for the ops the VSU issue queue held,
    /// weighed as [`Core::divide_share`] is.
    vscr_share: u32,
    /// The share of this thread's recent cycles in which its `mtvscr`
    /// instructions wait for the ops the VSU issue queue holds, weighed as
    /// [`Core::divide_share`] is.
    drain_share: u32,
    /// This thread's recent mean wait past its in-order floor, in 1/256
    /// cycle, over its other instructions: a wait for operands or for a
    /// busy unit. Each instruction weighs one in
    /// [`PRESSURE_WINDOW_INSTRUCTIONS`].
    operand_wait: u32,
    /// This thread's recent mean cycles, in 1/256 cycle, that its VSU
    /// work holds the head of the VSU issue queue. An op holds the head
    /// while it waits for its operands, its unit or the fence, past the op
    /// before it in the queue. Each instruction weighs one in
    /// [`PRESSURE_WINDOW_INSTRUCTIONS`].
    queue_wait: u32,
    /// The part of [`Core::queue_wait`] that its ops held at the fence take.
    fence_wait: u32,
    /// This thread's recent mean cycles between its own issues, in 1/256
    /// cycle, past what a sibling added. Each instruction weighs one in
    /// [`PRESSURE_WINDOW_INSTRUCTIONS`].
    interval: u32,
    /// The earliest cycle, in 1/256 cycle, an outranking sibling leaves
    /// this thread's next instruction a dispatch slot at.
    gate: u64,
    /// The LV2 call whose round trip the thread has yet to charge.
    syscall: Option<PendingSyscall>,
    /// The thread left the PPE between calls for a higher-priority thread,
    /// and has yet to charge [`k::PPU_PREEMPTED_RESUME_CYCLES`].
    preempted: bool,
    /// The share of this thread's recent cycles, out of [`SHARE_ONE`],
    /// spent in LV2 calls, over a window of [`KERNEL_WINDOW_CYCLES`].
    kernel_share: u32,
    /// This thread's recent mean cycles between two of its divides that
    /// space a sibling (`Op::spaces_sibling`), in 1/256 cycle. Each sample
    /// weighs one in [`PRESSURE_WINDOW_INSTRUCTIONS`].
    divide_interval: u32,
    /// The cycle this thread's last divide that spaces a sibling issued
    /// at: `None` before the first, and after an LV2 call or a resume.
    last_divide: Option<u64>,
    /// The cycle this thread's last dispatch group opened at.
    group_open: u64,
}

// `Core::in_order_writes` holds one bit for each tracked register.
const _: () = assert!(REGS <= u128::BITS as usize);

/// The LV2 call of an `sc` the thread retired, charged when the thread
/// resumes after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingSyscall {
    /// The cycle the `sc` issued at.
    issue: u64,
    /// The syscall number.
    number: u64,
    /// The low words of r3 and r4 at the `sc`.
    args: [u32; 2],
    /// The code the call returned, once the runtime commits it.
    code: Option<u64>,
    /// The thread left the PPE before the call returned: it blocked, or
    /// it waited for a hardware thread.
    resumed: bool,
    /// The thread left the PPE only for a higher-priority thread.
    preempted: bool,
}

/// [`Core::divide_share`] for a thread that spends all its cycles on
/// divides and square roots.
const SHARE_ONE: u32 = 1 << 16;

/// The cycles [`Core::divide_share`] and [`Core::alone_share`]
/// average over.
const PRESSURE_WINDOW_CYCLES: u64 = 64;

/// The cycles [`Core::stall_share`] averages over: about a sibling's round
/// of divides and the work between them, as in the `ppu_issue_viq_depth`
/// capture's rounds of 300 to 1250 cycles.
const STALL_WINDOW_CYCLES: u64 = 1024;

/// The [`Core::stall_share`] from which a thread's divides stall the
/// shared issue point back to back, and a sibling's dispatch groups wait
/// [`PpeIssuePressure::window_cycles`] apart.
const WINDOW_STALL_SHARE: u32 = SHARE_ONE * 15 / 16;

/// The cycles [`Core::kernel_share`] averages over: long enough to span an
/// LV2 call and the instructions between two calls.
const KERNEL_WINDOW_CYCLES: u64 = 4096;

/// The instructions [`Core::operand_wait`] and [`Core::interval`] average
/// over.
const PRESSURE_WINDOW_INSTRUCTIONS: u32 = 8;

/// The pending store entries: those of the store queue and those past it.
///
/// An entry pends from its store's issue until it writes its bytes into
/// the cache.
const PENDING_ENTRIES: usize = k::PPU_STORE_QUEUE_ENTRIES + k::PPU_STORE_WRITE_BACK_ENTRIES;

/// One pending store entry: the bytes stores wrote to one 16-byte block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Queued {
    /// The 16-byte block, as its address over 16.
    block: u64,
    /// The bytes of the block the stores wrote, bit `i` for byte `i`. An
    /// entry with no bit set holds no store.
    mask: u16,
    /// The cycle this entry writes its bytes into the cache.
    done: u64,
}

/// The tables a faulted batch keeps: the caches, the predictor and the
/// store queue. They advance on instructions the batch retires again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Tables {
    caches: Caches,
    predictor: Predictor,
    /// The pending store entries, a ring in the order the stores queued
    /// them.
    stores: [Queued; PENDING_ENTRIES],
    /// The ring index of the oldest entry, which the next new entry takes.
    next_store: usize,
    /// The cycle each slot of the store queue is free again, a ring in
    /// the order the entries took the slots.
    slots: [u64; k::PPU_STORE_QUEUE_ENTRIES],
    /// The ring index of the oldest slot, which the next new entry takes.
    next_slot: usize,
    /// The latest cycle the L2 has from memory a line that a store found
    /// missing, or 0.
    line_fetched: u64,
}

/// The issue model of one PPU thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PpeTiming {
    core: Core,
    tables: Box<Tables>,
    /// The cycles a runnable sibling thread holds issue after each of
    /// this thread's dispatch groups, of which its stall covers as many
    /// cycles as it lasts: zero with no sibling runnable.
    hold: u64,
    /// What a runnable sibling's stalls at the shared issue point add to
    /// each of this thread's instructions.
    shared_stall: PpeIssuePressure,
}

impl Default for PpeTiming {
    fn default() -> Self {
        Self::new()
    }
}

fn unit_slots(units: u8) -> impl Iterator<Item = usize> {
    (0..5).filter(move |b| units & (1 << b) != 0)
}

/// Whether `younger` may issue beside `older`, by the unit pairs of
/// Figure A-1.
///
/// [CBE-Handbook p:753 s:A.5] a VXU permute in slot 0 can dual issue
/// with an LSU instruction in slot 1.
fn pairs(older: &Op, younger: &Op) -> bool {
    const ALLOWED: [u8; 5] = [
        unit::LSU | unit::BRU | unit::VSU1 | unit::VSU2,
        unit::FXU | unit::BRU,
        unit::FXU | unit::LSU | unit::VSU1 | unit::VSU2,
        unit::FXU | unit::LSU | unit::BRU | unit::VSU2,
        unit::FXU | unit::BRU | unit::VSU1,
    ];
    if older.units & younger.units != 0 {
        return false;
    }
    unit_slots(older.units).all(|a| {
        let allowed = if a == 4 && older.permute {
            ALLOWED[a] | unit::LSU
        } else {
            ALLOWED[a]
        };
        younger.units & !allowed == 0
    })
}

/// Whether an older instruction keeps the next one from issuing beside
/// it.
///
/// [CBE-Handbook p:753 s:A.5] a nonpipelined, context-synchronizing or
/// microcoded older instruction issues alone.
fn issues_alone(op: &Op) -> bool {
    op.words != 0
        || matches!(
            op.kind,
            Kind::Unpipelined(_)
                | Kind::Syscall { .. }
                | Kind::Barrier(_)
                | Kind::Drain
                | Kind::VscrWrite
        )
}

/// The direction a `BO` hint predicts, or `None` when `bo` carries no
/// hint.
///
/// [PPC-Book1 p:20 s:2.4.1] `BO` encodings `001at` and `011at` carry the
/// hint in bits 3 and 4; `1a00t` and `1a01t` in bits 1 and 4. `a = 1`
/// predicts the direction `t`.
fn static_prediction(bo: u8) -> Option<bool> {
    let hint = if bo & 0b10100 == 0b00100 {
        bo & 0b00010 != 0
    } else if bo & 0b10100 == 0b10000 {
        bo & 0b01000 != 0
    } else {
        false
    };
    hint.then_some(bo & 1 != 0)
}

/// Whether a taken branch at `pc` times as one that falls through.
///
/// It does when the branch:
/// - is direct, to `pc + 4`, and does not link;
/// - tests a CR bit and leaves CTR alone;
/// - has no hint.
///
/// In the `ppu_issue_quickened_forms` capture, eight `cmpwi; beq` or
/// `cmpw; beq` taken to the next instruction, and the loop's `bdnz`,
/// take 38 to 52 cycles an iteration. Eight `cmpwi; bne` that fall
/// through take 40 to 48. A model that times each `beq` as a
/// predicted-taken fetch gives 72.
fn lands_on_fall_through(pc: u64, bo: u8, link: bool, target: Target, next_pc: u64) -> bool {
    matches!(target, Target::Direct)
        && !link
        && next_pc == pc.wrapping_add(4)
        && bo & 0b10100 == 0b00100
        && static_prediction(bo).is_none()
}

impl PpeTiming {
    /// A thread with no issued instruction and empty caches.
    pub(crate) fn new() -> Self {
        Self {
            core: Core {
                floor: 0,
                retired: 0,
                ahead: [0; AHEAD_RING],
                older: None,
                block_open: false,
                block_cycle: 0,
                block_end: 0,
                next_pc: 0,
                next_block: 0,
                ready: [0; REGS],
                addr_ready: [0; REGS],
                miss: [0; REGS],
                unit_free: [0; 5],
                vsu_fence: 0,
                vsu_queue: 0,
                divide_done: 0,
                viq: [0; k::PPU_VIQ_HELD_OPS],
                viq_next: 0,
                viq_tail: 0,
                fp_store_data: 0,
                fp_store_done: 0,
                fp_store_read: 0,
                vsu_done: 0,
                store_done: 0,
                store_dwords: (0, 0),
                status_read: 0,
                vscr_done: 0,
                fpscr_partner: None,
                odd_target_floor: 0,
                fpscr_queue_floor: 0,
                vscr_hold: 0,
                in_order_writes: 0,
                divide_share: 0,
                stall_share: 0,
                divide_carry: 0,
                alone_share: 0,
                vscr_share: 0,
                drain_share: 0,
                operand_wait: 0,
                queue_wait: 0,
                fence_wait: 0,
                interval: 0,
                gate: 0,
                syscall: None,
                preempted: false,
                kernel_share: 0,
                divide_interval: 0,
                last_divide: None,
                group_open: 0,
            },
            tables: Box::new(Tables {
                caches: Caches::new(),
                predictor: Predictor::new(),
                stores: [Queued::default(); PENDING_ENTRIES],
                next_store: 0,
                slots: [0; k::PPU_STORE_QUEUE_ENTRIES],
                next_slot: 0,
                line_fetched: 0,
            }),
            hold: 0,
            shared_stall: PpeIssuePressure::NONE,
        }
    }

    /// Set the number of other PPE threads runnable, and the largest
    /// [`PpeTiming::pressure`] among them as [`PpeIssuePressure::max`]
    /// gives it, for the instructions retired from here on.
    pub(crate) fn set_siblings(&mut self, siblings: u32, pressure: PpeIssuePressure) {
        if siblings == 0 {
            self.hold = 0;
            self.shared_stall = PpeIssuePressure::NONE;
            // [CBE-Handbook p:301 s:10.2.3.1] a stall at IS2 stalls both
            // threads. With no sibling, no divide stall reaches this thread,
            // so the thread drops the fraction of a cycle it carries.
            self.core.divide_carry = 0;
        } else {
            self.hold = k::PPU_SMT_SIBLING_HOLD_CYCLES;
            self.shared_stall = pressure;
        }
        // A gate set by a sibling that no longer outranks this thread holds
        // nothing once an outranking one returns.
        if self.shared_stall.issue_interval == 0 {
            let c = &mut self.core;
            c.gate = c.gate.min(c.floor.saturating_mul(256));
        }
    }

    /// What this thread's recent stalls at the shared issue point add to
    /// each instruction of a sibling thread, and its mean issue interval.
    ///
    /// [CBE-Handbook p:301 s:10.2.3.1] a stall at IS2 stalls both threads.
    pub(crate) fn pressure(&self) -> PpeIssuePressure {
        let c = &self.core;
        let share =
            |cycles: u64, share: u32| cycles * 256 * u64::from(share) / u64::from(SHARE_ONE);
        let wait = |(num, den): (u64, u64)| u64::from(c.operand_wait) * num / den;
        let whole = |sum: u64, cap: u64| ((sum + 128) >> 8).min(cap) as u32;
        let cap = k::PPU_SMT_SHARED_STALL_CYCLES;
        // This thread stalls the shared issue point for a share f of its
        // cycles. That stretches the own cycles of each sibling instruction
        // (`PPU_SMT_OWN_ISSUE_CYCLES`) by f / (1 - f).
        let rest = u64::from(SHARE_ONE.saturating_sub(c.stall_share)).max(1);
        let divide =
            (k::PPU_SMT_OWN_ISSUE_CYCLES * 256 * u64::from(c.stall_share) / rest).min(cap * 256);
        // The sibling carries the part past a whole cycle
        // (`divide_fraction`).
        let fraction = divide % 256;
        let divide = divide - fraction;
        let long = divide + share(k::PPU_SMT_MICROCODE_STALL_CYCLES, c.alone_share);
        let all = (long + wait(k::PPU_SMT_DEPENDENT_STALL_SHARE)).min(cap * 256);
        let (num, den) = k::PPU_SMT_VSU_STALL_SHARE;
        PpeIssuePressure {
            cycles: whole(all, cap),
            vsu_cycles: whole(divide + (all - divide) * num / den, cap),
            // A multiply beside microcode waits about the divide cap: in
            // the `ppu_issue_shared` capture each `mulli` beside `lha`
            // waits 14.8 to 16.2 cycles, more than any other chain there.
            multiply_cycles: whole(
                divide
                    + share(cap, c.alone_share)
                    + wait(k::PPU_SMT_MULTIPLY_DEPENDENT_STALL_SHARE),
                cap,
            ),
            divide_cycles: whole(divide, cap),
            divide_fraction: fraction as u32,
            multiply_hold_share: (c.divide_share * 256 + SHARE_ONE / 2) / SHARE_ONE,
            vscr_cycles: whole(
                share(k::PPU_SMT_MTVSCR_STALL_CYCLES, c.vscr_share),
                k::PPU_SMT_MTVSCR_STALL_CYCLES,
            ),
            drain_cycles: {
                let (num, den) = k::PPU_SMT_MTVSCR_DRAIN_STALL_SHARE;
                whole(
                    share(k::PPU_SMT_MTVSCR_STALL_CYCLES, c.drain_share) * num / den,
                    k::PPU_SMT_MTVSCR_STALL_CYCLES,
                )
            },
            queue_cycles: whole(u64::from(c.queue_wait), cap),
            fence_cycles: whole(u64::from(c.fence_wait), cap),
            kernel_share: (c.kernel_share * 256 + SHARE_ONE / 2) / SHARE_ONE,
            issue_interval: c.interval,
            // The window opens only when a divide that spaces a sibling
            // issued within the last `STALL_WINDOW_CYCLES`, so other
            // divides keep no stale interval open.
            window_cycles: if c.stall_share >= WINDOW_STALL_SHARE
                && c.last_divide
                    .is_some_and(|last| c.floor.saturating_sub(last) <= STALL_WINDOW_CYCLES)
            {
                let (num, den) = k::PPU_SMT_DIVIDE_WINDOW_SHARE;
                ((u64::from(c.divide_interval) * num / den + 128) >> 8) as u32
            } else {
                0
            },
        }
    }

    /// The thread's cycle: the earliest the next instruction can issue.
    pub(crate) fn now(&self) -> u64 {
        self.core.floor
    }

    /// The tag-only caches.
    #[cfg(test)]
    pub(crate) fn caches(&self) -> &super::cache::Caches {
        &self.tables.caches
    }

    /// The scalar state, for a batch to restore after a fault.
    pub(crate) fn core(&self) -> Core {
        self.core
    }

    /// Restore the scalar state a batch took at its entry.
    pub(crate) fn restore(&mut self, core: Core) {
        self.core = core;
    }

    /// The cycle the fetch block holding `pc` arrives at, and whether
    /// `pc` opens a new block.
    fn fetch(&self, pc: u64) -> (u64, bool) {
        let c = &self.core;
        if c.block_open && pc == c.next_pc {
            return (c.block_cycle, false);
        }
        let mut cycle = c.next_block;
        if let Some(n) = c.retired.checked_sub(k::PPU_FETCH_AHEAD_INSTRUCTIONS) {
            cycle = cycle.max(c.ahead[(n % AHEAD_RING as u64) as usize]);
        }
        (cycle, true)
    }

    /// The cycle `op` at `pc` issues at, from the state the instructions
    /// before it left. For a microcoded instruction this is the cycle
    /// its first microword issues at.
    pub(crate) fn issue_cycle(&self, pc: u64, op: &Op) -> u64 {
        self.schedule(pc, op).issue
    }

    /// [`PpeTiming::issue_cycle`], and the cycles of that wait a sibling
    /// thread added.
    fn schedule(&self, pc: u64, op: &Op) -> Scheduled {
        let c = &self.core;
        let (fetched, opens) = self.fetch(pc);
        let (floor, opens_group) = match c.older {
            Some(older) if !opens && pc == older.pc.wrapping_add(4) => {
                let alone = issues_alone(&older.op)
                    || issues_alone(op)
                    || !pairs(&older.op, op)
                    || op.reads_any_of(&older.op);
                (older.issue + u64::from(alone), false)
            }
            _ => (c.floor, true),
        };
        // An instruction younger than an `mfvscr` issues no earlier than
        // `PPU_MFVSCR_YOUNGER_HOLD_CYCLES` after it.
        let mut t = floor.max(fetched).max(c.vscr_hold);
        let mut queued = 0;
        let mut operands = 0;
        for (r, after) in op.reads() {
            operands = operands.max(c.ready[r as usize].saturating_sub(after));
        }
        for &r in &op.addr {
            if r != NO_REG {
                t = t.max(c.addr_ready[r as usize]);
            }
        }
        // [CBE-Handbook p:755 s:A.6.1] decode checks for write-after-write
        // hazards, and issue stalls at IS2 until they clear. A pipelined
        // fixed-point write lands in issue order, so it leaves no hazard.
        // In the `ppu_issue_mfvscr_younger` capture 32 `li` to one register
        // take 33 cycles an iteration, as 32 `li` to eight registers do.
        for r in op.writes() {
            operands = operands.max(c.miss[r as usize]);
            if c.in_order_writes & (1 << r) == 0 {
                operands = operands.max(c.ready[r as usize]);
            }
        }
        let vsu = unit::VSU1 | unit::VSU2;
        let vsu_busy = unit_slots(op.units & vsu)
            .map(|u| c.unit_free[u])
            .max()
            .unwrap_or(0);
        // [CBE-Handbook p:320 s:10.7.2.9] the FPU recycles a divide or
        // square root in its issue queue. [CBE-Handbook p:756 s:A.6.2] the
        // queue holds a VSU instruction at VQ8 until it can run, past IS2.
        // So a VSU instruction that waits for a divide in progress waits in
        // the queue, and the instructions after it continue to issue.
        let behind_divide = waits_in_queue(op) && {
            let wait = operands.max(vsu_busy);
            wait > t && wait <= c.divide_done
        };
        if !behind_divide {
            t = t.max(operands);
        }
        // [CBE-Handbook p:756 s:A.6.2] a VSU instruction leaves IS2 for the
        // VSU issue queue and waits at VQ8. So the fence holds the op past
        // IS2, and the younger instructions continue to issue. In the
        // `ppu_issue_vscr_queue` capture `mtvscr; vaddubm` and the same with
        // up to 32 `addi` after it each take 58 cycles an iteration. An op
        // the fence holds waits in the queue for its VSU unit too, and the
        // queue releases the ops it holds in order, one a cycle.
        let fenced = uses_vsu(op) && op.kind != Kind::VscrWrite;
        let queued_units = if behind_divide || (fenced && c.vsu_fence > floor) {
            op.units & vsu
        } else {
            0
        };
        for u in unit_slots(op.units & !queued_units) {
            t = t.max(c.unit_free[u]);
        }
        let mut fence = if fenced {
            c.vsu_fence.max(c.vsu_queue)
        } else {
            0
        };
        for u in unit_slots(queued_units) {
            fence = fence.max(c.unit_free[u]);
        }
        if behind_divide {
            fence = fence.max(operands);
        }
        // [CBE-Handbook p:320 s:10.7.2.9] the recycle's stall can reach
        // dispatch. [CBE-Handbook p:301 s:10.2.3.1] the queue's 2-stage
        // buffer takes up to four more instructions while VQ8 stalls. A VSU
        // instruction issues once the queue has room behind the divide
        // (`PPU_VIQ_HELD_OPS`).
        if uses_vsu(op) {
            t = t.max(c.viq[c.viq_next]);
        }
        if matches!(
            op.kind,
            Kind::Drain | Kind::Barrier(_) | Kind::VscrWrite | Kind::Syscall { .. }
        ) {
            // [CBE-Handbook p:722 s:A.1.2] an `mffs` stalls at VQ8 until
            // older vector-scalar instructions complete, so it does not wait
            // for older fixed-point results. In the
            // `ppu_issue_store_drain_distance` capture an `mffs` after 32
            // `li` takes 33.00 cycles an iteration, one cycle an instruction.
            let all = if op.is_fpscr_read() {
                vsu_ready(c)
            } else {
                c.ready.iter().chain(c.unit_free.iter()).copied().max()
            };
            // [CBE-Handbook p:742 s:A.3.2] a drain issues once every older
            // instruction completes. An op that the queue held completes a
            // cycle after it runs.
            // A queue that never held an op leaves `vsu_queue` at zero.
            let after_queue = if c.vsu_queue == 0 { 0 } else { c.vsu_queue + 1 };
            let fp_store = if op.is_fpscr_read() {
                c.fp_store_read
            } else if uses_vsu(op) {
                c.fp_store_data
            } else {
                c.fp_store_done
            };
            // [CBE-Handbook p:742 s:A.3.2] an `mfvscr` waits at issue for
            // older stores to complete as well. It also waits for older
            // vector-scalar instructions to complete, which is later than
            // their results (`vscr_done`).
            let store = if op.is_vscr_read() {
                c.store_done.max(c.vscr_done)
            } else {
                0
            };
            let drained = t
                .max(all.unwrap_or(0))
                .max(after_queue)
                .max(fp_store)
                .max(store);
            if c.vsu_queue > floor {
                queued = drained - floor;
            }
            t = drained;
        }
        if let Kind::Store { ra, len, .. } = op.kind {
            t = queue_room(&self.tables, t, ra, len);
        }
        let s = &self.shared_stall;
        let own_vscr = op.kind == Kind::VscrWrite || (fenced && c.vsu_fence > c.floor);
        let own_wait = t.saturating_sub(floor.max(fetched));
        // While the sibling's divides stall the shared issue point back to
        // back, this thread's dispatch groups wait the window apart. Its
        // instructions then wait no sibling stall, and the divide carry
        // holds.
        let window = u64::from(s.window_cycles);
        let stalled = if window == 0 {
            sibling_stall(op, s, own_vscr, own_wait)
        } else {
            0
        };
        // The fractions of a cycle the sibling's divide stalls leave carry
        // from one instruction to the next. A carried whole cycle adds to
        // the stall only below the shared stall's cap.
        let carry = c.divide_carry + s.divide_fraction;
        let (stalled, divide, divide_carry) = if window != 0 {
            (0, 0, c.divide_carry)
        } else if carry >= 256 && stalled < k::PPU_SMT_SHARED_STALL_CYCLES {
            (stalled + 1, u64::from(s.divide_cycles) + 1, carry - 256)
        } else {
            (stalled, u64::from(s.divide_cycles), carry.min(255))
        };
        // A new dispatch group: the sibling's group issues between. The
        // sibling's stall covers the hold, and so does any wait of this
        // thread's own; only the rest of the hold is the sibling's. A stall
        // behind the sibling's divides stops the sibling too, so it covers
        // none of the hold, up to the shared stall's cap on the two together.
        let past_hold = divide
            .min(stalled)
            .min(k::PPU_SMT_SHARED_STALL_CYCLES.saturating_sub(self.hold));
        let covers = stalled - past_hold;
        let held = if opens_group {
            let opener = (c.floor + self.hold)
                .saturating_sub(covers)
                .max(c.group_open + window);
            let held = opener.saturating_sub(t);
            t = t.max(opener);
            held
        } else {
            0
        };
        let began = t;
        t += stalled;
        // [CBE-Handbook p:310 s:10.5] with TSCR[DISP_CNT] = 1 an outranking
        // sibling takes every dispatch slot it can use. The gate's length is
        // the `ppu_issue_sibling` capture's: there a lower-priority thread
        // beside a dependent chain issues about once to each of the chain's
        // instructions, though the chain cannot use the slots between them.
        let gated = if s.issue_interval != 0 {
            let gate = c.gate >> 8;
            let gated = gate.saturating_sub(t);
            t = t.max(gate);
            gated
        } else {
            0
        };
        if op.words != 0 {
            t += k::PPU_MICROCODE_ROM_CYCLES;
        }
        // [CBE-Handbook p:301 s:10.2.3.1] the VSU-issue stall point, VQ8,
        // is not separate for each thread. An op the fence holds leaves the
        // queue behind the sibling's VSU work, and any other VSU op behind
        // the sibling's ops its fence holds. In the `ppu_issue_vscr_smt`
        // capture, the sibling of an `mtvscr` and seven `vaddubm` keeps 0.33
        // of its rate beside a chain of an `mtvscr` and four `vaddubm`, and
        // 0.40 beside eight dependent `fadd`. Without the wait the model
        // gives 0.45 and 0.66, with it 0.37 and 0.37.
        let exec = if fence > t {
            fence + u64::from(s.queue_cycles)
        } else if fenced {
            t + u64::from(s.fence_cycles)
        } else {
            t
        };
        // An independent VSU op younger than an `mfvscr` runs at the
        // `mfvscr`'s hold, before the `mfvscr` result. In the
        // `ppu_issue_mfvscr_gap` capture an `mfvscr` and an `fadd` take
        // 41.00 cycles an iteration, one more than an `mfvscr` alone.
        // [CBE-Handbook p:757 s:A.6.2] IS2 sends instructions to the VSU
        // issue queue in pairs. [CBE-Handbook p:722 s:A.1.2] an `mffs`
        // stalls at VQ8. Only the op in the `mffs`'s own pair waits for the
        // `mffs` result. In the `ppu_issue_mffs_gap` capture an `mffs` and an
        // `fadd` in one pair take 61.00 cycles an iteration. In the
        // `ppu_issue_mffs_align` capture the same two words across a pair
        // boundary take 31.00.
        // [CBE-Handbook p:756 s:A.6.2] the LSU part of a VXU or FPU load or
        // store leaves at issue, beside the VSU issue queue, so the wait
        // leaves it out; `retire` delays a floating-point store's data.
        let exec = if uses_vsu(op) && !matches!(op.kind, Kind::Load { .. } | Kind::Store { .. }) {
            exec.max(fpscr_floor(c, pc, opens))
        } else {
            exec
        };
        Scheduled {
            issue: t,
            exec,
            queued,
            sibling: held + stalled + gated,
            stalled,
            held,
            began,
            behind_divide,
            divide_carry,
            opens_group,
        }
    }

    /// Issue each op of the instruction at `pc`, and return its issue cycle.
    ///
    /// `next_pc` is the address the thread runs after the instruction. A
    /// fused pair's second op is the instruction at `pc + 4`. The first op
    /// has `pc + 4` as its next address, and the second has `next_pc`. A
    /// slot with no op holds 0.
    #[inline]
    pub(crate) fn retire_instruction(&mut self, pc: u64, ops: &[Op], next_pc: u64) -> [u64; 2] {
        let mut issued = [0; 2];
        debug_assert!(
            ops.len() <= issued.len(),
            "an instruction cracks into at most {} ops, got {}",
            issued.len(),
            ops.len()
        );
        for (index, (op, slot)) in ops.iter().zip(&mut issued).enumerate() {
            let at = pc.wrapping_add(4 * index as u64);
            let next = if index + 1 == ops.len() {
                next_pc
            } else {
                at.wrapping_add(4)
            };
            *slot = self.retire(at, op, next);
        }
        issued
    }

    /// Issue `op` at `pc`, and return the cycle it issues at.
    ///
    /// `next_pc` is the address the thread runs next. A branch reads it:
    /// - to check a branch to LR against the link stack's prediction;
    /// - to find a direct branch to `pc + 4` ([`lands_on_fall_through`]).
    fn retire(&mut self, pc: u64, op: &Op, next_pc: u64) -> u64 {
        let Scheduled {
            issue: t,
            exec,
            queued,
            sibling,
            stalled,
            held,
            began,
            behind_divide,
            divide_carry,
            opens_group,
        } = self.schedule(pc, op);
        let (fetched, opens) = self.fetch(pc);
        let outranked = u64::from(self.shared_stall.issue_interval);
        let c = &mut self.core;
        c.gate = c.gate.max(t.saturating_mul(256)).saturating_add(outranked);
        c.divide_carry = divide_carry;
        if opens_group {
            c.group_open = t;
        }
        if divides(op) && op.spaces_sibling {
            note_divide(c, t);
        }
        stretch_pending(c, &mut self.tables, began, stalled);
        let before = c.floor;
        if opens {
            c.block_open = true;
            c.block_cycle = fetched;
            // The last block of the effective space ends at 2^64, which
            // wraps to 0 as `next_pc` past its last word does.
            c.block_end = (pc & !(k::PPU_FETCH_BYTES - 1)).wrapping_add(k::PPU_FETCH_BYTES);
        }
        c.next_pc = pc.wrapping_add(4);

        let last = t + u64::from(op.span.saturating_sub(1));
        c.floor = c.floor.max(last + 1);
        // `head.0` is the cycles the op holds the head of the VSU issue
        // queue, past the op before it there. `head.1` is the same count
        // for an op that this thread's own fence holds, and zero otherwise.
        let head = if uses_vsu(op) && op.kind != Kind::VscrWrite {
            let head = exec
                .saturating_sub(c.vsu_queue.max(before))
                .saturating_sub(sibling);
            (head, if c.vsu_fence > before { head } else { 0 })
        } else {
            (0, 0)
        };
        note_pressure(
            c,
            op,
            (c.floor - before).saturating_sub(sibling),
            held,
            queued,
            head,
        );
        c.older = (pc.is_multiple_of(8) && op.words == 0).then_some(Older {
            pc,
            issue: t,
            op: *op,
        });
        c.ahead[(c.retired % AHEAD_RING as u64) as usize] = t;
        c.retired += 1;

        if exec > t {
            c.vsu_queue = exec + 1;
        }
        // [CBE-Handbook p:757 s:A.6.2] an issue cycle with no VSU
        // instruction sends a bubble down the queue, and the queue does not
        // compress bubbles. So while the queue holds an op behind a divide,
        // each instruction that does not use the VSU takes a stage too. That
        // stage leaves with the stage ahead of it.
        let stage = if behind_divide {
            Some(exec)
        } else {
            (!uses_vsu(op) && c.viq_tail > t).then_some(c.viq_tail)
        };
        if let Some(leaves) = stage {
            c.viq[c.viq_next] = leaves;
            c.viq_next = (c.viq_next + 1) % k::PPU_VIQ_HELD_OPS;
            c.viq_tail = leaves;
        }
        if divides(op) && uses_vsu(op) {
            if let Kind::Unpipelined(cycles) = op.kind {
                c.divide_done = exec + u64::from(cycles);
            }
        }
        mark_destinations(c, op, exec);
        if let Some(done) = vsu_completion(op, exec, k::PPU_MFFS_VSU_COMPLETE_CYCLES) {
            c.vsu_done = c.vsu_done.max(done);
        }
        if let Some(done) = vsu_completion(op, exec, k::PPU_MFVSCR_VSU_COMPLETE_CYCLES) {
            c.vscr_done = c.vscr_done.max(done);
        }
        if let Kind::Unpipelined(cycles) = op.kind {
            for u in unit_slots(op.units) {
                c.unit_free[u] = exec + u64::from(cycles);
            }
        }
        if op.kind == Kind::VscrWrite {
            for u in unit_slots(op.units) {
                c.unit_free[u] = t + k::PPU_MTVSCR_HOLD_CYCLES;
            }
            c.vsu_fence = t + k::PPU_MTVSCR_YOUNGER_VSU_CYCLES;
        }
        // [CBE-Handbook p:720 s:A.1.2] a floating-point store goes to the
        // FPU as well as the LSU.
        // A drain that uses the VSU, other than an `mffs`, waits for its data
        // to reach the store queue, as the `ppu_issue_vscr_mem` captures show
        // (`PPU_FP_STORE_DATA_CYCLES`). A drain that does not use the VSU
        // waits for the store to complete, as the `ppu_issue_fp_store_drain`
        // capture shows (`PPU_FP_STORE_COMPLETE_CYCLES`).
        //
        // The store sends its data no earlier than an older `mffs`'s result
        // is ready or an older `mfvscr` completes. In the
        // `ppu_issue_fp_store_drain` and `ppu_issue_fp_store_vscr` captures
        // a loop of a floating-point store and an `mfvscr` takes 53.00
        // cycles an iteration, 13 more than with an `std`.
        //
        // For a younger `mffs`, the store's data is a vector-scalar op like
        // any other. It takes the floors of the last `mffs` (`fpscr_floor`)
        // and completes `PPU_MFFS_VSU_COMPLETE_CYCLES` after it runs. In the
        // `ppu_issue_mffs_stfd` and `ppu_issue_mffs_far` captures, a loop
        // with an `stfd` takes the cycles an iteration of the same loop with
        // an `fadd`. This holds at every distance from an `mffs` and at
        // either word.
        if matches!(op.kind, Kind::Store { .. }) && op.reads().any(|(r, _)| is_fpr(r)) {
            let sent = exec.max(c.status_read);
            let data = exec.max(fpscr_floor(c, pc, opens));
            c.fp_store_read = c.fp_store_read.max(data + k::PPU_MFFS_VSU_COMPLETE_CYCLES);
            c.fp_store_data = c.fp_store_data.max(sent + k::PPU_FP_STORE_DATA_CYCLES);
            c.fp_store_done = c.fp_store_done.max(sent + k::PPU_FP_STORE_COMPLETE_CYCLES);
        }
        note_store_done(c, op, t, last);
        c.fpscr_partner = None;
        if op.is_fpscr_read() {
            c.status_read = exec + k::PPU_MFFS_LATENCY_CYCLES;
            c.fpscr_partner = pc.is_multiple_of(8).then_some(pc.wrapping_add(4));
            c.odd_target_floor = exec + k::PPU_MFFS_ODD_TARGET_VSU_CYCLES;
            c.fpscr_queue_floor = exec + k::PPU_MFFS_YOUNGER_VSU_CYCLES;
        }
        if op.is_vscr_read() {
            c.status_read = exec + k::PPU_MFVSCR_LATENCY_CYCLES;
            c.vscr_done = c.vscr_done.max(c.status_read);
            c.vscr_hold = t + k::PPU_MFVSCR_YOUNGER_HOLD_CYCLES;
        }

        match op.kind {
            Kind::Load {
                ea,
                ra,
                len,
                class,
                tail,
            } => self.load(exec, op, Operand { ea, ra, len, tail }, class),
            Kind::Store {
                ea, ra, len, tail, ..
            } => {
                queue_store(&mut self.tables, exec, op, Operand { ea, ra, len, tail });
            }
            Kind::ZeroLine { ra, .. } => self.tables.caches.zero(t, ra),
            Kind::Touch { ea, ra } => self.tables.caches.touch(t, ea, ra),
            Kind::Flush { ra, .. } => self.tables.caches.flush(t, ra),
            Kind::StoreBack { ra, .. } => self.tables.caches.store_back(t, ra),
            Kind::Branch {
                bo,
                link,
                target,
                taken,
            } => {
                self.branch(pc, t, bo, link, target, taken, next_pc);
                return t;
            }
            Kind::Barrier(barrier) => {
                // [CBE-Handbook p:722 s:A.1.2] after a `sync` or `lwsync`
                // issues, it forces older stores out of the store queue.
                let queued = self.tables.stores.iter().any(|s| s.done > t);
                let back =
                    barrier_start(&self.tables, t, barrier) + barrier_cycles(barrier, queued);
                let c = &mut self.core;
                c.floor = c.floor.max(back);
                // The next instruction issues no earlier than `back`, so it
                // does not pair with the barrier as the older of a group.
                c.older = None;
                // [CBE-Handbook p:723 s:A.1.2] the IU performs a flush when
                // an `isync` completes.
                if barrier == Barrier::Isync {
                    self.end_block(back + k::PPU_BRANCH_MISS_CYCLES);
                    return t;
                }
            }
            // An LV2 call's round trip depends on its outcome, which the
            // runtime commits after the batch yields; the thread charges
            // it when it resumes ([`PpeTiming::charge_syscall`]).
            Kind::Syscall {
                number: Some(number),
                args,
            } => {
                self.core.syscall = Some(PendingSyscall {
                    issue: t,
                    number,
                    args,
                    code: None,
                    resumed: false,
                    preempted: false,
                });
                self.end_block(t + 1);
                return t;
            }
            // A hypervisor call takes no LV2 round trip. No hypercall
            // number has a measured cost, so the issue model charges
            // nothing past the `sc`.
            Kind::Syscall { number: None, .. } => {
                self.end_block(t + 1);
                return t;
            }
            Kind::Plain | Kind::Unpipelined(_) | Kind::Drain | Kind::VscrWrite => {}
        }
        let c = &mut self.core;
        if c.next_pc == c.block_end {
            let next = c.block_cycle + k::PPU_FETCH_INTERVAL_CYCLES;
            self.end_block(next);
        }
        t
    }

    /// Drop the pending LV2 call: the `sc` took the system call interrupt,
    /// so no LV2 round trip follows it.
    pub(crate) fn drop_pending_syscall(&mut self) {
        self.core.syscall = None;
    }

    /// Note the code the pending LV2 call returned.
    pub(crate) fn note_syscall_return(&mut self, code: u64) {
        if let Some(call) = self.core.syscall.as_mut() {
            call.code = Some(code);
        }
    }

    /// Note that the thread left the PPE since its last batch.
    ///
    /// `preempted` is true when the thread left only for a higher-priority
    /// thread. A pending LV2 call takes its resumed round trip. A thread
    /// preempted between calls waits [`k::PPU_PREEMPTED_RESUME_CYCLES`]
    /// before its next instruction issues. [`Self::charge_syscall`]
    /// charges either.
    pub(crate) fn note_syscall_resumed(&mut self, preempted: bool) {
        match self.core.syscall.as_mut() {
            Some(call) => {
                call.resumed = true;
                call.preempted = preempted;
            }
            None => self.core.preempted |= preempted,
        }
    }

    /// Charge a preempted thread's resume wait, then the pending LV2 call's round trip.
    ///
    /// The next instruction issues no earlier than the round trip after the
    /// `sc`. The first of these that applies selects the round trip:
    /// - the resumed round trip, if the thread left the PPE before the call
    ///   returned;
    /// - the round trip of its committed return and arguments.
    ///
    /// A runnable sibling adds [`Self::sibling_call_cycles`] to the round trip.
    pub(crate) fn charge_syscall(&mut self) {
        if std::mem::take(&mut self.core.preempted) {
            self.return_from_call(self.core.floor + k::PPU_PREEMPTED_RESUME_CYCLES);
        }
        let Some(call) = self.core.syscall.take() else {
            return;
        };
        let [r3, r4] = call.args.map(u64::from);
        let trip = if call.resumed {
            k::ppu_syscall_resumed_call_round_trip_cycles(
                call.number,
                call.code,
                call.preempted,
                r3,
                r4,
            )
        } else {
            k::ppu_syscall_call_round_trip_cycles(call.number, call.code, r3, r4)
        };
        let trip = trip + self.sibling_call_cycles(trip);
        let c = &mut self.core;
        c.kernel_share = weigh_over(c.kernel_share, trip, true, KERNEL_WINDOW_CYCLES);
        self.return_from_call(call.issue + trip);
    }

    /// The cycles a runnable sibling adds to an LV2 call whose round trip
    /// alone is `trip`.
    ///
    /// The added share of `trip` grows linearly with the sibling's
    /// `kernel_share`. It goes from `PPU_SMT_SYSCALL_SIBLING_SHARE` for a
    /// sibling with no LV2 calls to `PPU_SMT_SYSCALL_KERNEL_SIBLING_SHARE`
    /// for a sibling in LV2 calls all its recent cycles.
    fn sibling_call_cycles(&self, trip: u64) -> u64 {
        if self.hold == 0 {
            return 0;
        }
        let per = |(num, den): (u64, u64)| 256 * num / den;
        let user = per(k::PPU_SMT_SYSCALL_SIBLING_SHARE);
        let kernel = per(k::PPU_SMT_SYSCALL_KERNEL_SIBLING_SHARE);
        let share = u64::from(self.shared_stall.kernel_share).min(256);
        let share = user + (kernel.saturating_sub(user)) * share / 256;
        trip * share / 256
    }

    /// Hold the next instruction until cycle `back`, when the call
    /// returns, and fetch from there.
    fn return_from_call(&mut self, back: u64) {
        self.core.floor = self.core.floor.max(back);
        // [`Core::stall_share`] counts no cycle of the call, so the gap
        // across it is no interval between back-to-back divides.
        self.core.last_divide = None;
        self.end_block(back);
    }

    fn end_block(&mut self, next_block: u64) {
        self.core.block_open = false;
        self.core.next_block = next_block;
    }

    /// Sets the ready cycles of a load's targets from:
    /// - the level where the load found its line;
    /// - a touch of the line still on its way;
    /// - the reloads that the L1 set of a missed line holds;
    /// - a flush or store-back still busy with the line;
    /// - a store to the same bytes still in the store queue.
    ///
    /// A load that waits for such a store also holds the issue of the
    /// instructions after it. The store queue and the sector check use
    /// the real address.
    fn load(&mut self, t: u64, op: &Op, at: Operand, class: LoadClass) {
        let tables = &mut *self.tables;
        let (level, queued) = tables
            .caches
            .load_queued(t, at, class == LoadClass::Reserve);
        let arrival = match level {
            Level::Memory => 0,
            Level::L1 | Level::L2 => tables.caches.arrival(t, at),
        };
        // The load starts when its L1 set can take its reload, and when a
        // flush or store-back still busy with its lines finishes.
        let start = t.max(queued).max(tables.caches.busy(t, at));
        let Operand { ra, len, .. } = at;
        let hit_lat = match class {
            LoadClass::Fixed | LoadClass::Reserve => k::PPU_LOAD_FX_USE_CYCLES,
            LoadClass::Float => k::PPU_FP_LOAD_LATENCY_CYCLES,
            LoadClass::Vector => k::PPU_VMX_LOAD_LATENCY_CYCLES,
        };
        let miss_lat = match level {
            Level::L1 => None,
            Level::L2 => Some(k::PPU_L2_LOAD_USE_CYCLES),
            Level::Memory => Some(k::PPU_MEMORY_LOAD_USE_CYCLES),
        };
        let end = ra.wrapping_add(u64::from(len.max(1)));
        // [CBE-Handbook p:762 s:A.7.4] a load that hits a store still in
        // the store queue waits until the store has written its data into
        // the cache, and is then recycled.
        //
        // The same note says the PPU recycles the load after all older
        // stores complete. The load waits only for the entries it overlaps:
        // - In the `ppu_issue_multiple` capture an `lwz` of the first word
        //   of an 8-word `stmw` takes 71 cycles an iteration. A wait for
        //   the second entry too gives 100.
        // - In the `ppu_issue_load_hit_store` capture three stores to
        //   other lines before or after the store to the loaded line add
        //   at most 3 cycles to a 46-cycle chain.
        // - In the `ppu_issue_store_gather_hit` capture a load of bytes in
        //   a stored 16-byte block that no store wrote takes 8 cycles an
        //   iteration, as a load of a line no store writes does.
        let forwarded = tables
            .stores
            .iter()
            .filter(|s| s.mask & bytes_in(s.block, ra, end) != 0)
            .map(|s| s.done)
            .filter(|done| *done > t)
            .max()
            .map(|done| {
                // A recycled microcoded load runs its microcode again.
                if op.words == 0 {
                    done
                } else {
                    done + k::PPU_MICROCODE_ROM_CYCLES + u64::from(op.span)
                }
            });
        let c = &mut self.core;
        // The recycled load holds the instructions after it until the
        // store writes the cache, and gets its data later.
        if let Some(f) = forwarded {
            let release = f - (k::PPU_LOAD_HIT_STORE_CYCLES - k::PPU_LOAD_HIT_STORE_RELEASE_CYCLES);
            if release > c.floor {
                c.floor = release;
                c.older = None;
            }
        }
        // [CBE-Handbook p:730 s:A.1.3.2] an integer load across a 32-byte
        // sector is flushed, refetched and split into microcode, so the
        // instructions after it issue again behind it.
        //
        // The flush comes after the load's wait for its L1 set. In the
        // `ppu_issue_touch_set_split` capture the sector costs a load
        // behind 2 touches of its set 85 cycles more than it costs one
        // that waits for no set.
        let split = sector_split_cycles(op, ra, len, class);
        if split != 0 && start + split > c.floor {
            c.floor = start + split;
            c.older = None;
        }
        let micro = u64::from(op.span) + split;
        let targets = op
            .dsts
            .iter()
            .map(|d| d.0)
            .filter(|r| *r != NO_REG && !is_update_target(op, *r))
            .chain(op.range_targets());
        let (data, addr) = match miss_lat {
            None => {
                let addr = if class == LoadClass::Fixed {
                    k::PPU_L1_LOAD_USE_CYCLES
                } else {
                    hit_lat
                };
                (start + micro + hit_lat, start + micro + addr)
            }
            Some(lat) => (start + micro + lat, start + micro + lat),
        };
        // The load can find a touched line in the cache before the line
        // arrives from memory. The load then waits for the line, but no
        // longer than its own trip to memory. That trip starts no earlier
        // than the touch's request when the touch waited for its L1 set.
        //
        // [CBE-Handbook p:146 s:6.1.6.2] by default, data prefetch has the
        // lowest priority of any queue in the load subunit.
        let arrival = arrival.min(start + micro + k::PPU_MEMORY_LOAD_USE_CYCLES);
        let waits = arrival > data;
        let (data, addr) = (data.max(arrival), addr.max(arrival));
        let (data, addr) = match forwarded {
            Some(f) => (data.max(f), addr.max(f)),
            None => (data, addr),
        };
        for r in targets {
            c.ready[r as usize] = data;
            c.addr_ready[r as usize] = addr;
            if miss_lat.is_some() || waits {
                c.miss[r as usize] = data;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn branch(
        &mut self,
        pc: u64,
        t: u64,
        bo: u8,
        link: bool,
        target: Target,
        taken: bool,
        next_pc: u64,
    ) {
        let taken = taken && !lands_on_fall_through(pc, bo, link, target, next_pc);
        let predictor = &mut self.tables.predictor;
        let guessed = if bo & PPC_BO_ALWAYS == PPC_BO_ALWAYS {
            true
        } else if let Some(hint) = static_prediction(bo) {
            hint
        } else {
            predictor.predict(pc, taken)
        };
        let target_ok = match target {
            Target::Direct | Target::Count => true,
            Target::Link => !taken || predictor.pop() == Some(next_pc),
        };
        if link && taken {
            predictor.push(pc.wrapping_add(4));
        }
        let block = self.core.block_cycle;
        let next = if guessed != taken || !target_ok {
            t + k::PPU_BRANCH_MISS_CYCLES
        } else if taken {
            block + k::PPU_TAKEN_BRANCH_FETCH_CYCLES
        } else {
            block + k::PPU_NOT_TAKEN_BRANCH_FETCH_CYCLES
        };
        self.end_block(next);
    }
}

/// Whether `op` issues to the VSU and to no other unit: an FP or VMX
/// instruction past the VSU issue queue.
fn issues_to_vsu(op: &Op) -> bool {
    let vsu = unit::VSU1 | unit::VSU2;
    op.units != 0 && op.units & !vsu == 0
}

/// Whether the VSU issue queue can hold `op` for a divide in progress,
/// past IS2: a pipelined or unpipelined FP or VMX instruction that issues
/// to the VSU alone and runs no microcode.
///
/// [CBE-Handbook p:756 s:A.6.2] the queue checks the dependencies of the
/// VSU instructions apart from IS2.
fn waits_in_queue(op: &Op) -> bool {
    issues_to_vsu(op) && op.words == 0 && matches!(op.kind, Kind::Plain | Kind::Unpipelined(_))
}

/// Whether `op` uses the VSU at all: an FP or VMX instruction, a VMX or
/// FP load or store among them.
///
/// [CBE-Handbook p:756 s:A.6.2] the VXU and FPU loads and stores pass
/// through the VSU issue queue.
fn uses_vsu(op: &Op) -> bool {
    op.units & (unit::VSU1 | unit::VSU2) != 0
}

/// The cycle every older vector-scalar instruction in `c` completes.
///
/// It is the latest of:
/// - the last FPR or VR result;
/// - the last CR field a vector-scalar instruction writes;
/// - the release of each VSU unit.
fn vsu_ready(c: &Core) -> Option<u64> {
    let results = (0..REGS as u8)
        .filter(|&r| is_fpr(r) || is_vr(r))
        .map(|r| c.ready[usize::from(r)]);
    let units = unit_slots(unit::VSU1 | unit::VSU2).map(|u| c.unit_free[u]);
    results
        .chain(units)
        .chain(std::iter::once(c.vsu_done))
        .max()
}

/// The earliest cycle the last `mffs` lets a younger vector-scalar op at
/// `pc` run at. `opens` says the op opens a fetch block.
///
/// Callers apply the floor to a VSU op other than a load or store, and to
/// a floating-point store's data. Such an op runs no earlier than:
/// - `fpscr_queue_floor`;
/// - the `mffs` result, when the op shares the `mffs`'s pair;
/// - `odd_target_floor`, when a taken branch lands on the op at an odd
///   word.
///
/// The `ppu_issue_mffs_odd_target` capture times an `mffs`, one or eight
/// `li`, a `b` and an `fadd`. With the `fadd` at an odd word an iteration
/// takes 40.00 cycles. At an even word it takes 32.00 or 37.00.
fn fpscr_floor(c: &Core, pc: u64, opens: bool) -> u64 {
    let partner = c.fpscr_partner == Some(pc);
    let odd_target = opens && !pc.is_multiple_of(8);
    c.fpscr_queue_floor
        .max(if partner { c.status_read } else { 0 })
        .max(if odd_target { c.odd_target_floor } else { 0 })
}

/// The cycle the vector-scalar op `op`, run at `exec`, completes for a
/// younger `mffs` or `mfvscr`: no earlier than `complete` cycles after it
/// runs.
///
/// Returns `None` when `op`:
/// - is a load or store;
/// - uses no VSU unit;
/// - writes no register.
fn vsu_completion(op: &Op, exec: u64, complete: u64) -> Option<u64> {
    if !uses_vsu(op) || matches!(op.kind, Kind::Load { .. } | Kind::Store { .. }) {
        return None;
    }
    // [CBE-Handbook p:722 s:A.1.2] an `mffs` waits for every older
    // vector-scalar op to complete, which is later than its result.
    // [CBE-Handbook p:742 s:A.3.2] so does an `mfvscr`. A
    // recording op completes as many cycles later as its CR field
    // follows its result.
    let lats = || {
        (op.dsts.iter())
            .filter(|d| d.0 != NO_REG)
            .map(|&(_, lat)| u64::from(lat))
    };
    let (first, last) = (lats().min()?, lats().max()?);
    Some(exec + first.max(complete) + (last - first))
}

/// The cycles from the last cycle the store `op` issues in to the cycle it
/// completes, for a younger `mfvscr`.
fn store_wait(op: &Op) -> u64 {
    if op.reads().any(|(r, _)| is_fpr(r)) {
        k::PPU_MFVSCR_FP_STORE_WAIT_CYCLES
    } else if op.reads().any(|(r, _)| is_vr(r)) {
        k::PPU_MFVSCR_VECTOR_STORE_WAIT_CYCLES
    } else {
        k::PPU_MFVSCR_STORE_WAIT_CYCLES
    }
}

/// Records the cycle the store `op`, issued from `t` to `last`, completes
/// for a younger `mfvscr`.
#[inline]
fn note_store_done(c: &mut Core, op: &Op, t: u64, last: u64) {
    let Kind::Store { ra, len, .. } = op.kind else {
        return;
    };
    let dwords = (ra >> 3, ra.wrapping_add(u64::from(len.max(1)) - 1) >> 3);
    let pending = c.store_done > t;
    let same = dwords.0 <= c.store_dwords.1 && c.store_dwords.0 <= dwords.1;
    let queued = match (pending, same) {
        (false, _) => 0,
        (true, false) => k::PPU_MFVSCR_QUEUED_STORE_CYCLES,
        (true, true) => k::PPU_MFVSCR_QUEUED_STORE_CYCLES + k::PPU_MFVSCR_SAME_DOUBLEWORD_CYCLES,
    };
    c.store_done =
        (last + store_wait(op) + queued).max(c.store_done + k::PPU_MFVSCR_STORE_GAP_CYCLES);
    c.store_dwords = dwords;
}

/// Marks the data and address of every register `op` writes ready
/// at `t` plus its latency.
#[inline]
fn mark_destinations(c: &mut Core, op: &Op, t: u64) {
    for &(r, lat) in &op.dsts {
        if r != NO_REG {
            c.ready[r as usize] = t + u64::from(lat);
            c.addr_ready[r as usize] = t + u64::from(lat);
        }
    }
    if let Some((_, _, lat)) = op.range {
        for r in op.range_targets() {
            c.ready[r as usize] = t + u64::from(lat);
            c.addr_ready[r as usize] = t + u64::from(lat);
        }
    }
    let written = op.writes().fold(0u128, |bits, r| bits | 1 << r);
    if op.writes_in_order() {
        c.in_order_writes |= written;
    } else {
        c.in_order_writes &= !written;
    }
}

/// Whether `r` is the address register a load with update writes, which
/// the FXU produces apart from the load's data.
///
/// The instruction updates only RA, the first address register. An
/// indexed form may load into its RB, and that target still waits for
/// the load.
///
/// [PPC-Book1 p:37 s:3.3.2] `lwzux` is an invalid form only for `RA = 0`
/// or `RA = RT`, so `RT = RB` is valid.
fn is_update_target(op: &Op, r: u8) -> bool {
    op.units & unit::FXU != 0 && op.addr[0] == r && op.words == 0
}

/// The cycles `barrier` holds the instructions after it once it issues,
/// with `queued` set while an older store is still in the store queue.
fn barrier_cycles(barrier: Barrier, queued: bool) -> u64 {
    let (alone, store) = match barrier {
        Barrier::Sync => (k::PPU_SYNC_CYCLES, k::PPU_SYNC_STORE_CYCLES),
        Barrier::Lwsync => (k::PPU_LWSYNC_CYCLES, k::PPU_LWSYNC_STORE_CYCLES),
        Barrier::Isync => (k::PPU_ISYNC_CYCLES, k::PPU_ISYNC_STORE_CYCLES),
    };
    if queued {
        alone + store
    } else {
        alone
    }
}

/// The cycles of an integer load's split across an L1 sector, or 0.
///
/// A load multiple, and a load whose bytes stay inside one sector, cost 0.
///
/// [CBE-Handbook p:730 s:A.1.3.2] a misaligned integer load that stays
/// inside one 32-byte sector performs as an aligned one.
fn sector_split_cycles(op: &Op, ea: u64, len: u8, class: LoadClass) -> u64 {
    let first = ea % k::PPU_L1_SECTOR_BYTES;
    if class != LoadClass::Fixed
        || op.words != 0
        || first + u64::from(len) <= k::PPU_L1_SECTOR_BYTES
    {
        return 0;
    }
    match len {
        // A word half crosses unless the sector ends between the halves.
        8 if !first.is_multiple_of(4) => {
            k::PPU_SECTOR_CROSS_LOAD_CYCLES + k::PPU_SECTOR_CROSS_BYTE_RETRY_CYCLES
        }
        4 => k::PPU_SECTOR_CROSS_WORD_LOAD_CYCLES,
        2 => k::PPU_SECTOR_CROSS_HALFWORD_LOAD_CYCLES,
        _ => k::PPU_SECTOR_CROSS_LOAD_CYCLES,
    }
}
