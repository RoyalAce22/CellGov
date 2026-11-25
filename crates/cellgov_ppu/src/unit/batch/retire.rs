//! The per-step bookkeeping of a batch:
//! - the start of the batch;
//! - each retirement, and the end check after it;
//! - the `sc` retirement and its LV2 yield;
//! - the profiles.

use super::clock::ctrl_for_placement;
use super::exit::Attempt;
use super::run::{BatchEntry, Cursor};
use crate::profile::LoadOutcome;
use crate::unit::ppu_unit::PpuExecutionUnit;
use crate::{instruction, state};
use cellgov_effects::Effect;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_time::Budget;

impl PpuExecutionUnit {
    /// Retires `attempt`, which ran with no fault, and returns the pc of
    /// the last instruction it retired.
    #[inline(always)]
    pub(super) fn retire_step(
        &mut self,
        ctx: &ExecutionContext<'_>,
        attempt: &Attempt,
        ops: &[crate::timing::Op],
        load: Option<(Option<u64>, LoadOutcome)>,
    ) -> u64 {
        let step_pc = attempt.step_pc;

        // The unit publishes a fetch once its instruction retires, so
        // a block that ends at a fault names no text it did not run.
        self.note_fetch(step_pc);
        self.timing.retire_instruction(step_pc, ops, self.state.pc);

        if self.profile_mode {
            self.profile_retirement(attempt, load);
        }

        if ctx.trace_per_step() {
            self.per_step_hashes
                .push((step_pc, self.state.state_hash()));
            if let Some(kind) = attempt.insn.barrier_kind() {
                self.barriers
                    .push(cellgov_exec::RetiredBarrier { pc: step_pc, kind });
            }
            // A traced batch runs no fused pair, so `insn` is one
            // instruction.
            self.note_code_events(&attempt.insn, step_pc, attempt.stores_before);
        }
        self.record_retirement(step_pc);
        let mut last_pc = step_pc;
        if attempt.pair {
            last_pc = step_pc + 4;
            self.record_retirement(last_pc);
        }
        last_pc
    }

    /// Records the retired `attempt` in the dispatch, operand and load
    /// profiles.
    #[inline(always)]
    fn profile_retirement(&mut self, attempt: &Attempt, load: Option<(Option<u64>, LoadOutcome)>) {
        self.profile_dispatch(&attempt.insn);
        self.operand_profile.record(attempt.step_pc);
        if attempt.pair {
            // Both halves count, so the profile sees the
            // instructions the shadow fused.
            self.operand_profile.record(attempt.step_pc + 4);
        }
        if let Some((slot, outcome)) = load {
            self.load_profile.record(slot, outcome);
        }
        if crate::unit::load_probe::has_unkeyed_load(&attempt.insn) {
            self.load_profile.unkeyed_fused += 1;
        }
    }

    /// Retires the `sc` at `step_pc` ahead of its yield.
    #[inline]
    pub(super) fn retire_syscall(
        &mut self,
        ctx: &ExecutionContext<'_>,
        step_pc: u64,
        ops: &[crate::timing::Op],
    ) {
        // `sc` retires at the yield like any other instruction
        // [PPC-Book1 p:26 s:2.4.2]: counted, hashed and charged.
        // Its records hold the state at the yield. For an LV2
        // call, the next batch applies the return value and the
        // step past it, and charges the call's round trip.
        self.note_fetch(step_pc);
        self.timing.retire_instruction(step_pc, ops, step_pc + 4);
        if self.profile_mode {
            self.operand_profile.record(step_pc);
        }
        if ctx.trace_per_step() {
            self.per_step_hashes
                .push((step_pc, self.state.state_hash()));
            self.note_syscall_code_events(step_pc);
        }
        self.record_retirement(step_pc);
    }

    /// Closes the block and yields the LV2 call of the retired `sc` at
    /// `step_pc`.
    #[inline]
    pub(super) fn syscall_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        step_pc: u64,
        lev: u8,
    ) -> ExecutionStepResult {
        self.close_block(effects);
        let args = state::ppu_syscall_args(&self.state);
        ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: self.cost(entry),
            local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(
                step_pc,
                self.state.lr(),
                lev,
            ),
            fault: None,
            syscall_args: Some(args),
        }
    }

    /// Applies the runtime's committed inputs and clears the clock-read flag.
    ///
    /// The inputs the runtime committed since the last batch:
    /// - the reservation clear
    /// - a syscall return
    /// - whether the unit left the PPE since its last step
    /// - register writes
    /// - the CR fields that the hypervisor's compares set
    /// - the PPE sibling count
    #[inline]
    fn apply_committed_inputs(&mut self, ctx: &ExecutionContext<'_>) {
        // Cross-unit reservation clear: committed table is authoritative.
        if self.state.reservation().is_some() && !ctx.reservation_held(self.id) {
            self.state.set_reservation(None);
        }

        // `mftb`, `mftbu` and a decrementer move set the flag, and each
        // step starts clear.
        self.state.clock_read = false;

        if let Some(code) = ctx.syscall_return() {
            self.state.set_gpr(3, code);
            self.state.pc += 4;
            self.timing.note_syscall_return(code);
        }
        if let Some(kind) = ctx.resumed() {
            let preempted = match kind {
                cellgov_exec::ResumeKind::Woken => false,
                cellgov_exec::ResumeKind::Preempted => true,
            };
            self.timing.note_syscall_resumed(preempted);
        }
        for &(reg, val) in ctx.register_writes() {
            if (reg as usize) < 32 {
                self.state.set_gpr(reg as usize, val);
            }
        }
        if let Some(fields) = ctx.compare_fields() {
            super::super::hypercall::take_compare_fields(&mut self.state, fields);
        }

        // A runnable sibling shares the PPE's issue for this batch.
        self.timing
            .set_siblings(ctx.ppe_siblings(), ctx.ppe_sibling_pressure());
        self.state.ctrl = match self.fixed_ctrl {
            Some(fixed) => fixed_ctrl_for(fixed, ctx.ppe_siblings()),
            None => ctrl_for_placement(ctx.ppe_siblings(), ctx.ppe_older_sibling()),
        };
    }

    /// Records one retirement at `step_pc`: the full-state window entry,
    /// then the retirement count.
    #[inline]
    fn record_retirement(&mut self, step_pc: u64) {
        if let Some((lo, hi)) = self.full_state_window {
            if self.retirement_counter >= lo && self.retirement_counter <= hi {
                self.per_step_full_states.push((
                    self.retirement_counter,
                    step_pc,
                    self.state.fingerprint(),
                ));
            }
        }
        debug_assert!(
            self.state.hash_is_current(),
            "state-hash accumulator out of date after retirement at 0x{step_pc:x}"
        );
        self.retirement_counter += 1;
    }

    /// Counts the dispatched variant, and the pair it forms with the one
    /// before, so quickenings and super-pairs show in the profile.
    #[inline]
    fn profile_dispatch(&mut self, insn: &instruction::PpuInstruction) {
        let name: &'static str = insn.into();
        *self.profile_insns.entry(name).or_insert(0) += 1;
        if let Some(prev) = self.profile_prev {
            *self.profile_pairs.entry((prev, name)).or_insert(0) += 1;
        }
        self.profile_prev = Some(name);
    }

    /// Opens a batch and returns the entry a fault inside it rewinds to.
    #[inline(always)]
    pub(super) fn begin_batch(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> BatchEntry {
        let max_budget = budget.raw();
        effects.clear();
        self.store_buf.clear();
        self.drop_fetch_runs();
        self.apply_committed_inputs(ctx);

        // Taken after the runtime's committed inputs (syscall return,
        // register writes) are applied: a mid-batch rollback must not
        // undo state the commit pipeline already owns.
        let entry = BatchEntry {
            snapshot: (max_budget > 1).then(|| self.state.clone()),
            hashes: self.per_step_hashes.len(),
            fulls: self.per_step_full_states.len(),
            barriers: self.barriers.len(),
            code_events: self.code_events.len(),
            retired: self.retirement_counter,
            cycle: self.timing.now(),
            timing: (max_budget > 1).then(|| self.timing.core()),
            trace: ctx.trace_per_step(),
        };
        // The round trip of the LV2 call the last batch yielded at counts
        // toward this batch, which runs after it returns.
        self.timing.charge_syscall();
        entry
    }

    /// Moves `cur` past `attempt`, which retired through `last_pc`, and
    /// returns whether the batch ends there.
    #[inline(always)]
    pub(super) fn ends_after(
        &self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        effects: &[Effect],
        cur: &mut Cursor<'_>,
        attempt: &Attempt,
        last_pc: u64,
    ) -> bool {
        cur.remaining = cur
            .remaining
            .saturating_sub(if attempt.pair { 2 } else { 1 });
        let reached_window = cur.window_span.is_some_and(|span| {
            self.reaches_window(span, ctx.memory(), effects, attempt, cur.translating)
        });
        cur.last_retired = last_pc;
        if matches!(attempt.insn, instruction::PpuInstruction::Isync) {
            cur.synced_stores = self.store_buf.span();
        }
        cur.remaining == 0
            || reached_window
            || (cur.translating
                && self.translation_ends_batch(&attempt.insn, attempt.stores_before))
            || self.leads_sibling(ctx, entry)
    }
}

/// The CTRL value that a thread with the fixed CTRL `fixed` reads beside
/// `siblings` runnable PPE threads.
///
/// With no runnable sibling, the other thread is in an lv1 pause, and lv1
/// suspends a paused thread. So TE holds only the reader's own thread. In
/// the `ppu_ctrl_threads` capture, a thread that runs alone reads only its
/// own TE bit.
///
/// [CBE-Handbook p:321 s:10.8.1] The only way software can suspend a thread is to execute the mtctrl instruction with an operand that leaves the thread's Thread Enable bit cleared to '0'.
fn fixed_ctrl_for(fixed: u32, siblings: u32) -> u32 {
    use cellgov_ps3_abi::hw::ppu_spr::{CTRL_CT_THREAD0, CTRL_TE_THREAD0, CTRL_TE_THREAD1};
    if siblings > 0 {
        return fixed;
    }
    let own = if fixed & CTRL_CT_THREAD0 != 0 {
        CTRL_TE_THREAD0
    } else {
        CTRL_TE_THREAD1
    };
    (fixed & !(CTRL_TE_THREAD0 | CTRL_TE_THREAD1)) | own
}
