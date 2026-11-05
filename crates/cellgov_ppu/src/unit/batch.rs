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
            retired: self.retirement_counter,
            cycle: self.timing.now(),
            timing: (max_budget > 1).then(|| self.timing.core()),
        };

        let mem = ctx.memory().as_bytes();
        let regions = RegionTable::new(ctx.memory().region_views());
        let region_views = regions.as_slice();
        // A window's bytes are another unit's state, which the runtime
        // updates only between batches. The batch ends at the instruction
        // that reaches a window, so the next instruction sees the update.
        let window_span = ctx.memory().window_span();

        // The pc of the last instruction the batch retired.
        let mut last_retired = self.state.pc;
        // A fused pair that faulted runs again split, at this pc.
        let mut force_split = None;
        // The span of the buffered stores when the last `isync` retired.
        let mut synced_stores: Option<(u64, u64)> = None;
        // The fetchable span the last fetch read: its first byte and the
        // last word that starts in it. It starts empty.
        let mut fetchable = (1, 0);

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
                fetch_window(ctx.memory(), step_pc).map(|window| fetchable = window)
            };
            let mut insn = match fetched.and_then(|()| self.fetch_decode(step_pc, mem)) {
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
            // - per-step tracing is on, or
            // - a full-state window is set, or
            // - the second slot is not fetchable.
            // The trace and the window record the state after each
            // instruction, and a fused head has no state between its slots.
            // A head that faulted fused also runs split, so its first
            // instruction retires before the fault.
            let split = match insn {
                instruction::PpuInstruction::Consumed => true,
                _ if insn.is_super_pair() => {
                    forced
                        || remaining < 2
                        || !holds_word(fetchable, step_pc.wrapping_add(4))
                        || self.break_pc == Some(step_pc + 4)
                        || ctx.trace_per_step()
                        || self.full_state_window.is_some()
                }
                _ => false,
            };
            if split {
                insn = match decode_stored(step_pc, mem) {
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
                    // batch applies the return value and the step past it.
                    self.note_fetch(step_pc);
                    self.timing.retire_instruction(step_pc, ops, step_pc + 4);
                    if ctx.trace_per_step() {
                        self.per_step_hashes
                            .push((step_pc, self.state.state_hash()));
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
                    // A fused instruction may have applied its first half.
                    // That half reruns alone and leaves the same state. A
                    // first half that can write a register it reads
                    // (`lwz rT,d(rT)`) is a load, and faults before it
                    // writes. No other first half reads what it writes.
                    effects.truncate(effects_before);
                    self.store_buf.truncate(stores_before);
                    if self.break_pc == Some(step_pc) {
                        self.break_skip += 1;
                    }
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
            }

            if ctx.trace_per_step() {
                self.per_step_hashes
                    .push((step_pc, self.state.state_hash()));
                if let Some(kind) = insn.barrier_kind() {
                    self.barriers
                        .push(cellgov_exec::RetiredBarrier { pc: step_pc, kind });
                }
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
                    effects.get(effects_before..).unwrap_or_default(),
                    self.store_buf.stores_from(stores_before),
                )
            });
            last_retired = last_pc;
            if matches!(insn, instruction::PpuInstruction::Isync) {
                synced_stores = self.store_buf.span();
            }
            if remaining == 0 || reached_window {
                return self.budget_yield(&entry, effects, last_pc);
            }
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
        mem: &[u8],
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
        let (raw, insn) = decode_stored(step_pc, mem)?;
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

/// The fetchable span of `mem` that holds the word at `pc`, as its first
/// byte and the last word that starts in it.
///
/// # Errors
///
/// Returns `PpuInterrupt::InstructionStorage` if a byte of the word is
/// not fetchable. The interrupt sets `no_execute` if that byte is mapped.
fn fetch_window(mem: &cellgov_mem::GuestMemory, pc: u64) -> Result<(u64, u64), PpuFault> {
    use cellgov_mem::FetchRefusal;
    let refused = |r: FetchRefusal| {
        PpuFault::Interrupt(PpuInterrupt::InstructionStorage {
            no_execute: r == FetchRefusal::NoExecute,
        })
    };
    let (start, end) = mem.fetch_span(pc).map_err(refused)?;
    match pc.checked_add(4) {
        Some(word_end) if word_end <= end => Ok((start, end - 4)),
        _ => Err(refused(
            mem.fetch_span(end).err().unwrap_or(FetchRefusal::Unmapped),
        )),
    }
}

/// The word stored at `pc` and its decode, without the shadow.
///
/// # Errors
///
/// - A fetch past guest memory: the instruction storage interrupt.
/// - A word with no decode: the illegal-instruction program interrupt.
#[inline]
fn decode_stored(pc: u64, mem: &[u8]) -> Result<(u32, instruction::PpuInstruction), PpuFault> {
    let bytes = usize::try_from(pc)
        .ok()
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

/// Whether one instruction's new read intents or staged stores reach
/// `[start, end)`.
fn touches_window(
    (start, end): (u64, u64),
    new_effects: &[Effect],
    mut new_stores: impl Iterator<Item = (u64, u8)>,
) -> bool {
    let overlaps = |addr: u64, len: u64| addr < end && addr.saturating_add(len) > start;
    new_effects.iter().any(|e| match e {
        Effect::SharedReadIntent { range, .. } => overlaps(range.start().raw(), range.length()),
        _ => false,
    }) || new_stores.any(|(addr, len)| overlaps(addr, u64::from(len)))
}
