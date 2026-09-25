//! One batch: the fetch-decode-execute loop and its yields.

use super::ppu_unit::PpuExecutionUnit;
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::tap::PpuTap;
use crate::{
    decode, exec, instruction, state, FAULT_DEBUG_BREAK, FAULT_DECODE_ERROR, FAULT_INVALID_ADDRESS,
    FAULT_PC_OUT_OF_RANGE,
};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_time::{Budget, InstructionCost};

/// What a batch rewinds to when a step inside it faults.
///
/// `snapshot` is `None` for a single-step batch: no step retires ahead
/// of a fault there, so the rollback needs no state clone.
pub(super) struct BatchEntry {
    pub(super) snapshot: Option<state::PpuState>,
    pub(super) hashes: usize,
    pub(super) fulls: usize,
    pub(super) retired: u64,
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

        // Cross-unit reservation clear: committed table is authoritative.
        if self.state.reservation().is_some() && !ctx.reservation_held(self.id) {
            self.state.set_reservation(None);
        }

        // `max` preserves strict TB monotonicity when a prior step's
        // mftb advances outran the inter-step tick delta.
        let tb_from_tick = cellgov_time::ticks_to_tb(ctx.current_tick().raw());
        if tb_from_tick > self.state.tb {
            self.state.tb = tb_from_tick;
        }
        // The resync reaches no guest register, so it is no clock read:
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

        // Taken after the runtime's committed inputs (syscall return,
        // register writes) are applied: a mid-batch rollback must not
        // undo state the commit pipeline already owns.
        let entry = BatchEntry {
            snapshot: (max_budget > 1).then(|| self.state.clone()),
            hashes: self.per_step_hashes.len(),
            fulls: self.per_step_full_states.len(),
            retired: self.retirement_counter,
        };

        let mem = ctx.memory().as_bytes();
        // Stack-allocated region table avoids per-call heap alloc on the
        // Budget=1 hot path. Boot installs six regions and each
        // shared-memory mapping adds one; a larger layout spills to the
        // heap. Nothing guest-visible turns on the cutoff.
        const MAX_REGIONS: usize = 8;
        let mut region_views_storage: [cellgov_mem::RegionView<'_>; MAX_REGIONS] =
            [cellgov_mem::RegionView::plain(0, &[]); MAX_REGIONS];
        let mut region_views_spill: Vec<cellgov_mem::RegionView<'_>> = Vec::new();
        let mut n_regions = 0usize;
        for view in ctx.memory().region_views() {
            if n_regions < MAX_REGIONS {
                region_views_storage[n_regions] = view;
            } else {
                if region_views_spill.is_empty() {
                    region_views_spill.extend_from_slice(&region_views_storage);
                }
                region_views_spill.push(view);
            }
            n_regions += 1;
        }
        let region_views: &[cellgov_mem::RegionView<'_>] = if n_regions <= MAX_REGIONS {
            &region_views_storage[..n_regions]
        } else {
            &region_views_spill
        };

        loop {
            let step_pc = self.state.pc;

            // Diagnostics are taken before the rollback so they carry
            // the registers at the break; the window's effects are
            // discarded like any other mid-window fault.
            if self.break_pc == Some(step_pc) {
                if self.break_skip > 0 {
                    self.break_skip -= 1;
                } else {
                    self.break_pc = None;
                    let diag = self.fault_diag(step_pc);
                    return self.fault_yield(&entry, effects, diag, FAULT_DEBUG_BREAK);
                }
            }

            self.note_fetch(step_pc);
            let insn = if let Some(cached) = self
                .instruction_shadow
                .as_ref()
                .and_then(|s| s.get(step_pc))
            {
                self.shadow_hits += 1;
                cached
            } else {
                self.shadow_misses += 1;
                let pc = step_pc as usize;
                if pc + 4 > mem.len() {
                    let diag = self.fault_diag(step_pc);
                    return self.fault_yield(&entry, effects, diag, FAULT_PC_OUT_OF_RANGE);
                }
                let raw = u32::from_be_bytes([mem[pc], mem[pc + 1], mem[pc + 2], mem[pc + 3]]);
                match decode::decode(raw) {
                    Ok(i) => {
                        if let Some(s) = self.instruction_shadow.as_mut() {
                            let _ = s.refresh(step_pc, raw);
                        }
                        i
                    }
                    Err(_) => {
                        let diag = self.fault_diag(step_pc);
                        return self.fault_yield(&entry, effects, diag, FAULT_DECODE_ERROR);
                    }
                }
            };

            // `Consumed` is the second slot of a fused super-pair; it
            // must retire so retirement_counter and consumed_cost stay
            // aligned.
            if matches!(insn, instruction::PpuInstruction::Consumed) {
                self.state.pc += 4;
                if ctx.trace_per_step() {
                    self.per_step_hashes
                        .push((step_pc, self.state.state_hash()));
                }
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
                remaining = remaining.saturating_sub(1);
                if remaining == 0 {
                    self.close_block(effects);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::BudgetExhausted,
                        consumed_cost: InstructionCost::new(budget.raw()),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
                continue;
            }

            tap.dispatch(self.id, &insn, &self.state);
            match exec::execute(
                &insn,
                &mut self.state,
                self.id,
                region_views,
                effects,
                &mut self.store_buf,
            ) {
                ExecuteVerdict::Continue => {
                    self.state.pc += 4;
                    // Super-pair without `Consumed` at PC+4 would
                    // re-execute its second half.
                    assert!(
                        !insn.is_super_pair()
                            || self
                                .instruction_shadow
                                .as_ref()
                                .and_then(|s| s.get(self.state.pc))
                                .is_none_or(|next| matches!(
                                    next,
                                    instruction::PpuInstruction::Consumed
                                )),
                        "super-pair {} at 0x{step_pc:x} not followed by Consumed at 0x{:x}",
                        <&'static str>::from(&insn),
                        self.state.pc,
                    );
                }
                ExecuteVerdict::Branch => {}
                ExecuteVerdict::Syscall { lev } => {
                    self.close_block(effects);
                    let args = state::ppu_syscall_args(&self.state);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::Syscall,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(
                            step_pc,
                            self.state.lr(),
                            lev,
                        ),
                        fault: None,
                        syscall_args: Some(args),
                    };
                }
                ExecuteVerdict::Fault(f) => {
                    // Diag captured before rollback so registers
                    // reflect the fault site. Address-bearing variants
                    // route the address through `faulting_ea` to keep
                    // the low 16 bits free for the category-prefix
                    // contract with cellgov_core.
                    let diag = match f {
                        PpuFault::InvalidAddress(a) => self.fault_diag_ea(step_pc, a),
                        PpuFault::PcOutOfRange(a) => self.fault_diag_ea(step_pc, a),
                        PpuFault::AlignmentInterrupt(a) => self.fault_diag_ea(step_pc, a),
                        _ => self.fault_diag(step_pc),
                    };
                    // Mask guards against upper-bit collision with the category prefix.
                    let code = f.guest_code();
                    return self.fault_yield(&entry, effects, diag, code);
                }
                ExecuteVerdict::MemFault(e) => {
                    let (ea, unmapped) = match &e {
                        cellgov_mem::MemError::Unmapped(ctx) => (ctx.addr, true),
                        // Only Unmapped is reachable on this path.
                        _ => {
                            debug_assert!(
                                false,
                                "ExecuteVerdict::MemFault carrying non-Unmapped MemError: {e:?}"
                            );
                            (0, false)
                        }
                    };
                    let diag = self.fault_diag_ea(step_pc, ea);
                    let result = self.fault_yield(&entry, effects, diag, FAULT_INVALID_ADDRESS);
                    // Incremented after the rollback: the counters are
                    // hash-excluded instruments, and restoring the entry
                    // snapshot must not erase the record of this fault.
                    self.state.mem_fault_arm_entries =
                        self.state.mem_fault_arm_entries.wrapping_add(1);
                    if unmapped {
                        self.state.mem_fault_unmapped_routed =
                            self.state.mem_fault_unmapped_routed.wrapping_add(1);
                    }
                    return result;
                }
                ExecuteVerdict::BufferFull => {
                    // PC stays at the failing store; retries next step.
                    // Break skips count retirements, and nothing retired,
                    // so a skip spent at the top of this iteration is
                    // handed back.
                    if self.break_pc == Some(step_pc) {
                        self.break_skip += 1;
                    }
                    self.close_block(effects);
                    return ExecutionStepResult {
                        yield_reason: YieldReason::BudgetExhausted,
                        consumed_cost: InstructionCost::new(budget.raw() - remaining),
                        local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                        fault: None,
                        syscall_args: None,
                    };
                }
            }

            if self.profile_mode {
                // Attribute to the dispatched variant so quickenings
                // and super-pairs show up in the profile.
                let name: &'static str = (&insn).into();
                *self.profile_insns.entry(name).or_insert(0) += 1;
                if let Some(prev) = self.profile_prev {
                    *self.profile_pairs.entry((prev, name)).or_insert(0) += 1;
                }
                self.profile_prev = Some(name);
            }

            if ctx.trace_per_step() {
                self.per_step_hashes
                    .push((step_pc, self.state.state_hash()));
            }

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

            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                self.close_block(effects);
                return ExecutionStepResult {
                    yield_reason: YieldReason::BudgetExhausted,
                    consumed_cost: InstructionCost::new(budget.raw()),
                    local_diagnostics: LocalDiagnostics::with_pc(step_pc),
                    fault: None,
                    syscall_args: None,
                };
            }
        }
    }
}
