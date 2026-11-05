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

use super::cache::{Caches, Level};
use super::op::{is_fpr, is_vr, unit, Barrier, Kind, LoadClass, Op, Target, NO_REG, REGS};
use super::predictor::Predictor;
use cellgov_exec::PpeIssuePressure;
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
use cellgov_ps3_abi::hw::ppu_issue as k;

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
    /// The cycle the sibling's stall began at.
    began: u64,
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
    /// The cycle the last floating-point store's data reaches the store
    /// queue. A drain that uses the VSU issues no earlier.
    fp_store_data: u64,
    /// The cycle the last floating-point store completes. A drain that
    /// does not use the VSU issues no earlier.
    fp_store_done: u64,
    /// The cycle the last store completes, for a younger `mfvscr`. Stores
    /// complete in order.
    store_done: u64,
    /// The cycle the last `mfvscr` result is ready. A younger VSU op
    /// other than a load or store runs no earlier. A younger
    /// floating-point store sends its data no earlier.
    vscr_read: u64,
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
}

// `Core::in_order_writes` holds one bit for each tracked register.
const _: () = assert!(REGS <= u128::BITS as usize);

/// An `sc` the thread retired, charged when the thread resumes after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingSyscall {
    /// The cycle the `sc` issued at.
    issue: u64,
    /// The syscall number.
    number: u64,
    /// The code the call returned, once the runtime commits it.
    code: Option<u64>,
}

/// [`Core::divide_share`] for a thread that spends all its cycles on
/// divides and square roots.
const SHARE_ONE: u32 = 1 << 16;

/// The cycles [`Core::divide_share`] and [`Core::alone_share`]
/// average over.
const PRESSURE_WINDOW_CYCLES: u64 = 64;

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
                fp_store_data: 0,
                fp_store_done: 0,
                store_done: 0,
                vscr_read: 0,
                vscr_hold: 0,
                in_order_writes: 0,
                divide_share: 0,
                alone_share: 0,
                vscr_share: 0,
                drain_share: 0,
                operand_wait: 0,
                queue_wait: 0,
                fence_wait: 0,
                interval: 0,
                gate: 0,
                syscall: None,
            },
            tables: Box::new(Tables {
                caches: Caches::new(),
                predictor: Predictor::new(),
                stores: [Queued::default(); PENDING_ENTRIES],
                next_store: 0,
                slots: [0; k::PPU_STORE_QUEUE_ENTRIES],
                next_slot: 0,
            }),
            hold: 0,
            shared_stall: PpeIssuePressure::NONE,
        }
    }

    /// Set the number of other PPE threads runnable, and the field by
    /// field largest [`PpeTiming::pressure`] among them, for the
    /// instructions retired from here on.
    pub(crate) fn set_siblings(&mut self, siblings: u32, pressure: PpeIssuePressure) {
        if siblings == 0 {
            self.hold = 0;
            self.shared_stall = PpeIssuePressure::NONE;
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
        let divide = share(cap, c.divide_share);
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
            issue_interval: c.interval,
        }
    }

    /// The thread's cycle: the earliest the next instruction can issue.
    pub(crate) fn now(&self) -> u64 {
        self.core.floor
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
            Some(older) if !opens && pc == older.pc + 4 => {
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
        for (r, after) in op.reads() {
            t = t.max(c.ready[r as usize].saturating_sub(after));
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
            t = t.max(c.miss[r as usize]);
            if c.in_order_writes & (1 << r) == 0 {
                t = t.max(c.ready[r as usize]);
            }
        }
        // [CBE-Handbook p:756 s:A.6.2] a VSU instruction leaves IS2 for the
        // VSU issue queue and waits at VQ8. So the fence holds the op past
        // IS2, and the younger instructions continue to issue. In the
        // `ppu_issue_vscr_queue` capture `mtvscr; vaddubm` and the same with
        // up to 32 `addi` after it each take 58 cycles an iteration. An op
        // the fence holds waits in the queue for its VSU unit too, and the
        // queue releases the ops it holds in order, one a cycle.
        let fenced = uses_vsu(op) && op.kind != Kind::VscrWrite;
        let queued_units = if fenced && c.vsu_fence > floor {
            op.units & (unit::VSU1 | unit::VSU2)
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
        if matches!(
            op.kind,
            Kind::Drain | Kind::Barrier(_) | Kind::VscrWrite | Kind::Syscall { .. }
        ) {
            let all = c.ready.iter().chain(c.unit_free.iter()).copied().max();
            // [CBE-Handbook p:742 s:A.3.2] a drain issues once every older
            // instruction completes. An op that the queue held completes a
            // cycle after it runs.
            // A queue that never held an op leaves `vsu_queue` at zero.
            let after_queue = if c.vsu_queue == 0 { 0 } else { c.vsu_queue + 1 };
            let fp_store = if uses_vsu(op) {
                c.fp_store_data
            } else {
                c.fp_store_done
            };
            // [CBE-Handbook p:742 s:A.3.2] an `mfvscr` waits at issue for
            // older stores to complete as well.
            let store = if op.is_vscr_read() { c.store_done } else { 0 };
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
        if let Kind::Store { ea, len, .. } = op.kind {
            t = queue_room(&self.tables, t, ea, len);
        }
        let s = &self.shared_stall;
        let own_vscr = op.kind == Kind::VscrWrite || (fenced && c.vsu_fence > c.floor);
        let own_wait = t.saturating_sub(floor.max(fetched));
        let stalled = sibling_stall(op, s, own_vscr, own_wait);
        // A new dispatch group: the sibling's group issues between. The
        // sibling's stall covers the hold, and so does any wait of this
        // thread's own; only the rest of the hold is the sibling's.
        let held = if opens_group {
            let opener = (c.floor + self.hold).saturating_sub(stalled);
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
        // A VSU op younger than an `mfvscr` runs no earlier than its
        // result. In the `ppu_issue_mfvscr_younger` capture an `mfvscr` and
        // four independent `vaddubm` take 51.48 cycles an iteration.
        // [CBE-Handbook p:756 s:A.6.2] the LSU part of a VXU or FPU load or
        // store leaves at issue, beside the VSU issue queue, so the wait
        // leaves it out; `retire` delays a floating-point store's data.
        let exec = if uses_vsu(op) && !matches!(op.kind, Kind::Load { .. } | Kind::Store { .. }) {
            exec.max(c.vscr_read)
        } else {
            exec
        };
        Scheduled {
            issue: t,
            exec,
            queued,
            sibling: held + stalled + gated,
            stalled,
            began,
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
            began,
        } = self.schedule(pc, op);
        let (fetched, opens) = self.fetch(pc);
        let outranked = u64::from(self.shared_stall.issue_interval);
        let c = &mut self.core;
        c.gate = c.gate.max(t.saturating_mul(256)).saturating_add(outranked);
        stretch_pending(c, &mut self.tables.stores, began, stalled);
        let before = c.floor;
        if opens {
            c.block_open = true;
            c.block_cycle = fetched;
            c.block_end = (pc & !(k::PPU_FETCH_BYTES - 1)) + k::PPU_FETCH_BYTES;
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
        mark_destinations(c, op, exec);
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
        // A drain that uses the VSU waits for its data to reach the store
        // queue, as the `ppu_issue_vscr_mem` captures show
        // (`PPU_FP_STORE_DATA_CYCLES`). Any other drain waits for the store
        // to complete, as the `ppu_issue_fp_store_drain` capture shows
        // (`PPU_FP_STORE_COMPLETE_CYCLES`).
        //
        // The store sends its data no earlier than an older `mfvscr`'s
        // result is ready. In the `ppu_issue_fp_store_drain` and
        // `ppu_issue_fp_store_vscr` captures a loop of a floating-point
        // store and an `mfvscr` takes 53.00 cycles an iteration, 13 more
        // than with an `std`.
        if matches!(op.kind, Kind::Store { .. }) && op.reads().any(|(r, _)| is_fpr(r)) {
            let sent = exec.max(c.vscr_read);
            c.fp_store_data = c.fp_store_data.max(sent + k::PPU_FP_STORE_DATA_CYCLES);
            c.fp_store_done = c.fp_store_done.max(sent + k::PPU_FP_STORE_COMPLETE_CYCLES);
        }
        if matches!(op.kind, Kind::Store { .. }) {
            c.store_done =
                (last + store_wait(op)).max(c.store_done + k::PPU_MFVSCR_STORE_GAP_CYCLES);
        }
        if op.is_vscr_read() {
            c.vscr_read = exec + k::PPU_MFVSCR_LATENCY_CYCLES;
            c.vscr_hold = t + k::PPU_MFVSCR_YOUNGER_HOLD_CYCLES;
        }

        match op.kind {
            Kind::Load { ea, len, class } => self.load(exec, op, ea, len, class),
            Kind::Store { ea, len, .. } => queue_store(&mut self.tables, exec, op, ea, len),
            Kind::ZeroLine { ea } => self.tables.caches.zero(ea),
            Kind::Touch { ea } => self.tables.caches.touch(t, ea),
            Kind::Flush { ea } => self.tables.caches.flush(t, ea),
            Kind::StoreBack { ea } => self.tables.caches.store_back(t, ea),
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
                let back = t + barrier_cycles(barrier, queued);
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
            } => {
                self.core.syscall = Some(PendingSyscall {
                    issue: t,
                    number,
                    code: None,
                });
                self.end_block(t + 1);
                return t;
            }
            Kind::Syscall { number: None } => {
                self.return_from_call(t + k::PPU_SYSCALL_ROUND_TRIP_CYCLES);
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

    /// Note the code the pending LV2 call returned.
    pub(crate) fn note_syscall_return(&mut self, code: u64) {
        if let Some(call) = self.core.syscall.as_mut() {
            call.code = Some(code);
        }
    }

    /// Charge the pending LV2 call's round trip, by its outcome: the next
    /// instruction issues no earlier than the round trip after the `sc`.
    /// A call with no committed return takes the round trip of its success
    /// path.
    pub(crate) fn charge_syscall(&mut self) {
        let Some(call) = self.core.syscall.take() else {
            return;
        };
        let trip = match call.code {
            Some(code) => k::ppu_syscall_outcome_round_trip_cycles(call.number, code),
            None => k::ppu_syscall_round_trip_cycles(call.number),
        };
        self.return_from_call(call.issue + trip);
    }

    /// Hold the next instruction until cycle `back`, when the call
    /// returns, and fetch from there.
    fn return_from_call(&mut self, back: u64) {
        self.core.floor = self.core.floor.max(back);
        self.end_block(back);
    }

    fn end_block(&mut self, next_block: u64) {
        self.core.block_open = false;
        self.core.next_block = next_block;
    }

    /// Sets the ready cycles of a load's targets from:
    /// - the level where the load found its line;
    /// - a touch of the line still on its way;
    /// - a flush or store-back still busy with the line;
    /// - a store to the same bytes still in the store queue.
    ///
    /// A load that waits for such a store also holds the issue of the
    /// instructions after it.
    fn load(&mut self, t: u64, op: &Op, ea: u64, len: u8, class: LoadClass) {
        let tables = &mut *self.tables;
        let level = tables.caches.load(ea, len, class == LoadClass::Reserve);
        let arrival = match level {
            Level::Memory => 0,
            Level::L1 | Level::L2 => tables.caches.arrival(t, ea, len),
        };
        // The load starts when a flush or store-back still busy with its
        // lines finishes.
        let start = t.max(tables.caches.busy(t, ea, len));
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
        let end = ea.wrapping_add(u64::from(len.max(1)));
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
            .filter(|s| s.mask & bytes_in(s.block, ea, end) != 0)
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
        let split = sector_split_cycles(op, ea, len, class);
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
        // longer than its own trip to memory.
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

/// Whether `op` holds the FXU or the VSU longer than a multiply does: a
/// divide or a square root. Its cycles count in full toward a sibling's
/// stall, and toward the multiply hold a sibling repeats.
///
/// [CBE-Handbook p:301 s:10.2.3.1] an instruction that depends on a
/// divide in progress is a very long-latency stall condition. The captures
/// below are the evidence that the sibling waits through it.
///
/// The `ppu_issue_shared` capture: a sibling of divides adds about 13
/// cycles to each of a thread's instructions, and one of dependent
/// `mulld` about 5. The `ppu_issue_chain` capture's sibling runs one
/// `divd` to each `bdnz`, and still adds 13 or more: the divide's cycles
/// count, whatever the instructions between.
fn divides(op: &Op) -> bool {
    let long = matches!(op.kind, Kind::Unpipelined(cycles) if u64::from(cycles) > k::PPU_MULLD_LATENCY_CYCLES);
    long && op.units & (unit::FXU | unit::VSU1 | unit::VSU2) != 0
}

/// Whether `op` issues alone for long without dividing: a microcoded
/// instruction, or a long unpipelined one on another unit (`mfcr` and
/// `mfocrf` on the branch unit). Its cycles count in full toward a
/// sibling's stall, and toward no repeated multiply hold.
///
/// [CBE-Handbook p:753 s:A.5] a nonpipelined or microcoded instruction
/// issues alone.
fn issues_alone_for_long(op: &Op) -> bool {
    let long = matches!(op.kind, Kind::Unpipelined(cycles) if u64::from(cycles) > k::PPU_MULLD_LATENCY_CYCLES);
    !divides(op) && (op.words != 0 || long)
}

/// Whether `op` issues to the VSU and to no other unit: an FP or VMX
/// instruction past the VSU issue queue.
fn issues_to_vsu(op: &Op) -> bool {
    let vsu = unit::VSU1 | unit::VSU2;
    op.units != 0 && op.units & !vsu == 0
}

/// Whether `op` uses the VSU at all: an FP or VMX instruction, a VMX or
/// FP load or store among them.
///
/// [CBE-Handbook p:756 s:A.6.2] the VXU and FPU loads and stores pass
/// through the VSU issue queue.
fn uses_vsu(op: &Op) -> bool {
    op.units & (unit::VSU1 | unit::VSU2) != 0
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

/// The cycles a multiply holds the FXU, or `None` for any other
/// instruction: a multiply is an unpipelined FXU instruction that holds
/// it no longer than `mulld` does.
///
/// [CBE-Handbook p:718 s:A.1.2] the FXU does not pipeline `mulli`,
/// `mullw`, `mulhw`, `mulld` or their non-recording forms, and each
/// causes a stall after issue. The recording forms run as microcode.
///
/// Beside a sibling of divides or square roots a multiply waits
/// [`PPU_SMT_SHARED_STALL_CYCLES`] and then this hold again. In the
/// `ppu_issue_multiply` capture, beside dependent `divd` or `fdiv`, the
/// slowest run of each multiply chain waits 23 to 33 cycles an
/// instruction past the chain alone. Beside `fdiv` the means are 22 for
/// `mulli` and 25 to 27 for `mullw`, `mulhw` and `mulld`; beside `divd`,
/// 24 to 26 for those three, and 10 for `mulli`, whose runs spread from
/// 1 to 27. Beside a microcoded `lha` sibling a `mulli` waits 14.8 to
/// 16.2 cycles in the `ppu_issue_shared` capture, and its `add`, `addi`
/// and `ld` chains 12.5 to 14.4.
///
/// [`PPU_SMT_SHARED_STALL_CYCLES`]: k::PPU_SMT_SHARED_STALL_CYCLES
fn multiply_hold(op: &Op) -> Option<u64> {
    match op.kind {
        Kind::Unpipelined(cycles)
            if u64::from(cycles) <= k::PPU_MULLD_LATENCY_CYCLES && op.units == unit::FXU =>
        {
            Some(u64::from(cycles))
        }
        _ => None,
    }
}

/// The cycles the sibling's stalls in `s` add to `op`.
///
/// `own_vscr` marks this thread's own `mtvscr` or a wait at its own
/// `mtvscr` fence. `own_wait` is the cycles `op` already waits past its
/// in-order floor and its fetch.
fn sibling_stall(op: &Op, s: &PpeIssuePressure, own_vscr: bool, own_wait: u64) -> u64 {
    // [CBE-Handbook p:301 s:10.2.3.1] the sibling's stalls at IS2
    // stall this thread too. In the `ppu_issue_units` capture the
    // younger of a pair waits the stall as well.
    let stalled = match multiply_hold(op) {
        Some(hold) => {
            u64::from(s.multiply_cycles) + (hold * u64::from(s.multiply_hold_share) + 128) / 256
        }
        None if issues_to_vsu(op) => u64::from(s.vsu_cycles),
        None => u64::from(s.cycles),
    };
    // A sibling's `mtvscr` stall overlaps this thread's own `mtvscr`
    // and its wait at its own fence. The `ppu_issue_vscr_smt` capture
    // times a loop of an `mtvscr` and four `vaddubm` beside a sibling of
    // `mtvscr`. On the mean of five runs, the model with the overlap lies
    // 22% over the console, and with no overlap 37% over.
    if own_vscr {
        return stalled;
    }
    // Each instruction waits the sibling's drain at IS2. A VSU instruction
    // waits it only past its own wait at issue: for its operands, its
    // unit, or an older `mfvscr` or drain. In the
    // `ppu_issue_vscr_smt_runs` capture the sibling runs an `mtvscr` and
    // seven `vaddubm`. Beside it, 8 independent `vaddubm` take 63.54 cycles
    // an iteration on the median shared run, and 8 `li` take 57.84. In the
    // `ppu_issue_vscr_smt` capture 8 dependent `fadd` beside that sibling
    // take 117.40. A store waits all of the drain and only a share of the
    // `mtvscr` stall (the `ppu_issue_vscr_smt_drain` capture).
    let drain = if issues_to_vsu(op) {
        u64::from(s.drain_cycles).saturating_sub(own_wait)
    } else {
        u64::from(s.drain_cycles)
    };
    let vscr = if matches!(op.kind, Kind::Store { .. }) {
        let (num, den) = k::PPU_SMT_MTVSCR_STORE_STALL_SHARE;
        u64::from(s.vscr_cycles) * num / den
    } else {
        u64::from(s.vscr_cycles)
    } + drain;
    stalled.max(vscr)
}

/// One instruction's `w` cycles in a share averaged over
/// [`PRESSURE_WINDOW_CYCLES`]: toward [`SHARE_ONE`] when `counts`, toward
/// zero otherwise.
fn weigh(share: u32, w: u64, counts: bool) -> u32 {
    let w = w.min(PRESSURE_WINDOW_CYCLES);
    let kept = u64::from(share) * (PRESSURE_WINDOW_CYCLES - w);
    let added = if counts { u64::from(SHARE_ONE) * w } else { 0 };
    ((kept + added) / PRESSURE_WINDOW_CYCLES) as u32
}

/// Delays each pending time by the length of the sibling's stall.
///
/// A time is pending when it is later than the start of the stall:
/// - each result and each outstanding load miss;
/// - each unit's release;
/// - each queued store's write into the cache.
///
/// The cycle a slot of the store queue is free again does not move.
///
/// The console captures show the delay:
/// - `ppu_issue_units`: a dependent `fadd` with an independent `addi`
///   after each waits its latency and both stalls.
/// - `ppu_issue_miss`: an L2 miss beside an `fdiv` sibling waits out
///   the stall of every instruction after it, about 40 cycles each
///   there, past the model's own stall.
/// - `ppu_issue_stall_release`, beside an `fdiv` sibling: a load of
///   bytes a store just queued costs more over a load of another line
///   than it does alone. This is also true with four `addi` between the
///   two, whose stalls outlast the store's write when that write does
///   not move.
/// - `ppu_issue_stall_release`, beside an `fdiv` sibling: a `divd` with
///   four independent loads after it costs most of what it costs alone
///   over an `addi` with the same loads. The loads' stalls outlast the
///   divide when its release does not move.
#[inline]
fn stretch_pending(c: &mut Core, stores: &mut [Queued], began: u64, stalled: u64) {
    if stalled == 0 {
        return;
    }
    let pending = c
        .ready
        .iter_mut()
        .chain(c.addr_ready.iter_mut())
        .chain(c.miss.iter_mut())
        .chain(c.unit_free.iter_mut())
        .chain(stores.iter_mut().map(|s| &mut s.done));
    for r in pending {
        if *r > began {
            *r += stalled;
        }
    }
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

/// The 16-byte block that holds `ea`.
fn block_of(ea: u64) -> u64 {
    ea / k::PPU_STORE_ENTRY_BYTES
}

/// The bytes of block `block` in `[ea, end)`, bit `i` for byte `i`.
fn bytes_in(block: u64, ea: u64, end: u64) -> u16 {
    let base = block.wrapping_mul(k::PPU_STORE_ENTRY_BYTES);
    let lo = ea.max(base) - base;
    let hi = end
        .min(base.saturating_add(k::PPU_STORE_ENTRY_BYTES))
        .saturating_sub(base);
    if lo >= hi {
        return 0;
    }
    ((1u32 << hi) - (1u32 << lo)) as u16
}

/// A store's parts as (address, bytes), one per 16-byte block it writes.
///
/// [PPC-Book1 p:49 s:3.3.6] an `stswx` with a byte count of zero stores
/// no bytes, so it has no part.
fn store_parts(ea: u64, len: u8) -> impl Iterator<Item = (u64, u8)> {
    let bytes = u64::from(len);
    let mut at = 0;
    std::iter::from_fn(move || {
        if at >= bytes {
            return None;
        }
        let here = ea.wrapping_add(at);
        let room = k::PPU_STORE_ENTRY_BYTES - here % k::PPU_STORE_ENTRY_BYTES;
        let part = (bytes - at).min(room);
        at += part;
        Some((here, part as u8))
    })
}

/// Whether a store's first part at `ea` gathers into the youngest entry.
///
/// It does when that entry holds bytes of the same 16-byte block. In the
/// `ppu_issue_store_queue_drain` capture, 32 `std` two to a block take 90
/// cycles an iteration, and 32 `std` one to a block take 253.
fn gathers(tables: &Tables, ea: u64) -> bool {
    let youngest = &tables.stores[(tables.next_store + PENDING_ENTRIES - 1) % PENDING_ENTRIES];
    youngest.mask != 0 && youngest.block == block_of(ea)
}

/// The new entries a store takes: one per 16-byte block, less one that gathers.
fn new_entries(tables: &Tables, ea: u64, len: u8) -> usize {
    let parts = store_parts(ea, len).count();
    parts.saturating_sub(usize::from(parts != 0 && gathers(tables, ea)))
}

/// The earliest cycle from `t` at which a store has room for its new entries.
///
/// The store waits:
/// - while the store queue is full, until the oldest entries leave it;
/// - while the pending entries are full, until the oldest entry writes
///   its bytes into the cache.
///
/// No entry is lost. In the `ppu_issue_store_queue_full` capture, 16
/// `std` and a load of the first one's bytes take 49 cycles an iteration.
/// With 17 `std` they take 52, so the load still waits for its store.
///
/// [CBE-Handbook p:762 s:A.7.4] a load that hits the store queue waits
/// until the store has written its data into the cache.
fn queue_room(tables: &Tables, t: u64, ea: u64, len: u8) -> u64 {
    let new = new_entries(tables, ea, len);
    let slot = (0..new.min(k::PPU_STORE_QUEUE_ENTRIES))
        .map(|i| tables.slots[(tables.next_slot + i) % k::PPU_STORE_QUEUE_ENTRIES])
        .max()
        .unwrap_or(0);
    let written = (0..new.min(PENDING_ENTRIES))
        .map(|i| &tables.stores[(tables.next_store + i) % PENDING_ENTRIES])
        .filter(|s| s.mask != 0)
        .map(|s| s.done)
        .max()
        .unwrap_or(0);
    t.max(slot).max(written)
}

/// Puts a store of `len` bytes at `ea`, issued at `t`, into the store queue.
///
/// The store takes one entry per 16-byte block. Its first part gathers
/// into the youngest entry when that entry holds the same block.
///
/// An entry keeps the bytes its stores wrote, so a load waits only for
/// the bytes it reads. When a store gathers into an entry that wrote its
/// bytes into the cache, the entry drops those bytes. In the
/// `ppu_issue_store_gather_hit` capture, an `stw`, 96 `addi`, an `stw` to
/// the same block and a load of the first word take 100 cycles an
/// iteration. With the second `stw` to another line they take 103.
///
/// Each entry takes a slot of the store queue until it leaves, and
/// pends until it writes its bytes into the cache. The entries of one
/// store leave [`PPU_STORE_SLOT_DRAIN_CYCLES`] apart and write
/// [`PPU_STORE_DRAIN_CYCLES`] apart.
///
/// [`PPU_STORE_SLOT_DRAIN_CYCLES`]: k::PPU_STORE_SLOT_DRAIN_CYCLES
/// [`PPU_STORE_DRAIN_CYCLES`]: k::PPU_STORE_DRAIN_CYCLES
#[inline]
fn queue_store(tables: &mut Tables, t: u64, op: &Op, ea: u64, len: u8) {
    tables.caches.store(ea, len);
    // [CBE-Handbook p:134 s:6.1.2.2] the CIU store queue holds
    // 16-byte entries. [CBE-Handbook p:144 s:6.1.5.7] the L2
    // store queue that takes each CIU request unloads first in,
    // first out. The interval between two entries of one store
    // is fitted to the `ppu_issue_multiple` capture. Each store
    // starts its own chain: in the `ppu_issue_load_hit_store`
    // capture, a store to the loaded line after three stores to
    // other lines takes as long as that store alone.
    let first = t + u64::from(op.span.saturating_sub(1)) + k::PPU_LOAD_HIT_STORE_CYCLES;
    let (mut done, mut leaves) = (first, first);
    let gather = gathers(tables, ea);
    let slots = k::PPU_STORE_QUEUE_ENTRIES;
    for (index, (at, part)) in store_parts(ea, len).enumerate() {
        let block = block_of(at);
        let mask = bytes_in(block, at, at.saturating_add(u64::from(part)));
        if index == 0 && gather {
            let youngest =
                &mut tables.stores[(tables.next_store + PENDING_ENTRIES - 1) % PENDING_ENTRIES];
            if youngest.done <= t {
                youngest.mask = 0;
            }
            youngest.mask |= mask;
            youngest.done = youngest.done.max(done);
            let slot = &mut tables.slots[(tables.next_slot + slots - 1) % slots];
            *slot = (*slot).max(leaves);
        } else {
            let entry = &mut tables.stores[tables.next_store];
            debug_assert!(
                entry.mask == 0 || entry.done <= t,
                "a store overwrites an entry that has not written its bytes"
            );
            *entry = Queued { block, mask, done };
            tables.next_store = (tables.next_store + 1) % PENDING_ENTRIES;
            debug_assert!(
                tables.slots[tables.next_slot] <= t,
                "a store takes a slot of the store queue that is not free"
            );
            tables.slots[tables.next_slot] = leaves;
            tables.next_slot = (tables.next_slot + 1) % slots;
        }
        done += k::PPU_STORE_DRAIN_CYCLES;
        leaves += k::PPU_STORE_SLOT_DRAIN_CYCLES;
    }
}

/// Add an instruction to the averages [`PpeTiming::pressure`] reads.
///
/// The instruction took `own` cycles of this thread's own. A drain spent
/// `queued` of them on the ops the VSU issue queue held. The instruction's
/// VSU work held the head of that queue `head.0` cycles, `head.1` of them
/// at the fence.
fn note_pressure(c: &mut Core, op: &Op, own: u64, queued: u64, head: (u64, u64)) {
    let n = PRESSURE_WINDOW_INSTRUCTIONS;
    let mean = |mean: u32, w: u64| {
        let w = w.saturating_mul(256).min(u64::from(u32::MAX));
        ((u64::from(mean) * u64::from(n - 1) + w) / u64::from(n)) as u32
    };
    let interval = own.saturating_mul(256).min(u64::from(u32::MAX));
    c.interval = ((u64::from(c.interval) * u64::from(n - 1) + interval) / u64::from(n)) as u32;
    c.queue_wait = mean(c.queue_wait, head.0);
    c.fence_wait = mean(c.fence_wait, head.1);
    let divide = divides(op);
    let alone = issues_alone_for_long(op);
    c.divide_share = weigh(c.divide_share, own, divide);
    c.alone_share = weigh(c.alone_share, own, alone);
    // An `mtvscr` that drains its own thread's held vector work leaves the
    // sibling most of its issue slots, so the drain counts in a share of
    // its own (`PPU_SMT_MTVSCR_DRAIN_STALL_SHARE`). In the
    // `ppu_issue_vscr_smt` capture a sibling of an `mtvscr` and seven
    // `vaddubm` slows a `vaddubm` chain from 17.9 to 61.4 cycles, and one
    // of back-to-back `mtvscr` to 220.
    let queued = queued.min(own);
    let vscr = op.kind == Kind::VscrWrite;
    c.vscr_share = weigh(c.vscr_share, queued, false);
    c.vscr_share = weigh(c.vscr_share, own - queued, vscr);
    c.drain_share = weigh(c.drain_share, own - queued, false);
    c.drain_share = weigh(c.drain_share, queued, vscr);
    if divide || alone {
        return;
    }
    let mut wait = own.saturating_sub(1).saturating_mul(256);
    // [CBE-Handbook p:756 s:A.6.2] a VSU instruction waits for its
    // operands in the VSU issue queue, past IS2.
    // [CBE-Handbook p:301 s:10.2.3.1] it stalls the sibling only once the
    // queue fills.
    if issues_to_vsu(op) {
        let (num, den) = k::PPU_SMT_VSU_STALL_SHARE;
        wait = wait.saturating_mul(num) / den;
    }
    let wait = u32::try_from(wait).unwrap_or(u32::MAX);
    c.operand_wait =
        ((u64::from(c.operand_wait) * u64::from(n - 1) + u64::from(wait)) / u64::from(n)) as u32;
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
