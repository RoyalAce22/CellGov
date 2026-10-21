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
use cellgov_ps3_abi::hw::ppc_isa::PPC_BO_ALWAYS;
use cellgov_ps3_abi::hw::ppu_issue as k;

/// The length of the ring of latest issue cycles that the fetch-ahead
/// bound reads.
const AHEAD_RING: usize = 32;

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
}

/// A store still in the store queue.
#[derive(Debug, Clone, Copy, Default)]
struct Queued {
    ea: u64,
    len: u8,
    issue: u64,
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
    /// this thread's dispatch groups: zero with no sibling runnable.
    hold: u64,
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
            Kind::Unpipelined(_) | Kind::Syscall | Kind::Refetch | Kind::Drain
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
            },
            tables: Box::new(Tables {
                caches: Caches::new(),
                predictor: Predictor::new(),
                stores: [Queued::default(); k::PPU_STORE_QUEUE_ENTRIES],
                next_store: 0,
            }),
            hold: 0,
        }
    }

    /// Set the number of other PPE threads runnable for the instructions
    /// retired from here on.
    pub(crate) fn set_siblings(&mut self, siblings: u32) {
        self.hold = if siblings == 0 {
            0
        } else {
            k::PPU_SMT_SIBLING_HOLD_CYCLES
        };
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
        let c = &self.core;
        let (fetched, opens) = self.fetch(pc);
        let floor = match c.older {
            Some(older) if !opens && pc == older.pc + 4 => {
                let alone = issues_alone(&older.op)
                    || issues_alone(op)
                    || !pairs(&older.op, op)
                    || op.reads_any_of(&older.op);
                older.issue + u64::from(alone)
            }
            // A new dispatch group: the sibling's group issues between.
            _ => c.floor + self.hold,
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
        if matches!(op.kind, Kind::Drain | Kind::Refetch | Kind::Syscall) {
            let all = c.ready.iter().chain(c.unit_free.iter()).copied().max();
            t = t.max(all.unwrap_or(0));
        }
        if op.words != 0 {
            t += k::PPU_MICROCODE_ROM_CYCLES;
        }
        t
    }

    /// Issue `op` at `pc`.
    ///
    /// `next_pc` is the address the thread runs next. It gives the
    /// direction of a branch whose `taken` is `None`.
    pub(crate) fn retire(&mut self, pc: u64, op: &Op, next_pc: u64) {
        let t = self.issue_cycle(pc, op);
        let (fetched, opens) = self.fetch(pc);
        let c = &mut self.core;
        if opens {
            c.block_open = true;
            c.block_cycle = fetched;
            c.block_end = (pc & !(k::PPU_FETCH_BYTES - 1)) + k::PPU_FETCH_BYTES;
        }
        c.next_pc = pc.wrapping_add(4);

        let last = t + u64::from(op.span.saturating_sub(1));
        c.floor = c.floor.max(last + 1);
        c.older = (pc.is_multiple_of(8) && op.words == 0).then_some(Older {
            pc,
            issue: t,
            op: *op,
        });
        c.ahead[(c.retired % AHEAD_RING as u64) as usize] = t;
        c.retired += 1;

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
        if let Kind::Unpipelined(cycles) = op.kind {
            for u in unit_slots(op.units) {
                c.unit_free[u] = t + u64::from(cycles);
            }
        }

        match op.kind {
            Kind::Load { ea, len, class } => self.load(t, op, ea, len, class),
            Kind::Store { ea, len, .. } => {
                let tables = &mut *self.tables;
                tables.caches.store(ea);
                tables.stores[tables.next_store] = Queued { ea, len, issue: t };
                tables.next_store = (tables.next_store + 1) % k::PPU_STORE_QUEUE_ENTRIES;
            }
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
            Kind::Syscall => {
                let back = t + k::PPU_SYSCALL_ROUND_TRIP_CYCLES;
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
        let forwarded = tables
            .stores
            .iter()
            .filter(|s| s.len != 0 && s.ea < end && ea < s.ea.wrapping_add(u64::from(s.len)))
            .map(|s| s.issue + k::PPU_LOAD_HIT_STORE_CYCLES)
            .filter(|ready| *ready > t)
            .max();
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
        for r in targets {
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
