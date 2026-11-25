//! One batch: the fetch-decode-execute loop and its yields.

use super::ppu_unit::PpuExecutionUnit;
use crate::exec::{ExecuteVerdict, PpuFault, PpuInterrupt};
use crate::mmu::Access;
use crate::tap::PpuTap;
use crate::{decode, exec, instruction, state};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_ps3_abi::hw::ppu::PPU_TIME_BASE_ORIGIN;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
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
    /// Whether the batch records code events.
    pub(super) trace: bool,
}

/// What a translating thread does after an access missed every view.
enum MissStep {
    /// The instruction stops with this fault.
    Stop(FaultExit),
    /// The batch ends before the instruction, which the next batch runs
    /// again.
    EndBatch,
    /// The instruction runs again against the rebuilt views.
    Rerun,
}

/// How an instruction faulted.
pub(super) enum FaultExit {
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
    pub(super) fn cost(&self, entry: &BatchEntry) -> InstructionCost {
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
            trace: ctx.trace_per_step(),
        };
        // The round trip of the LV2 call the last batch yielded at counts
        // toward this batch, which runs after it returns.
        self.timing.charge_syscall();

        // A thread that translates runs against views of its translations
        // at their effective addresses. It decodes each fetch fresh: the
        // shadow and the fused pairs key on addresses its translation can
        // move.
        let translating = self.translates();
        let mut regions = self.batch_views(ctx.memory(), translating);
        // A window's bytes are another unit's state, which the runtime
        // updates only between batches. The batch ends at the instruction
        // that reaches a window, so the next instruction sees the update.
        let window_span = ctx.memory().window_span();
        if self.profile_mode {
            self.load_profile.note_layout(
                regions
                    .as_slice()
                    .iter()
                    .map(|v| (v.base, v.bytes.len() as u64)),
            );
        }

        // The pc of the last instruction the batch retired.
        let mut last_retired = self.state.pc;
        // A fused pair that faulted runs again split, at this pc.
        let mut force_split = None;
        // An instruction that runs again after a translation miss, at
        // this pc.
        let mut rerun: Option<u64> = None;
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
            // next batch fetches what was stored. A translating thread's
            // fetch and its stores carry effective addresses, and two of
            // them can reach one real word [PPC-Book2 p:7 s:1.7], so it
            // ends before any fetch while the buffer holds a synced store.
            if synced_stores
                .is_some_and(|span| translating || self.fetches_buffered_store(span, step_pc))
            {
                return self.budget_yield(&entry, effects, last_retired);
            }

            // A thread with supervisor registers takes an external or
            // decrementer interrupt ahead of the instruction.
            if translating {
                if let Some(interrupt) = self.boundary_interrupt(ctx, &entry, step_pc) {
                    return self.boundary_interrupt_yield(&entry, effects, step_pc, interrupt);
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
                    return self.budget_yield(&entry, effects, last_retired);
                } else {
                    self.break_pc = None;
                    return self.fault_yield(&entry, effects, step_pc, &PpuFault::DebugBreak);
                }
            }

            let boundary = (effects.len(), self.store_buf.len());
            // CellGov runs supervisor code in 64-bit mode only. An MSR move
            // ends the batch, so a thread that clears SF stops at the first
            // instruction of its next one.
            if translating && self.state.supervisor().is_some_and(|s| s.msr & MSR_SF == 0) {
                let exit =
                    FaultExit::Verdict(PpuFault::Unmodelled("32-bit mode in supervisor state"));
                return self.end_at_fault(&entry, effects, step_pc, last_retired, boundary, exit);
            }
            let fetched = if holds_word(fetchable, step_pc) {
                Ok(())
            } else {
                let window = if translating {
                    self.translated_fetch(ctx.memory(), step_pc)
                } else {
                    fetch_window(ctx.memory(), step_pc)
                };
                window.map(|(window, bytes)| {
                    fetchable = window;
                    code = bytes;
                })
            };
            let decoded = fetched.and_then(|()| {
                if translating {
                    decode_stored(step_pc, code).map(|(_, insn)| insn)
                } else {
                    self.fetch_decode(step_pc, code)
                }
            });
            let mut insn = match decoded {
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
            // - the second slot is not fetchable,
            // - a Raw SPU window is mapped and the second half reads memory
            //   after the first half accessed it, or
            // - a PPE sibling is runnable.
            // The trace and the full-state window record the state after each
            // instruction, and a fused head has no state between its slots.
            // A Raw SPU window changes only between batches, and the batch
            // ends at the first half that reaches it. Beside a sibling the
            // batch ends at the first instruction that reaches a grid line
            // (`leads_sibling`), and that can be the first half.
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
                        || ctx.ppe_siblings() > 0
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

            // A rerun after a translation miss is the same dispatch.
            if rerun.take() != Some(step_pc) {
                tap.dispatch(self.id, &insn, &self.state);
            }
            let (mut ops, n_ops) = crate::timing::classify(&insn, &self.state);
            if translating {
                self.real_op_addresses(&mut ops[..n_ops]);
            }
            let ops = &ops[..n_ops];
            self.align_clock_read_to_issue(ctx, &entry, step_pc, &insn, ops);
            let (effects_before, stores_before) = (effects.len(), self.store_buf.len());
            // A pair that refuses runs again split, and
            // `undo_fused_attempt` takes back its reads. An instruction
            // that runs again after a translation miss takes back its
            // reads the same way.
            let reads_before = if pair || translating {
                ctx.memory().provisional_read_count()
            } else {
                0
            };
            let load = if self.profile_mode {
                self.probe_load(&insn, step_pc, regions.as_slice())
            } else {
                None
            };
            // An `lwarx` or `ldarx` counts its execution before its access,
            // so an attempt that misses in the translation cache or faults
            // gives its count back before the instruction runs again.
            let witnesses =
                translating.then_some((self.state.lwarx_executed, self.state.ldarx_executed));
            let mut fault = match exec::execute(
                &insn,
                &mut self.state,
                self.id,
                regions.as_slice(),
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
                    // A thread with supervisor registers takes the system
                    // call interrupt at the kernel's vector instead, or
                    // yields a hypercall.
                    if translating {
                        return self.supervisor_sc_yield(&entry, effects, step_pc, lev);
                    }
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
            // A translating thread's access that no view holds is a miss:
            // the unit translates the address, and the instruction runs
            // again against the new views. A translation that refuses
            // the access is the instruction's interrupt. A fault at an
            // address the cache translates already is no miss. The views
            // hold each piece of storage the cached translations reach. So
            // a held address refuses as unmapped only for an access that
            // wraps the effective space. An address no view holds then has
            // no storage at its real address. A hit and a miss of the
            // cache then give one stop.
            if let (true, Some(FaultExit::Mem(cellgov_mem::MemError::Unmapped(miss), store))) =
                (translating, &fault)
            {
                let access = if *store { Access::Store } else { Access::Load };
                let attempt = (effects_before, stores_before);
                let mem = ctx.memory();
                match self.translation_miss(
                    mem,
                    &insn,
                    &mut regions,
                    effects,
                    attempt,
                    (miss.addr, access),
                ) {
                    MissStep::Stop(exit) => fault = Some(exit),
                    // The next batch runs the instruction again and logs
                    // its reads then.
                    MissStep::EndBatch => {
                        self.retract_missed_attempt(mem, step_pc, reads_before, witnesses);
                        return self.budget_yield(&entry, effects, last_retired);
                    }
                    // The rerun logs its own reads, as a run that hits the
                    // cache logs them once.
                    MissStep::Rerun => {
                        self.retract_missed_attempt(mem, step_pc, reads_before, witnesses);
                        rerun = Some(step_pc);
                        continue;
                    }
                }
            }
            if let Some(exit) = fault {
                // The instruction did not retire, and it runs again: in the
                // next batch, or after the kernel's handler returns. So its
                // `lwarx` or `ldarx` count goes back, as after a miss.
                // [PPC-Book3 p:62 s:5.3.1] Depending on the interrupt type, the instruction that causes a precise interrupt may appear not to have begun execution.
                self.restore_witnesses(witnesses);
                if pair {
                    let boundary = (effects_before, stores_before, reads_before);
                    self.undo_fused_attempt(ctx.memory(), effects, boundary, step_pc);
                    force_split = Some(step_pc);
                    continue;
                }
                let boundary = (effects_before, stores_before);
                return self.end_at_fault(&entry, effects, step_pc, last_retired, boundary, exit);
            }

            if translating {
                self.real_effects(effects, effects_before, Access::Load);
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
                let new_effects = effects.get(effects_before..).unwrap_or_default();
                let stores = self.store_buf.stores_from(stores_before);
                if translating {
                    let real = stores.flat_map(|(ea, len)| self.real_store_pieces(ea, len));
                    touches_window(span, ctx.memory(), new_effects, real)
                } else {
                    touches_window(span, ctx.memory(), new_effects, stores)
                }
            });
            last_retired = last_pc;
            if matches!(insn, instruction::PpuInstruction::Isync) {
                synced_stores = self.store_buf.span();
            }
            if remaining == 0
                || reached_window
                || (translating && self.translation_ends_batch(&insn, stores_before))
                || self.leads_sibling(ctx, &entry)
            {
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

    /// The views a batch runs against: a translating thread's
    /// translations at their effective addresses, after it follows its
    /// translation registers, or else the regions of `mem`.
    #[inline]
    fn batch_views<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        translating: bool,
    ) -> RegionTable<'m> {
        if !translating {
            return RegionTable::new(mem.region_views());
        }
        self.follow_translation();
        RegionTable::new(self.translated_views(mem).into_iter())
    }

    /// Whether a translating thread's batch ends after `insn`, which
    /// retired with the store buffer grown from `stores_before`.
    #[inline]
    fn translation_ends_batch(
        &self,
        insn: &instruction::PpuInstruction,
        stores_before: usize,
    ) -> bool {
        // A move of the MSR, the SLB or the partition ends the batch: the
        // next one runs against the translations it gives, and this one's
        // buffered stores publish under the old ones.
        let retranslates = matches!(insn, instruction::PpuInstruction::PpePrivileged { .. })
            && self.translation_moved();
        // A store to real bytes that a second cached translation also
        // reaches ends the batch, so no later load reads them through the
        // other alias while the store sits in the buffer.
        let aliased =
            self.store_buf.len() > stores_before && self.buffered_store_aliased(stores_before);
        // [PPC-Book3 p:59 s:4.12] `ptesync` orders the page table stores before it ahead of the next table search.
        // A search reads committed memory, so the batch ends and the next
        // one searches after the stores commit.
        let table_synced = matches!(insn, instruction::PpuInstruction::Sync { l: 2 });
        retranslates || aliased || table_synced
    }

    /// What a translating thread's access at `ea` that no view held does
    /// next: translate it and run again, end the batch, or stop.
    ///
    /// A miss truncates the attempt's effects and buffered stores to
    /// `attempt` and refills the cache. A rerun finds `regions` rebuilt
    /// from the new translations.
    #[inline]
    fn translation_miss<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        insn: &instruction::PpuInstruction,
        regions: &mut RegionTable<'m>,
        effects: &mut Vec<Effect>,
        (effects_before, stores_before): (usize, usize),
        (ea, access): (u64, Access),
    ) -> MissStep {
        let refilled = if self.translations.lookup(access, ea).is_some() {
            None
        } else {
            effects.truncate(effects_before);
            self.store_buf.truncate(stores_before);
            Some(self.refill_for(mem, ea, access))
        };
        let t = match refilled {
            // A held address refuses as unmapped only when the access
            // wraps past the end of the effective space.
            None if views_hold(regions.as_slice(), ea) => {
                return MissStep::Stop(FaultExit::Verdict(PpuFault::Unmodelled(
                    "an access that wraps the effective address space",
                )));
            }
            None => {
                return MissStep::Stop(self.unbacked_access(
                    insn,
                    mem,
                    regions.as_slice(),
                    ea,
                    access,
                ));
            }
            Some(Err(refused)) => return MissStep::Stop(FaultExit::Verdict(refused)),
            Some(Ok(t)) => t,
        };
        if self.alias_with_buffered_stores(&t) {
            return MissStep::EndBatch;
        }
        *regions = RegionTable::new(self.translated_views(mem).into_iter());
        if views_hold(regions.as_slice(), ea) {
            return MissStep::Rerun;
        }
        // The access translates to a real address no storage holds.
        MissStep::Stop(self.unbacked_access(insn, mem, regions.as_slice(), ea, access))
    }

    /// Takes back what an attempt whose access missed in the translation
    /// cache did before the instruction runs again: its break skip, its
    /// provisional reads and its reservation witnesses.
    #[inline]
    fn retract_missed_attempt(
        &mut self,
        mem: &cellgov_mem::GuestMemory,
        step_pc: u64,
        reads_before: u64,
        witnesses: Option<(u64, u64)>,
    ) {
        if self.break_pc == Some(step_pc) {
            self.break_skip += 1;
        }
        mem.rewind_provisional_reads(reads_before);
        self.restore_witnesses(witnesses);
    }

    /// Puts back the `lwarx` and `ldarx` execution counts an attempt
    /// changed before its access missed in the translation cache or
    /// faulted.
    ///
    /// The attempt that runs again counts the instruction. So a miss
    /// counts it once, as a hit does, and a faulted attempt counts nothing.
    fn restore_witnesses(&mut self, witnesses: Option<(u64, u64)>) {
        if let Some((lwarx, ldarx)) = witnesses {
            self.state.lwarx_executed = lwarx;
            self.state.ldarx_executed = ldarx;
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
    ///
    /// When no instruction of the batch retired, the thread ends at the
    /// fault, or takes its interrupt at the kernel's vector
    /// ([`Self::delivered_interrupt`]).
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
            FaultExit::Verdict(f) => match self.delivered_interrupt(&f) {
                Some(interrupt) => self.interrupt_yield(entry, effects, step_pc, &interrupt),
                None => self.fault_yield(entry, effects, step_pc, &f),
            },
            FaultExit::Mem(e, store) => self.mem_fault_yield(entry, effects, step_pc, e, store),
        }
    }

    /// Applies the runtime's committed inputs and clears the clock-read flag.
    ///
    /// The inputs the runtime committed since the last batch:
    /// - the reservation clear
    /// - a syscall return
    /// - whether the unit left the PPE since its last step
    /// - register writes
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

    /// Gives an `mftb` / `mftbu`, or a move to or from the decrementer,
    /// the time base at its issue cycle.
    #[inline]
    fn align_clock_read_to_issue(
        &mut self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        step_pc: u64,
        insn: &instruction::PpuInstruction,
        ops: &[crate::timing::Op],
    ) {
        let reads_time_base = match insn {
            instruction::PpuInstruction::Mftb { .. }
            | instruction::PpuInstruction::Mftbu { .. } => true,
            instruction::PpuInstruction::PpePrivileged { raw } => {
                crate::supervisor::moves_decrementer(*raw, &self.state)
            }
            _ => false,
        };
        let (true, [op]) = (reads_time_base, ops) else {
            return;
        };
        self.state.tb = self.time_base_at_issue(ctx, entry, step_pc, op);
    }

    /// The time base that `op` at `step_pc` reads at its issue cycle.
    ///
    /// The run has one time base: a read returns no less than any read
    /// before it, on this unit or another.
    #[inline]
    pub(super) fn time_base_at_issue(
        &self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        step_pc: u64,
        op: &crate::timing::Op,
    ) -> u64 {
        let issue = self.timing.issue_cycle(step_pc, op);
        let tick = ctx
            .current_tick()
            .raw()
            .saturating_add(issue.saturating_sub(entry.cycle));
        let tb = PPU_TIME_BASE_ORIGIN.wrapping_add(cellgov_time::ticks_to_tb(tick));
        self.state.tb.max(tb).max(ctx.time_base())
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
    /// instruction reported, or takes it at the kernel's vector, then
    /// counts it.
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
        let mut result = match self.delivered_interrupt(&fault) {
            Some(interrupt) => self.interrupt_yield(entry, effects, step_pc, &interrupt),
            None => self.fault_yield(entry, effects, step_pc, &fault),
        };
        // Only the terminal fault's diagnostics carry the data access.
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

/// Whether a view of `views` holds the byte at `ea`.
#[inline]
fn views_hold(views: &[cellgov_mem::RegionView<'_>], ea: u64) -> bool {
    views
        .iter()
        .any(|v| v.base <= ea && ea - v.base < v.bytes.len() as u64)
}

/// The base guest address of a region and its bytes.
pub(super) type CodeBytes<'a> = (u64, &'a [u8]);

/// The fetchable span that holds the word at `pc`, and its region.
///
/// The span is its first byte and the last word that starts in it.
///
/// # Errors
///
/// Returns `PpuInterrupt::InstructionStorage` if a byte of the word is
/// not fetchable. The interrupt sets `no_execute` if that byte is mapped.
pub(super) fn fetch_window(
    mem: &cellgov_mem::GuestMemory,
    pc: u64,
) -> Result<((u64, u64), CodeBytes<'_>), PpuFault> {
    use cellgov_mem::FetchRefusal;
    let refused = |r: FetchRefusal| {
        PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: r == FetchRefusal::NoExecute,
            protection: false,
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
            protection: false,
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

#[cfg(test)]
#[path = "tests/translation_hit_miss_tests.rs"]
mod translation_hit_miss_tests;

#[cfg(test)]
#[path = "tests/faulted_reserve_count_tests.rs"]
mod faulted_reserve_count_tests;
