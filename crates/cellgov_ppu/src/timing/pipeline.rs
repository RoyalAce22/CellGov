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
//!   - Neither one is unpipelined, microcoded, a drain, a refetch or an
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
use super::op::{unit, Kind, LoadClass, Op, Target, NO_REG, REGS};
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
    /// The cycles of the wait the sibling's hold and stall added.
    sibling: u64,
    /// The sibling's stall alone.
    stalled: u64,
    /// The cycle the sibling's stall began at.
    began: u64,
}

/// The older instruction of an open dispatch group.
#[derive(Debug, Clone, Copy)]
struct Older {
    pc: u64,
    issue: u64,
    op: Op,
}

/// The scalar part of the model, which a faulted batch rolls back.
#[derive(Debug, Clone, Copy)]
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
    /// The share of this thread's recent cycles, out of [`SHARE_ONE`],
    /// spent on divides and square roots. Each instruction weighs by its
    /// cycles, over a window of [`PRESSURE_WINDOW_CYCLES`].
    divide_share: u32,
    /// The share of this thread's recent cycles spent on the other
    /// instructions that issue alone for long (`issues_alone_for_long`),
    /// weighed as [`Core::divide_share`] is.
    alone_share: u32,
    /// This thread's recent mean wait past its in-order floor, in 1/256
    /// cycle, over its other instructions: a wait for operands or for a
    /// busy unit. Each instruction weighs one in
    /// [`PRESSURE_WINDOW_INSTRUCTIONS`].
    operand_wait: u32,
    /// This thread's recent mean cycles between its own issues, in 1/256
    /// cycle, past what a sibling added. Each instruction weighs one in
    /// [`PRESSURE_WINDOW_INSTRUCTIONS`].
    interval: u32,
    /// The earliest cycle, in 1/256 cycle, an outranking sibling leaves
    /// this thread's next instruction a dispatch slot at.
    gate: u64,
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

/// A store still in the store queue.
#[derive(Debug, Clone, Copy, Default)]
struct Queued {
    ea: u64,
    len: u8,
    /// The cycle this entry writes its bytes into the cache.
    done: u64,
}

/// The tables a faulted batch keeps: the caches, the predictor and the
/// store queue. They advance on instructions the batch retires again.
#[derive(Clone)]
struct Tables {
    caches: Caches,
    predictor: Predictor,
    stores: [Queued; k::PPU_STORE_QUEUE_ENTRIES],
    next_store: usize,
}

/// The issue model of one PPU thread.
#[derive(Clone)]
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
            Kind::Unpipelined(_) | Kind::Syscall { .. } | Kind::Refetch | Kind::Drain
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
                divide_share: 0,
                alone_share: 0,
                operand_wait: 0,
                interval: 0,
                gate: 0,
            },
            tables: Box::new(Tables {
                caches: Caches::new(),
                predictor: Predictor::new(),
                stores: [Queued::default(); k::PPU_STORE_QUEUE_ENTRIES],
                next_store: 0,
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
        let mut t = floor.max(fetched);
        for &r in &op.srcs {
            if r != NO_REG {
                t = t.max(c.ready[r as usize]);
            }
        }
        for &r in &op.addr {
            if r != NO_REG {
                t = t.max(c.addr_ready[r as usize]);
            }
        }
        // [CBE-Handbook p:755 s:A.6.1] decode checks for write-after-write
        // hazards, and issue stalls at IS2 until they clear.
        for r in op.writes() {
            t = t.max(c.miss[r as usize]).max(c.ready[r as usize]);
        }
        for u in unit_slots(op.units) {
            t = t.max(c.unit_free[u]);
        }
        if matches!(op.kind, Kind::Drain | Kind::Refetch | Kind::Syscall { .. }) {
            let all = c.ready.iter().chain(c.unit_free.iter()).copied().max();
            t = t.max(all.unwrap_or(0));
        }
        // [CBE-Handbook p:301 s:10.2.3.1] the sibling's stalls at IS2
        // stall this thread too. In the `ppu_issue_units` capture the
        // younger of a pair waits the stall as well.
        let s = &self.shared_stall;
        let stalled = match multiply_hold(op) {
            Some(hold) => {
                u64::from(s.multiply_cycles) + (hold * u64::from(s.multiply_hold_share) + 128) / 256
            }
            None if issues_to_vsu(op) => u64::from(s.vsu_cycles),
            None => u64::from(s.cycles),
        };
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
        Scheduled {
            issue: t,
            sibling: held + stalled + gated,
            stalled,
            began,
        }
    }

    /// Issue `op` at `pc`.
    ///
    /// `next_pc` is the address the thread runs next. It gives the
    /// direction of a branch whose `taken` is `None`.
    pub(crate) fn retire(&mut self, pc: u64, op: &Op, next_pc: u64) {
        let Scheduled {
            issue: t,
            sibling,
            stalled,
            began,
        } = self.schedule(pc, op);
        let (fetched, opens) = self.fetch(pc);
        let outranked = u64::from(self.shared_stall.issue_interval);
        let c = &mut self.core;
        c.gate = c.gate.max(t.saturating_mul(256)).saturating_add(outranked);
        stretch_pending(c, began, stalled);
        let before = c.floor;
        if opens {
            c.block_open = true;
            c.block_cycle = fetched;
            c.block_end = (pc & !(k::PPU_FETCH_BYTES - 1)) + k::PPU_FETCH_BYTES;
        }
        c.next_pc = pc.wrapping_add(4);

        let last = t + u64::from(op.span.saturating_sub(1));
        c.floor = c.floor.max(last + 1);
        note_pressure(c, op, (c.floor - before).saturating_sub(sibling));
        c.older = (pc.is_multiple_of(8) && op.words == 0).then_some(Older {
            pc,
            issue: t,
            op: *op,
        });
        c.ahead[(c.retired % AHEAD_RING as u64) as usize] = t;
        c.retired += 1;

        mark_destinations(c, op, t);
        if let Kind::Unpipelined(cycles) = op.kind {
            for u in unit_slots(op.units) {
                c.unit_free[u] = t + u64::from(cycles);
            }
        }

        match op.kind {
            Kind::Load { ea, len, class } => self.load(t, op, ea, len, class),
            Kind::Store { ea, len, .. } => queue_store(&mut self.tables, t, op, ea, len),
            Kind::ZeroLine { ea } => self.tables.caches.zero(ea),
            Kind::Branch {
                bo,
                link,
                target,
                taken,
            } => {
                let taken = taken.unwrap_or(next_pc != pc.wrapping_add(4));
                self.branch(pc, t, bo, link, target, taken, next_pc);
                return;
            }
            Kind::Refetch => {
                self.end_block(t + k::PPU_BRANCH_MISS_CYCLES);
                return;
            }
            Kind::Syscall { number } => {
                let back = t + number.map_or(
                    k::PPU_SYSCALL_ROUND_TRIP_CYCLES,
                    k::ppu_syscall_round_trip_cycles,
                );
                self.core.floor = self.core.floor.max(back);
                self.end_block(back);
                return;
            }
            Kind::Plain | Kind::Unpipelined(_) | Kind::Drain => {}
        }
        let c = &mut self.core;
        if c.next_pc == c.block_end {
            let next = c.block_cycle + k::PPU_FETCH_INTERVAL_CYCLES;
            self.end_block(next);
        }
    }

    fn end_block(&mut self, next_block: u64) {
        self.core.block_open = false;
        self.core.next_block = next_block;
    }

    /// A load's result latency from where it found its line, and from a
    /// store to the same bytes still in the store queue.
    fn load(&mut self, t: u64, op: &Op, ea: u64, len: u8, class: LoadClass) {
        let tables = &mut *self.tables;
        let level = tables.caches.load(ea, class == LoadClass::Reserve);
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
        let forwarded = tables
            .stores
            .iter()
            .filter(|s| s.len != 0 && s.ea < end && ea < s.ea.wrapping_add(u64::from(s.len)))
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
        let micro = u64::from(op.span);
        let targets =
            op.dsts
                .iter()
                .map(|d| d.0)
                .filter(|r| *r != NO_REG && !is_update_target(op, *r))
                .chain(op.range.into_iter().flat_map(|(first, n, _)| {
                    (first..first.saturating_add(n)).filter(|r| *r < 32)
                }));
        let (data, addr) = match miss_lat {
            None => {
                let addr = if class == LoadClass::Fixed {
                    k::PPU_L1_LOAD_USE_CYCLES
                } else {
                    hit_lat
                };
                (t + micro + hit_lat, t + micro + addr)
            }
            Some(lat) => (t + micro + lat, t + micro + lat),
        };
        let (data, addr) = match forwarded {
            Some(f) => (data.max(f), addr.max(f)),
            None => (data, addr),
        };
        for r in targets {
            c.ready[r as usize] = data;
            c.addr_ready[r as usize] = addr;
            if miss_lat.is_some() {
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

/// One instruction's `w` cycles in a share averaged over
/// [`PRESSURE_WINDOW_CYCLES`]: toward [`SHARE_ONE`] when `counts`, toward
/// zero otherwise.
fn weigh(share: u32, w: u64, counts: bool) -> u32 {
    let w = w.min(PRESSURE_WINDOW_CYCLES);
    let kept = u64::from(share) * (PRESSURE_WINDOW_CYCLES - w);
    let added = if counts { u64::from(SHARE_ONE) * w } else { 0 };
    ((kept + added) / PRESSURE_WINDOW_CYCLES) as u32
}

/// Add an instruction that took `own` cycles of this thread's own to the
/// averages [`PpeTiming::pressure`] reads.
/// Pushes back every result still pending when the sibling's stall
/// began by the length of that stall.
///
/// In the `ppu_issue_units` capture a dependent `fadd` with an
/// independent `addi` after each waits its latency and both stalls. In
/// the `ppu_issue_miss` capture an L2 miss beside an `fdiv` sibling
/// waits out the stall of every instruction after it, about 40 cycles
/// each there, past the model's own stall.
#[inline]
fn stretch_pending(c: &mut Core, began: u64, stalled: u64) {
    if stalled == 0 {
        return;
    }
    for r in c.ready.iter_mut().chain(c.addr_ready.iter_mut()) {
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
    if let Some((first, n, lat)) = op.range {
        for r in (first..first.saturating_add(n)).filter(|r| *r < 32) {
            c.ready[r as usize] = t + u64::from(lat);
            c.addr_ready[r as usize] = t + u64::from(lat);
        }
    }
}

/// Puts a store of `len` bytes at `ea`, issued at `t`, into the store
/// queue, one entry per 16 bytes.
#[inline]
fn queue_store(tables: &mut Tables, t: u64, op: &Op, ea: u64, len: u8) {
    tables.caches.store(ea);
    // [CBE-Handbook p:134 s:6.1.2.2] the CIU store queue holds
    // 16-byte entries. [CBE-Handbook p:144 s:6.1.5.7] the L2
    // store queue that takes each CIU request unloads first in,
    // first out. The interval between two entries of one store
    // is fitted to the `ppu_issue_multiple` capture; the model
    // does not chain it across stores.
    let mut done = t + u64::from(op.span.saturating_sub(1)) + k::PPU_LOAD_HIT_STORE_CYCLES;
    // [PPC-Book1 p:49 s:3.3.6] an `stswx` with a byte count of
    // zero stores no bytes, so it takes no entry.
    let bytes = u64::from(len);
    let mut at = 0;
    while at < bytes {
        let part = (bytes - at).min(k::PPU_STORE_ENTRY_BYTES);
        tables.stores[tables.next_store] = Queued {
            ea: ea.wrapping_add(at),
            len: part as u8,
            done,
        };
        tables.next_store = (tables.next_store + 1) % k::PPU_STORE_QUEUE_ENTRIES;
        at += part;
        done += k::PPU_STORE_DRAIN_CYCLES;
    }
}

fn note_pressure(c: &mut Core, op: &Op, own: u64) {
    let n = PRESSURE_WINDOW_INSTRUCTIONS;
    let interval = own.saturating_mul(256).min(u64::from(u32::MAX));
    c.interval = ((u64::from(c.interval) * u64::from(n - 1) + interval) / u64::from(n)) as u32;
    let divide = divides(op);
    let alone = issues_alone_for_long(op);
    c.divide_share = weigh(c.divide_share, own, divide);
    c.alone_share = weigh(c.alone_share, own, alone);
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
