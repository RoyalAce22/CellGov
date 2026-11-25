//! The batch loop: its entry, its tap, and the order of its steps.

use super::decode::CodeBytes;
use super::exit::{FaultExit, Retry};
use super::regions::RegionTable;
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::tap::PpuTap;
use crate::unit::ppu_unit::PpuExecutionUnit;
use crate::{exec, instruction, state};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
use cellgov_time::Budget;

/// What a batch rewinds to when a step inside it faults.
///
/// `snapshot` is `None` for a single-step batch: no step retires ahead
/// of a fault there, so the rollback needs no state clone.
pub(in crate::unit) struct BatchEntry {
    pub(in crate::unit) snapshot: Option<state::PpuState>,
    pub(in crate::unit) hashes: usize,
    pub(in crate::unit) fulls: usize,
    pub(in crate::unit) barriers: usize,
    pub(in crate::unit) code_events: usize,
    pub(in crate::unit) retired: u64,
    /// The issue model's cycle at entry.
    pub(in crate::unit) cycle: u64,
    /// The issue model's scalar state at entry, taken when a step can
    /// retire ahead of a fault.
    pub(in crate::unit) timing: Option<crate::timing::Core>,
    /// Whether the batch records code events.
    pub(in crate::unit) trace: bool,
}

/// The tap a unit with none installed runs its batch loop with.
pub(in crate::unit) struct NoTap;

impl PpuTap for NoTap {
    #[inline(always)]
    fn dispatch(
        &self,
        _unit: UnitId,
        _insn: &instruction::PpuInstruction,
        _state: &state::PpuState,
    ) {
    }
}

/// The state the batch loop carries from one instruction to the next.
pub(super) struct Cursor<'m> {
    pub(super) translating: bool,
    /// The span that holds every window, or `None` when memory has no
    /// window.
    pub(super) window_span: Option<(u64, u64)>,
    /// How many instructions the budget can still retire.
    pub(super) remaining: u64,
    /// The pc of the last instruction the batch retired.
    pub(super) last_retired: u64,
    /// A fused pair that faulted runs again split, at this pc.
    force_split: Option<u64>,
    /// An instruction that runs again after a translation miss, at
    /// this pc.
    rerun: Option<u64>,
    /// The span of the buffered stores when the last `isync` retired.
    pub(super) synced_stores: Option<(u64, u64)>,
    /// The fetchable span the last fetch read: its first byte and the
    /// last word that starts in it. It starts empty.
    pub(super) fetchable: (u64, u64),
    /// The base and bytes of the region that holds `fetchable`.
    pub(super) code: CodeBytes<'m>,
}

impl Cursor<'_> {
    /// The cursor at the start of a batch that `budget` bounds, with the
    /// next instruction at `pc`.
    #[inline(always)]
    fn start(translating: bool, window_span: Option<(u64, u64)>, budget: Budget, pc: u64) -> Self {
        Cursor {
            translating,
            window_span,
            remaining: budget.raw(),
            last_retired: pc,
            force_split: None,
            rerun: None,
            synced_stores: None,
            fetchable: (1, 0),
            code: (0, &[]),
        }
    }
}

impl PpuExecutionUnit {
    /// Run one batch and report each dispatch to `tap`.
    ///
    /// The compiler builds one copy per tap type, so the [`NoTap`] copy
    /// has no call in its per-instruction loop.
    pub(in crate::unit) fn run_batch<T: PpuTap + ?Sized>(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
        tap: &T,
    ) -> ExecutionStepResult {
        let entry = self.begin_batch(budget, ctx, effects);
        let mem = ctx.memory();

        // A thread that translates runs against views of its translations
        // at their effective addresses. It decodes each fetch fresh: the
        // shadow and the fused pairs key on addresses its translation can
        // move.
        let translating = self.translates();
        let mut regions = self.batch_views(mem, translating);
        // A window's bytes are another unit's state, which the runtime
        // updates only between batches. The batch ends at the instruction
        // that reaches a window, so the next instruction sees the update.
        let window_span = mem.window_span();
        self.profile_layout(&regions);
        let mut cur = Cursor::start(translating, window_span, budget, self.state.pc);

        loop {
            let step_pc = self.state.pc;
            let forced = cur.force_split.take() == Some(step_pc);
            if let Some(stop) = self.stop_ahead_of_fetch(ctx, &entry, effects, &cur, step_pc) {
                return stop;
            }
            let boundary = (effects.len(), self.store_buf.len());
            let decoded = self
                .fetch_step(mem, step_pc, translating, &mut cur.fetchable, &mut cur.code)
                .and_then(|insn| self.split_or_fused(ctx, &cur, insn, step_pc, forced));
            let insn = match decoded {
                Ok(insn) => insn,
                Err(fault) => {
                    let exit = FaultExit::Verdict(fault);
                    let last = cur.last_retired;
                    return self.end_at_fault(&entry, effects, step_pc, last, boundary, exit);
                }
            };

            // A rerun after a translation miss is the same dispatch.
            if cur.rerun.take() != Some(step_pc) {
                tap.dispatch(self.id, &insn, &self.state);
            }
            let (mut ops, n_ops) = crate::timing::classify(&insn, &self.state);
            if translating {
                self.real_op_addresses(&mut ops[..n_ops]);
            }
            let ops = &ops[..n_ops];
            self.align_clock_read_to_issue(ctx, &entry, step_pc, &insn, ops);
            let attempt = self.begin_attempt(mem, effects.len(), insn, step_pc, translating);
            let load = if self.profile_mode {
                self.probe_load(&insn, step_pc, regions.as_slice())
            } else {
                None
            };
            let fault = match exec::execute(
                &insn,
                &mut self.state,
                self.id,
                regions.as_slice(),
                effects,
                &mut self.store_buf,
            ) {
                ExecuteVerdict::Continue => {
                    self.state.pc += if attempt.pair { 8 } else { 4 };
                    None
                }
                ExecuteVerdict::Branch => None,
                ExecuteVerdict::Syscall { lev } => {
                    return self.sc_yield(ctx, &entry, effects, step_pc, ops, lev, translating);
                }
                ExecuteVerdict::Fault(f) => Some(FaultExit::Verdict(f)),
                ExecuteVerdict::MemFault(e) => Some(FaultExit::Mem(e, false)),
                ExecuteVerdict::StoreFault(e) => Some(FaultExit::Mem(e, true)),
                ExecuteVerdict::BufferFull if attempt.pair => {
                    // The second half found no room. Run split, so the
                    // first half retires and the second waits at its own
                    // address, as the unfused store does.
                    self.undo_fused_attempt(mem, effects, attempt.marks(), step_pc);
                    cur.force_split = Some(step_pc);
                    continue;
                }
                ExecuteVerdict::BufferFull => {
                    return self.full_buffer_yield(&entry, effects, step_pc);
                }
            };
            if let Some(exit) = fault {
                let boundary = (attempt.effects_before, attempt.stores_before);
                let last = cur.last_retired;
                match self.settle_fault(mem, effects, &mut regions, translating, &attempt, exit) {
                    Retry::Rerun => cur.rerun = Some(step_pc),
                    Retry::Split => cur.force_split = Some(step_pc),
                    Retry::EndBatch => return self.budget_yield(&entry, effects, last),
                    Retry::Stop(exit) => {
                        return self.end_at_fault(&entry, effects, step_pc, last, boundary, exit);
                    }
                }
                continue;
            }

            let last_pc = self.retire_step(ctx, effects, &attempt, ops, translating, load);
            if self.ends_after(ctx, &entry, effects, &mut cur, &attempt, last_pc) {
                return self.budget_yield(&entry, effects, last_pc);
            }
        }
    }

    /// Counts a change of region layout in the load profile, when the
    /// unit profiles.
    #[inline(always)]
    fn profile_layout(&mut self, regions: &RegionTable<'_>) {
        if self.profile_mode {
            let views = regions.as_slice().iter();
            self.load_profile
                .note_layout(views.map(|v| (v.base, v.bytes.len() as u64)));
        }
    }

    /// Retires the `sc` at `step_pc` and yields its LV2 call.
    ///
    /// A thread with supervisor registers takes the system call
    /// interrupt at the kernel's vector instead, or yields a hypercall.
    #[allow(clippy::too_many_arguments)]
    #[inline(always)]
    fn sc_yield(
        &mut self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        step_pc: u64,
        ops: &[crate::timing::Op],
        lev: u8,
        translating: bool,
    ) -> ExecutionStepResult {
        self.retire_syscall(ctx, step_pc, ops);
        if translating {
            return self.supervisor_sc_yield(entry, effects, step_pc, lev);
        }
        self.syscall_yield(entry, effects, step_pc, lev)
    }

    /// Yields at the store at `step_pc` that found the store buffer full.
    ///
    /// The pc stays at the store, which runs again in the next batch.
    #[inline(always)]
    fn full_buffer_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        step_pc: u64,
    ) -> ExecutionStepResult {
        // Break skips count retirements, and nothing retired, so the unit
        // hands back a skip it spent ahead of the fetch at `step_pc`.
        if self.break_pc == Some(step_pc) {
            self.break_skip += 1;
        }
        self.note_fetch(step_pc);
        self.budget_yield(entry, effects, step_pc)
    }

    /// The yield that ends the batch ahead of the fetch at `step_pc`, or
    /// `None` when the fetch goes ahead.
    ///
    /// The batch ends there at:
    /// - a fetch that a synced store can reach;
    /// - an interrupt at the instruction boundary;
    /// - a break;
    /// - a thread in supervisor state with SF clear.
    #[inline(always)]
    fn stop_ahead_of_fetch(
        &mut self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        cur: &Cursor<'_>,
        step_pc: u64,
    ) -> Option<ExecutionStepResult> {
        // [PPC-Book2 p:12 s:1.8] after `dcbst`, `sync`, `icbi`, `isync`
        // the next fetch of a stored-to instruction location sees the
        // new instruction. Fetch reads committed bytes, so the batch
        // ends before it fetches a word the buffer still holds, and the
        // next batch fetches what was stored. A translating thread's
        // fetch and its stores carry effective addresses, and two of
        // them can reach one real word [PPC-Book2 p:7 s:1.7], so it
        // ends before any fetch while the buffer holds a synced store.
        if cur
            .synced_stores
            .is_some_and(|span| cur.translating || self.fetches_buffered_store(span, step_pc))
        {
            return Some(self.budget_yield(entry, effects, cur.last_retired));
        }

        // A thread with supervisor registers takes an external or
        // decrementer interrupt ahead of the instruction.
        if cur.translating {
            if let Some(interrupt) = self.boundary_interrupt(ctx, entry, step_pc) {
                return Some(self.boundary_interrupt_yield(entry, effects, step_pc, interrupt));
            }
        }

        // Diagnostics are taken before the rollback so they carry
        // the registers at the break; the window's effects are
        // discarded like any other mid-window fault.
        if self.break_pc == Some(step_pc) {
            if self.break_skip > 0 {
                self.break_skip -= 1;
            } else if self.retirement_counter > entry.retired {
                // The retired instructions commit first; the next
                // batch stops here with nothing ahead of the break.
                return Some(self.budget_yield(entry, effects, cur.last_retired));
            } else {
                self.break_pc = None;
                return Some(self.fault_yield(entry, effects, step_pc, &PpuFault::DebugBreak));
            }
        }

        // CellGov runs supervisor code in 64-bit mode only. An MSR move
        // ends the batch, so a thread that clears SF stops at the first
        // instruction of its next one.
        if cur.translating && self.state.supervisor().is_some_and(|s| s.msr & MSR_SF == 0) {
            let boundary = (effects.len(), self.store_buf.len());
            let exit = FaultExit::Verdict(PpuFault::Unmodelled("32-bit mode in supervisor state"));
            let last = cur.last_retired;
            return Some(self.end_at_fault(entry, effects, step_pc, last, boundary, exit));
        }
        None
    }
}
