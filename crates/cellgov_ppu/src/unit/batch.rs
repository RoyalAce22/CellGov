//! One batch: the fetch-decode-execute loop and its yields.

use super::ppu_unit::PpuExecutionUnit;
use crate::exec::{ExecuteVerdict, PpuFault, PpuInterrupt};
use crate::tap::PpuTap;
use crate::{decode, exec, instruction, state};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_ps3_abi::hw::ppu::PPU_TIME_BASE_ORIGIN;
use cellgov_time::{Budget, InstructionCost};

/// What a batch rewinds to when a step inside it faults.
///
/// `snapshot` is `None` for a single-step batch: no step retires ahead
/// of a fault there, so the rollback needs no state clone.
pub(super) struct BatchEntry {
    pub(super) snapshot: Option<state::PpuState>,
    pub(super) hashes: usize,
    pub(super) fulls: usize,
    pub(super) barriers: usize,
    pub(super) code_events: usize,
    pub(super) retired: u64,
    /// The issue model's cycle at entry.
    pub(super) cycle: u64,
    /// The issue model's scalar state at entry, taken when a step can
    /// retire ahead of a fault.
    pub(super) timing: Option<crate::timing::Core>,
}

/// How an instruction faulted.
enum FaultExit {
    /// A fault the fetch, the decode or the instruction reported.
    Verdict(PpuFault),
    /// A memory fault the instruction reported, and whether its access
    /// was a store.
    Mem(cellgov_mem::MemError, bool),
}

/// The spacing, in core cycles, of the grid of a PPE thread's own clock
/// at which its batch ends while a PPE sibling is runnable.
///
/// In the `ppu_issue_mix_switch` capture a sibling turns from dependent
/// `fdiv` to independent `addi` between two halves of a timed loop. The
/// console's second half costs what it costs beside a steady `addi`
/// sibling. A replay at one instruction a step times that half near the
/// console. A replay at the runtime's 256-instruction step adds to it:
/// - with no grid, 2.6 cycles an iteration, outside the console's spread;
/// - with a grid of 1024 cycles, 0.25;
/// - with this grid, 0.08.
pub(super) const SIBLING_LEAD_CYCLES: u64 = 256;

/// The tap a unit with none installed runs its batch loop with.
pub(super) struct NoTap;

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

impl PpuExecutionUnit {
    /// Guest ticks for the instructions this batch retired: the core
    /// cycles the issue model advanced.
    ///
    /// A tick is one core cycle. The second instruction of a pair that
    /// issues with the first costs nothing.
    fn cost(&self, entry: &BatchEntry) -> InstructionCost {
        InstructionCost::new(self.timing.now() - entry.cycle)
    }

    /// Run one batch and report each dispatch to `tap`.
    ///
    /// The compiler builds one copy per tap type, so the [`NoTap`] copy
    /// has no call in its per-instruction loop.
    pub(super) fn run_batch<T: PpuTap + ?Sized>(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
        tap: &T,
    ) -> ExecutionStepResult {
        let max_budget = budget.raw();
        let mut remaining = max_budget;
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
        };
        // The round trip of the LV2 call the last batch yielded at counts
        // toward this batch, which runs after it returns.
        self.timing.charge_syscall();

        let regions = RegionTable::new(ctx.memory().region_views());
        let region_views = regions.as_slice();
        // A window's bytes are another unit's state, which the runtime
        // updates only between batches. The batch ends at the instruction
        // that reaches a window, so the next instruction sees the update.
        let window_span = ctx.memory().window_span();
        if self.profile_mode {
            self.load_profile
                .note_layout(region_views.iter().map(|v| (v.base, v.bytes.len() as u64)));
        }

        // The pc of the last instruction the batch retired.
        let mut last_retired = self.state.pc;
        // A fused pair that faulted runs again split, at this pc.
        let mut force_split = None;
        // The span of the buffered stores when the last `isync` retired.
        let mut synced_stores: Option<(u64, u64)> = None;
        // The fetchable span the last fetch read: its first byte and the
        // last word that starts in it. It starts empty.
        let mut fetchable = (1, 0);
        // The base and bytes of the region that holds `fetchable`.
        let mut code: CodeBytes<'_> = (0, &[]);

        loop {
            let step_pc = self.state.pc;
            let forced = force_split.take() == Some(step_pc);

            // [PPC-Book2 p:12 s:1.8] after `dcbst`, `sync`, `icbi`, `isync`
            // the next fetch of a stored-to instruction location sees the
            // new instruction. Fetch reads committed bytes, so the batch
            // ends before it fetches a word the buffer still holds, and the
            // next batch fetches what was stored.
            if synced_stores.is_some_and(|span| self.fetches_buffered_store(span, step_pc)) {
                return self.budget_yield(&entry, effects, last_retired);
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
                    return self.budget_yield(&entry, effects, last_retired);
                } else {
                    self.break_pc = None;
                    return self.fault_yield(&entry, effects, step_pc, &PpuFault::DebugBreak);
                }
            }

            let boundary = (effects.len(), self.store_buf.len());
            let fetched = if holds_word(fetchable, step_pc) {
                Ok(())
            } else {
                fetch_window(ctx.memory(), step_pc).map(|(window, bytes)| {
                    fetchable = window;
                    code = bytes;
                })
            };
            let mut insn = match fetched.and_then(|()| self.fetch_decode(step_pc, code)) {
                Ok(insn) => insn,
                Err(fault) => {
                    let exit = FaultExit::Verdict(fault);
                    return self.end_at_fault(
                        &entry,
                        effects,
                        step_pc,
                        last_retired,
                        boundary,
                        exit,
                    );
                }
            };

            // A fused head retires both of its slots. The loop reaches a
            // `Consumed` slot only by a branch to it or after a split head.
            // The instruction stored there then runs [PPC-Book1 p:17 s:2.2].
            // A head splits and runs as its first instruction alone when:
            // - the budget cannot retire both slots,
            // - a break is set on the second slot,
            // - per-step tracing is on,
            // - a full-state window is set,
            // - the second slot is not fetchable, or
            // - a Raw SPU window is mapped and the second half reads memory
            //   after the first half accessed it.
            // The trace and the full-state window record the state after each
            // instruction, and a fused head has no state between its slots.
            // A Raw SPU window changes only between batches, and the batch
            // ends at the first half that reaches it.
            // A head whose second half refused fused also runs split, so
            // its first instruction retires before the refusal.
            let split = match insn {
                instruction::PpuInstruction::Consumed => true,
                _ if insn.is_super_pair() => {
                    forced
                        || remaining < 2
                        || !holds_word(fetchable, step_pc.wrapping_add(4))
                        || self.break_pc == Some(step_pc + 4)
                        || ctx.trace_per_step()
                        || self.full_state_window.is_some()
                        || (window_span.is_some() && insn.reads_after_access())
                }
                _ => false,
            };
            if split {
                insn = match decode_stored(step_pc, code) {
                    Ok((_, insn)) => insn,
                    Err(fault) => {
                        let exit = FaultExit::Verdict(fault);
                        return self.end_at_fault(
                            &entry,
                            effects,
                            step_pc,
                            last_retired,
                            boundary,
                            exit,
                        );
                    }
                };
            }
            let pair = insn.is_super_pair();

            tap.dispatch(self.id, &insn, &self.state);
            let (ops, n_ops) = crate::timing::classify(&insn, &self.state);
            let ops = &ops[..n_ops];
            self.align_clock_read_to_issue(ctx, &entry, step_pc, &insn, ops);
            let (effects_before, stores_before) = (effects.len(), self.store_buf.len());
            // A pair that refuses runs again split, and
            // `undo_fused_attempt` takes back its reads.
            let reads_before = if pair {
                ctx.memory().provisional_read_count()
            } else {
                0
            };
            let load = if self.profile_mode {
                self.probe_load(&insn, step_pc, region_views)
            } else {
                None
            };
            let fault = match exec::execute(
                &insn,
                &mut self.state,
                self.id,
                region_views,
                effects,
                &mut self.store_buf,
            ) {
                ExecuteVerdict::Continue => {
                    self.state.pc += if pair { 8 } else { 4 };
                    None
                }
                ExecuteVerdict::Branch => None,
                ExecuteVerdict::Syscall { lev } => {
                    // `sc` retires at the yield like any other instruction
                    // [PPC-Book1 p:26 s:2.4.2]: counted, hashed and charged.
                    // Its records hold the state at the yield. The next
                    // batch applies the return value and the step past it,
                    // and charges the call's round trip.
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
                    self.close_block(effects);
                    let args = state::ppu_syscall_args(&self.state);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Syscall,
                        consumed_cost: self.cost(&entry),
                        local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(
                            step_pc,
                            self.state.lr(),
                            lev,
                        ),
                        fault: None,
                        syscall_args: Some(args),
                    };
                }
                ExecuteVerdict::Fault(f) => Some(FaultExit::Verdict(f)),
                ExecuteVerdict::MemFault(e) => Some(FaultExit::Mem(e, false)),
                ExecuteVerdict::StoreFault(e) => Some(FaultExit::Mem(e, true)),
                ExecuteVerdict::BufferFull if pair => {
                    // The second half found no room. Run split, so the
                    // first half retires and the second waits at its own
                    // address, as the unfused store does.
                    let boundary = (effects_before, stores_before, reads_before);
                    self.undo_fused_attempt(ctx.memory(), effects, boundary, step_pc);
                    force_split = Some(step_pc);
                    continue;
                }
                ExecuteVerdict::BufferFull => {
                    // PC stays at the failing store; retries next step.
                    // Break skips count retirements, and nothing retired,
                    // so a skip spent at the top of this iteration is
                    // handed back.
                    if self.break_pc == Some(step_pc) {
                        self.break_skip += 1;
                    }
                    self.note_fetch(step_pc);
                    return self.budget_yield(&entry, effects, step_pc);
                }
            };
            if let Some(exit) = fault {
                if pair {
                    let boundary = (effects_before, stores_before, reads_before);
                    self.undo_fused_attempt(ctx.memory(), effects, boundary, step_pc);
                    force_split = Some(step_pc);
                    continue;
                }
                let boundary = (effects_before, stores_before);
                return self.end_at_fault(&entry, effects, step_pc, last_retired, boundary, exit);
            }

            // The unit publishes a fetch once its instruction retires, so
            // a block that ends at a fault names no text it did not run.
            self.note_fetch(step_pc);
            self.timing.retire_instruction(step_pc, ops, self.state.pc);

            if self.profile_mode {
                self.profile_dispatch(&insn);
                self.operand_profile.record(step_pc);
                if pair {
                    // Both halves count, so the profile sees the
                    // instructions the shadow fused.
                    self.operand_profile.record(step_pc + 4);
                }
                if let Some((slot, outcome)) = load {
                    self.load_profile.record(slot, outcome);
                }
                if super::load_probe::has_unkeyed_load(&insn) {
                    self.load_profile.unkeyed_fused += 1;
                }
            }

            if ctx.trace_per_step() {
                self.per_step_hashes
                    .push((step_pc, self.state.state_hash()));
                if let Some(kind) = insn.barrier_kind() {
                    self.barriers
                        .push(cellgov_exec::RetiredBarrier { pc: step_pc, kind });
                }
                // A traced batch runs no fused pair, so `insn` is one
                // instruction.
                self.note_code_events(&insn, step_pc, stores_before);
            }
            self.record_retirement(step_pc);
            let mut last_pc = step_pc;
            if pair {
                last_pc = step_pc + 4;
                self.record_retirement(last_pc);
            }

            remaining = remaining.saturating_sub(if pair { 2 } else { 1 });
            let reached_window = window_span.is_some_and(|span| {
                touches_window(
                    span,
                    ctx.memory(),
                    effects.get(effects_before..).unwrap_or_default(),
                    self.store_buf.stores_from(stores_before),
                )
            });
            last_retired = last_pc;
            if matches!(insn, instruction::PpuInstruction::Isync) {
                synced_stores = self.store_buf.span();
            }
            if remaining == 0 || reached_window || self.leads_sibling(ctx, &entry) {
                return self.budget_yield(&entry, effects, last_pc);
            }
        }
    }

    /// Whether a PPE sibling is runnable and the batch reaches a multiple
    /// of [`SIBLING_LEAD_CYCLES`] on the thread's own clock.
    ///
    /// A sibling's issue pressure and its view of memory hold as of its
    /// own clock. A thread that runs past that clock has two errors:
    /// - it charges the sibling's old instruction mix;
    /// - the sibling sees this thread's stores only at its next step.
    ///
    /// The runtime's default scheduler runs the thread with the lowest
    /// clock, so the threads stay within about one grid step of each
    /// other. A scheduler that prescribes the order, as the explorer's
    /// does, can let one thread lead by more, at a cost in timing accuracy
    /// only. The rule reads the thread's own clock and the sibling count,
    /// and no sibling's clock. Two steps that the dependency relation calls
    /// independent must commute [FlanaganGodefroid2005 p:3 s:2.2]. The
    /// relation records the sibling count, and no footprint records a
    /// clock of another thread.
    #[inline]
    fn leads_sibling(&self, ctx: &ExecutionContext<'_>, entry: &BatchEntry) -> bool {
        if ctx.ppe_siblings() == 0 {
            return false;
        }
        let into_line = ctx.current_tick().raw() % SIBLING_LEAD_CYCLES;
        into_line.saturating_add(self.timing.now() - entry.cycle) >= SIBLING_LEAD_CYCLES
    }

    /// Drops what a fused pair at `step_pc` staged before its second half
    /// refused, so the pair can run again split.
    ///
    /// The first half may apply before the second refuses. It reruns alone
    /// and leaves the same state. A first half that can write a register
    /// it reads is one of these:
    ///
    /// - a load (`lwz rT,d(rT)`), which refuses before it writes
    /// - the `or` of `or; lwz` (`or rA,rA,rB`)
    ///
    /// These fused pairs write no register until every load passes:
    ///
    /// - `ldu; ldu`
    /// - `lwz; lwz`
    /// - `ld; ld` outside the same-base pair
    /// - `lbz; lbz`
    /// - `or; lwz`
    ///
    /// It takes back the provisional reads the pair logged, down to the
    /// count `reads`, so the split run logs each of its reads once.
    fn undo_fused_attempt(
        &mut self,
        mem: &cellgov_mem::GuestMemory,
        effects: &mut Vec<Effect>,
        (effects_len, stores_len, reads): (usize, usize, u64),
        step_pc: u64,
    ) {
        effects.truncate(effects_len);
        self.store_buf.truncate(stores_len);
        mem.rewind_provisional_reads(reads);
        // The split run visits this pc again, so the unit hands back a
        // skip it spent at the top of this iteration.
        if self.break_pc == Some(step_pc) {
            self.break_skip += 1;
        }
    }

    /// Ends the batch at the instruction at `step_pc`, which faulted.
    ///
    /// When an earlier instruction of the batch retired, the batch ends
    /// as an exhausted budget does:
    /// - the retired instructions commit;
    /// - the unit drops the effects from `effects_len` on and the staged
    ///   stores from `stores_len` on;
    /// - PC stays on the faulting instruction.
    ///
    /// The next batch runs that instruction with no retired instruction
    /// ahead of it. Its fault then discards no instruction that completed
    /// before it [PPC-Book3 p:61 s:5.2].
    fn end_at_fault(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        step_pc: u64,
        last_retired: u64,
        (effects_len, stores_len): (usize, usize),
        exit: FaultExit,
    ) -> ExecutionStepResult {
        if self.retirement_counter > entry.retired {
            effects.truncate(effects_len);
            self.store_buf.truncate(stores_len);
            // The next batch visits this pc again, so the unit hands back
            // a skip it spent at the top of this iteration.
            if self.break_pc == Some(step_pc) {
                self.break_skip += 1;
            }
            return self.budget_yield(entry, effects, last_retired);
        }
        match exit {
            FaultExit::Verdict(f) => self.fault_yield(entry, effects, step_pc, &f),
            FaultExit::Mem(e, store) => self.mem_fault_yield(entry, effects, step_pc, e, store),
        }
    }

    /// Applies the runtime's committed inputs and clears the clock-read flag.
    ///
    /// The inputs the runtime committed since the last batch:
    /// - the reservation clear
    /// - a syscall return
    /// - register writes
    /// - the PPE sibling count
    #[inline]
    fn apply_committed_inputs(&mut self, ctx: &ExecutionContext<'_>) {
        // Cross-unit reservation clear: committed table is authoritative.
        if self.state.reservation().is_some() && !ctx.reservation_held(self.id) {
            self.state.set_reservation(None);
        }

        // `mftb` and `mftbu` set the flag, and each step starts clear.
        self.state.clock_read = false;

        if let Some(code) = ctx.syscall_return() {
            self.state.set_gpr(3, code);
            self.state.pc += 4;
            self.timing.note_syscall_return(code);
        }
        for &(reg, val) in ctx.register_writes() {
            if (reg as usize) < 32 {
                self.state.set_gpr(reg as usize, val);
            }
        }

        // A runnable sibling shares the PPE's issue for this batch.
        self.timing
            .set_siblings(ctx.ppe_siblings(), ctx.ppe_sibling_pressure());
        self.state.ctrl = ctrl_for_placement(ctx.ppe_siblings(), ctx.ppe_older_sibling());
    }

    /// The instruction at `step_pc`: the shadow's copy, or a fresh decode
    /// that refreshes the shadow.
    #[inline]
    fn fetch_decode(
        &mut self,
        step_pc: u64,
        code: CodeBytes<'_>,
    ) -> Result<instruction::PpuInstruction, PpuFault> {
        if let Some(cached) = self
            .instruction_shadow
            .as_ref()
            .and_then(|s| s.get(step_pc))
        {
            self.shadow_hits += 1;
            return Ok(cached);
        }
        self.shadow_misses += 1;
        let (raw, insn) = decode_stored(step_pc, code)?;
        if let Some(s) = self.instruction_shadow.as_mut() {
            let _ = s.refresh(step_pc, raw);
        }
        Ok(insn)
    }

    /// Gives an `mftb` / `mftbu` the time base at its issue cycle.
    ///
    /// The run has one time base: a read returns no less than any read
    /// before it, on this unit or another.
    #[inline]
    fn align_clock_read_to_issue(
        &mut self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        step_pc: u64,
        insn: &instruction::PpuInstruction,
        ops: &[crate::timing::Op],
    ) {
        let (
            instruction::PpuInstruction::Mftb { .. } | instruction::PpuInstruction::Mftbu { .. },
            [op],
        ) = (insn, ops)
        else {
            return;
        };
        let issue = self.timing.issue_cycle(step_pc, op);
        let tick = ctx
            .current_tick()
            .raw()
            .saturating_add(issue.saturating_sub(entry.cycle));
        let tb = PPU_TIME_BASE_ORIGIN.wrapping_add(cellgov_time::ticks_to_tb(tick));
        self.state.tb = self.state.tb.max(tb).max(ctx.time_base());
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

    /// Closes the block and yields with the budget spent at `pc`.
    #[inline]
    fn budget_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
    ) -> ExecutionStepResult {
        self.close_block(effects);
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: self.cost(entry),
            local_diagnostics: LocalDiagnostics::with_pc(pc),
            fault: None,
            syscall_args: None,
        }
    }

    /// Whether the fetch at `pc` reads a word a buffered store covers.
    ///
    /// `span` bounds the stores, so a fetch outside it costs two compares.
    /// The checked range is 8 bytes, both slots a fused head reads.
    #[inline]
    fn fetches_buffered_store(&self, (lo, hi): (u64, u64), pc: u64) -> bool {
        let end = pc.saturating_add(8);
        pc < hi && end > lo && self.store_buf.has_store_in_range(pc, end)
    }

    /// Yields the data storage interrupt for the memory fault an
    /// instruction reported, then counts it.
    #[inline]
    fn mem_fault_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        step_pc: u64,
        e: cellgov_mem::MemError,
        store: bool,
    ) -> ExecutionStepResult {
        let (refused_by, unmapped) = match &e {
            cellgov_mem::MemError::Unmapped(_) => (None, true),
            cellgov_mem::MemError::ReservedWrite { region, .. } => (Some(*region), false),
            _ => (None, false),
        };
        let fault = match PpuInterrupt::data_storage(&e, store) {
            Some(interrupt) => PpuFault::Interrupt(interrupt),
            None => {
                // A load or store refusal is `Unmapped` or `ReservedWrite`.
                debug_assert!(false, "a memory fault carrying {e:?}");
                PpuFault::Internal("memory fault without an address")
            }
        };
        let mut result = self.fault_yield(entry, effects, step_pc, &fault);
        if let Some(access) = result.local_diagnostics.data_access.as_mut() {
            access.refused_by = refused_by;
        }
        // Incremented after the rollback: the counters are
        // hash-excluded instruments, and restoring the entry
        // snapshot must not erase the record of this fault.
        self.state.mem_fault_arm_entries = self.state.mem_fault_arm_entries.wrapping_add(1);
        if unmapped {
            self.state.mem_fault_unmapped_routed =
                self.state.mem_fault_unmapped_routed.wrapping_add(1);
        }
        result
    }
}

/// The CTRL value a thread reads, from the number of PPE siblings that
/// run beside it and whether one of them was created first.
///
/// In the `ppu_ctrl_threads` capture, TE enables both hardware threads
/// while a sibling runs, and only the reader's own when it runs alone.
/// CT moves with the reader between the hardware threads. CellGov gives
/// hardware thread 1 to the thread created first, and thread 0 to the
/// other one.
#[inline]
fn ctrl_for_placement(siblings: u32, older_sibling: bool) -> u32 {
    use cellgov_ps3_abi::hw::ppu_spr::{
        CTRL_CT_THREAD0, CTRL_CT_THREAD1, CTRL_TE_THREAD0, CTRL_TE_THREAD1, PS3_PROCESS_CTRL,
    };
    match (siblings, older_sibling) {
        (0, _) => PS3_PROCESS_CTRL,
        (_, false) => CTRL_CT_THREAD1 | CTRL_TE_THREAD0 | CTRL_TE_THREAD1,
        (_, true) => CTRL_CT_THREAD0 | CTRL_TE_THREAD0 | CTRL_TE_THREAD1,
    }
}

/// Whether the word at `pc` lies in the window `(start, last)`, where
/// `last` is the last word that starts in the span.
#[inline]
fn holds_word((start, last): (u64, u64), pc: u64) -> bool {
    start <= pc && pc <= last
}

/// The base guest address of a region and its bytes.
type CodeBytes<'a> = (u64, &'a [u8]);

/// The fetchable span that holds the word at `pc`, and its region.
///
/// The span is its first byte and the last word that starts in it.
///
/// # Errors
///
/// Returns `PpuInterrupt::InstructionStorage` if a byte of the word is
/// not fetchable. The interrupt sets `no_execute` if that byte is mapped.
fn fetch_window(
    mem: &cellgov_mem::GuestMemory,
    pc: u64,
) -> Result<((u64, u64), CodeBytes<'_>), PpuFault> {
    use cellgov_mem::FetchRefusal;
    let refused = |r: FetchRefusal| {
        PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: r == FetchRefusal::NoExecute,
        })
    };
    let code = mem.fetch_code(pc).map_err(refused)?;
    let (start, end) = code.span;
    match pc.checked_add(4) {
        Some(word_end) if word_end <= end => Ok(((start, end - 4), (code.base, code.bytes))),
        _ => Err(refused(
            mem.fetch_span(end).err().unwrap_or(FetchRefusal::Unmapped),
        )),
    }
}

/// The word stored at `pc` in a region, and its decode, without the shadow.
///
/// # Errors
///
/// - A fetch outside the region: the instruction storage interrupt.
/// - A word with no decode: the illegal-instruction program interrupt.
#[inline]
fn decode_stored(
    pc: u64,
    (base, mem): CodeBytes<'_>,
) -> Result<(u32, instruction::PpuInstruction), PpuFault> {
    let bytes = pc
        .checked_sub(base)
        .and_then(|off| usize::try_from(off).ok())
        .and_then(|pc| mem.get(pc..pc.checked_add(4)?))
        .ok_or(PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: false,
        }))?;
    let raw = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let insn = decode::decode(raw).map_err(|_| PpuFault::unrecognized_word())?;
    Ok((raw, insn))
}

/// The batch's region views, on the stack while they fit.
///
/// The stack table avoids a heap allocation on the Budget=1 hot path.
/// Boot installs six regions, each shared-memory mapping adds one and
/// each window adds up to two; a larger layout spills to the heap.
/// Nothing guest-visible turns on the cutoff.
struct RegionTable<'a> {
    storage: [cellgov_mem::RegionView<'a>; MAX_REGIONS],
    spill: Vec<cellgov_mem::RegionView<'a>>,
    len: usize,
}

/// How many region views [`RegionTable`] holds before it spills.
const MAX_REGIONS: usize = 8;

impl<'a> RegionTable<'a> {
    #[inline]
    fn new(views: impl Iterator<Item = cellgov_mem::RegionView<'a>>) -> Self {
        let mut table = Self {
            storage: [cellgov_mem::RegionView::plain(0, &[]); MAX_REGIONS],
            spill: Vec::new(),
            len: 0,
        };
        for view in views {
            if table.len < MAX_REGIONS {
                table.storage[table.len] = view;
            } else {
                if table.spill.is_empty() {
                    table.spill.extend_from_slice(&table.storage);
                }
                table.spill.push(view);
            }
            table.len += 1;
        }
        table
    }

    #[inline]
    fn as_slice(&self) -> &[cellgov_mem::RegionView<'a>] {
        if self.len <= MAX_REGIONS {
            &self.storage[..self.len]
        } else {
            &self.spill
        }
    }
}

/// Whether one instruction's new read intents or staged stores reach a
/// window of `mem`.
///
/// `[start, end)` holds every window, so the function reads the window
/// list only for an access inside it.
fn touches_window(
    (start, end): (u64, u64),
    mem: &cellgov_mem::GuestMemory,
    new_effects: &[Effect],
    mut new_stores: impl Iterator<Item = (u64, u8)>,
) -> bool {
    let overlaps = |addr: u64, len: u64| {
        let last = addr.saturating_add(len);
        addr < end && last > start && mem.window_ranges().any(|(lo, hi)| addr < hi && last > lo)
    };
    new_effects.iter().any(|e| match e {
        Effect::SharedReadIntent { range, .. } => overlaps(range.start().raw(), range.length()),
        _ => false,
    }) || new_stores.any(|(addr, len)| overlaps(addr, u64::from(len)))
}
