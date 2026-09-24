//! PPU `ExecutionUnit`: fetch-decode-execute loop. Guest-visible
//! writes leave via `Effect`s flushed at yield / fault /
//! budget-exhaustion; mid-batch faults discard the batch and roll
//! architectural state back to the step's entry snapshot.

#![cfg_attr(test, allow(clippy::unwrap_used))]
#![cfg_attr(
    not(test),
    forbid(
        clippy::disallowed_methods,
        clippy::disallowed_macros,
        clippy::print_stdout,
        clippy::print_stderr,
        clippy::dbg_macro
    )
)]

pub mod caller_census;
pub mod decode;
pub mod differential;
pub mod disasm;
pub mod exec;
mod fp;
pub mod funcmap;
pub mod instruction;
pub mod loader;
pub mod lv2_gate;
pub mod lv2_stub;
pub mod lv2_subdispatch;
pub mod lv2_table;
pub mod observation;
pub mod prescan;
pub mod prx;
pub mod prx_loader;
pub mod shadow;
pub mod sprx;
pub mod state;
pub mod store_buffer;
pub mod tap;

pub use tap::PpuTap;

use crate::exec::{ExecuteVerdict, PpuFault};
use crate::store_buffer::StoreBuffer;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, FaultRegisterDump, LocalDiagnostics,
    UnitStatus, YieldReason,
};
use cellgov_time::{Budget, InstructionCost};

/// PPU tried to fetch at an address beyond guest memory.
pub const FAULT_PC_OUT_OF_RANGE: u32 = 0x0102_0000;
/// Instruction word did not match any implemented encoding.
pub const FAULT_DECODE_ERROR: u32 = 0x0105_0000;
/// Load or store targeted an out-of-bounds guest address.
pub const FAULT_INVALID_ADDRESS: u32 = 0x0106_0000;
/// Syscall number has no handler.
pub const FAULT_UNSUPPORTED_SYSCALL: u32 = 0x0107_0000;
/// Debug breakpoint fired at a user-requested PC.
pub const FAULT_DEBUG_BREAK: u32 = 0x0108_0000;
/// Decoded instruction (typically a VMX sub-opcode) had no exec arm.
pub const FAULT_UNIMPLEMENTED_INSN: u32 = 0x0109_0000;
/// Program trap fired (e.g. `tw` / `td` with a TO-selected
/// condition met).
pub const FAULT_PROGRAM_TRAP: u32 = 0x010A_0000;
/// Reservation operand EA not aligned to operand size.
pub const FAULT_ALIGNMENT_INTERRUPT: u32 = 0x010B_0000;
/// Instruction encoded in an invalid form.
pub const FAULT_INVALID_FORM: u32 = 0x010C_0000;

/// True when `code` belongs to the [`FAULT_DECODE_ERROR`] class.
#[inline]
pub fn is_decode_error(code: u32) -> bool {
    (code & 0xFFFF_0000) == FAULT_DECODE_ERROR
}

/// PPU architectural state snapshot for replay.
// [PPC-Book1 p:18 s:2.3 Branch Processor Registers] CR is 32 bits in eight 4-bit fields; LR and CTR are 64-bit branch registers.
#[derive(Debug, Clone)]
pub struct PpuSnapshot {
    /// General-purpose registers.
    pub gpr: [u64; 32],
    /// Raw f64 bit patterns, matching `PpuState`.
    pub fpr: [u64; 32],
    /// Big-endian (byte 0 in MSB).
    pub vr: [u128; 32],
    /// Program counter.
    pub pc: u64,
    /// 8 nibble fields.
    pub cr: u32,
    /// Link register.
    pub lr: u64,
    /// Count register.
    pub ctr: u64,
    /// Fixed-point exception register.
    pub xer: u64,
    // [PPC-Book2 p:29 s:Chapter 4. Time Base] TB is a 64-bit unsigned counter incremented monotonically.
    /// Time base register.
    pub tb: u64,
    /// Canonical reservation-line address, or `None` when no reservation is held.
    pub reservation_line: Option<u64>,
}

/// PPU `ExecutionUnit`: owns architectural state, fetches and executes
/// instructions, emits `Effect`s for stores and syscalls.
#[derive(Clone)]
pub struct PpuExecutionUnit {
    id: UnitId,
    state: state::PpuState,
    status: UnitStatus,
    /// Fires `FAULT_DEBUG_BREAK` after `break_skip` prior hits at this PC.
    break_pc: Option<u64>,
    break_skip: u32,
    per_step_hashes: Vec<(u64, u64)>,
    full_state_window: Option<(u64, u64)>,
    /// Increments on successful retirement only.
    retirement_counter: u64,
    per_step_full_states: Vec<(u64, u64, cellgov_exec::PpuFingerprint)>,
    instruction_shadow: Option<shadow::PredecodedShadow>,
    shadow_hits: u64,
    shadow_misses: u64,
    /// Start of the run of text the block is fetching from now.
    ///
    /// A fetch reads the text region whether or not the shadow answers
    /// it: a committed write there drives `invalidate_code` over the
    /// slot, so the next fetch takes the new bytes. Fetch is the
    /// highest-frequency read a boot has, so a straight run costs one
    /// compare and one add per instruction here and one read intent at
    /// the block boundary.
    fetch_start: u64,
    /// End of that run, or [`NO_FETCH_RUN`] when the block has none.
    ///
    /// A fetch that continues the run is one compare against this and
    /// one store back, which is what keeps the highest-frequency read
    /// in a boot off the profile.
    fetch_end: u64,
    /// Runs this block finished before the one `fetch_start` and
    /// `fetch_end` hold.
    ///
    /// A branch closes a run and opens another. Past
    /// [`FETCH_RUNS_MAX`] the block stops tracking them apart and
    /// reports one span covering every address it fetched, which is
    /// what a loop body would otherwise cost a run per iteration.
    fetch_runs: Vec<(u64, u64)>,
    store_buf: StoreBuffer,
    profile_mode: bool,
    profile_insns: std::collections::BTreeMap<&'static str, u64>,
    profile_pairs: std::collections::BTreeMap<(&'static str, &'static str), u64>,
    profile_prev: Option<&'static str>,
    /// `Clone` shares the tap, so a cloned unit reports to the same
    /// observer. A runtime snapshot holds such a clone.
    tap: Option<std::rc::Rc<dyn PpuTap>>,
}

impl PpuExecutionUnit {
    /// Fresh PPU unit with the given id and zeroed architectural state.
    pub fn new(id: UnitId) -> Self {
        Self {
            id,
            state: state::PpuState::new(),
            status: UnitStatus::Runnable,
            break_pc: None,
            break_skip: 0,
            per_step_hashes: Vec::new(),
            full_state_window: None,
            retirement_counter: 0,
            per_step_full_states: Vec::new(),
            instruction_shadow: None,
            shadow_hits: 0,
            shadow_misses: 0,
            fetch_start: 0,
            fetch_end: NO_FETCH_RUN,
            fetch_runs: Vec::new(),
            store_buf: StoreBuffer::new(),
            profile_mode: false,
            profile_insns: std::collections::BTreeMap::new(),
            profile_pairs: std::collections::BTreeMap::new(),
            profile_prev: None,
            tap: None,
        }
    }

    /// Report every dispatched instruction to `tap`.
    pub fn set_tap(&mut self, tap: std::rc::Rc<dyn PpuTap>) {
        self.tap = Some(tap);
    }

    /// Set the inclusive `[lo, hi]` retirement-index window for full-state capture.
    pub fn set_full_state_window(&mut self, window: Option<(u64, u64)>) {
        self.full_state_window = window;
    }

    /// Returns the current full-state-capture retirement-index window.
    pub fn full_state_window(&self) -> Option<(u64, u64)> {
        self.full_state_window
    }

    /// Skip `skip` hits at `pc`, then fault on the next.
    pub fn set_break_pc(&mut self, pc: u64, skip: u32) {
        self.break_pc = Some(pc);
        self.break_skip = skip;
    }

    /// Toggle per-instruction profile accumulation.
    pub fn set_profile_mode(&mut self, on: bool) {
        self.profile_mode = on;
    }

    /// Returns the accumulated per-instruction execution counts.
    pub fn profile_insns(&self) -> &std::collections::BTreeMap<&'static str, u64> {
        &self.profile_insns
    }

    /// Returns the accumulated adjacent-instruction-pair execution counts.
    pub fn profile_pairs(&self) -> &std::collections::BTreeMap<(&'static str, &'static str), u64> {
        &self.profile_pairs
    }

    /// Mutable access to architectural state.
    pub fn state_mut(&mut self) -> &mut state::PpuState {
        &mut self.state
    }

    /// Shared access to architectural state.
    pub fn state(&self) -> &state::PpuState {
        &self.state
    }

    /// Install the predecoded shadow. Caller must build it after all
    /// boot-time code writes (ELF/PRX load, HLE stub planting) and
    /// before the step loop begins; stale slots re-decode on every fetch.
    pub fn set_instruction_shadow(&mut self, shadow: shadow::PredecodedShadow) {
        self.instruction_shadow = Some(shadow);
    }

    /// Returns `(hits, misses)`; high miss ratios lose the O(1) fast path.
    pub fn shadow_stats(&self) -> (u64, u64) {
        (self.shadow_hits, self.shadow_misses)
    }
}

/// Runs one block reports apart before it collapses them into one
/// span.
const FETCH_RUNS_MAX: usize = 8;

/// [`PpuExecutionUnit::fetch_end`] when no run is open.
///
/// The value is odd and every branch form writes a word-aligned
/// target, so no pc reaches it from an aligned entry.
const NO_FETCH_RUN: u64 = u64::MAX;

impl PpuExecutionUnit {
    /// Publish what the block did to committed memory: the stores it
    /// buffered, and the text it fetched.
    ///
    /// Both reach the effect list at the block boundary rather than per
    /// instruction, so the two orders a write to the text region and a
    /// fetch of it can take are held apart without a packet per fetch.
    fn close_block(&mut self, effects: &mut Vec<Effect>) {
        self.store_buf.flush(effects, self.id);
        if self.fetch_end != NO_FETCH_RUN {
            self.fetch_runs.push((self.fetch_start, self.fetch_end));
            self.fetch_end = NO_FETCH_RUN;
        }
        for (start, end) in self.fetch_runs.drain(..) {
            let range =
                cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(start), end - start)
                    .expect("a run ends above its start, so its end is the u64 the range needs");
            effects.push(Effect::SharedReadIntent {
                range,
                source: self.id,
            });
        }
        if self.state.clock_read {
            effects.push(Effect::ClockRead { source: self.id });
        }
    }

    /// Extend the run in flight, or start another.
    #[inline(always)]
    fn note_fetch(&mut self, pc: u64) {
        // Every branch form writes a target with the low two bits
        // clear, so the sentinel's odd address names no fetch. The
        // assertion pins that rather than trusting it.
        debug_assert_ne!(pc, NO_FETCH_RUN, "a fetch at the no-run sentinel address");
        if self.fetch_end == pc {
            self.fetch_end = pc + 4;
        } else {
            self.start_fetch_run(pc);
        }
    }

    /// Drop the runs the block recorded without publishing them.
    ///
    /// A discarded batch retired nothing, so the text it fetched
    /// reaches no effect list. An open run left behind would instead
    /// publish at the next block's boundary, naming addresses that
    /// block never fetched.
    fn drop_fetch_runs(&mut self) {
        self.fetch_end = NO_FETCH_RUN;
        self.fetch_runs.clear();
    }

    /// Close the run in flight and open one at `pc`.
    ///
    /// Cold: a block reaches it once, plus once per branch it takes.
    /// Past [`FETCH_RUNS_MAX`] runs it stops tracking them apart and
    /// keeps one span, which is what a loop body would otherwise cost
    /// a run per iteration.
    #[cold]
    fn start_fetch_run(&mut self, pc: u64) {
        if self.fetch_end == NO_FETCH_RUN {
            self.fetch_start = pc;
            self.fetch_end = pc + 4;
            return;
        }
        if self.fetch_runs.len() >= FETCH_RUNS_MAX {
            let mut start = self.fetch_start.min(pc);
            let mut end = self.fetch_end.max(pc + 4);
            for (s, e) in self.fetch_runs.drain(..) {
                start = start.min(s);
                end = end.max(e);
            }
            self.fetch_start = start;
            self.fetch_end = end;
            return;
        }
        self.fetch_runs.push((self.fetch_start, self.fetch_end));
        self.fetch_start = pc;
        self.fetch_end = pc + 4;
    }

    fn capture_regs(&self) -> FaultRegisterDump {
        FaultRegisterDump {
            gprs: *self.state.gpr.as_array(),
            lr: self.state.lr(),
            ctr: self.state.ctr(),
            xer: self.state.xer(),
            cr: self.state.cr(),
        }
    }

    fn fault_diag(&self, pc: u64) -> LocalDiagnostics {
        LocalDiagnostics {
            pc: Some(pc),
            lr: Some(self.state.lr()),
            syscall_lev: None,
            faulting_ea: None,
            fault_regs: Some(self.capture_regs()),
        }
    }

    fn fault_diag_ea(&self, pc: u64, ea: u64) -> LocalDiagnostics {
        LocalDiagnostics {
            pc: Some(pc),
            lr: Some(self.state.lr()),
            syscall_lev: None,
            faulting_ea: Some(ea),
            fault_regs: Some(self.capture_regs()),
        }
    }

    /// Fault-discards-all: restore the batch-entry state, drop staged
    /// stores, fetch runs and effects, rewind the per-step trace to
    /// the batch entry, and mark the unit faulted.
    ///
    /// Retirements discarded by the rollback never reach the trace
    /// stream: their `PpuStateHash` / `PpuStateFull` entries are
    /// truncated and `retirement_counter` is restored.
    fn discard_batch(&mut self, entry: &BatchEntry, effects: &mut Vec<Effect>) {
        if let Some(snap) = entry.snapshot.as_ref() {
            self.state = snap.clone();
        }
        self.store_buf.clear();
        self.drop_fetch_runs();
        effects.clear();
        self.per_step_hashes.truncate(entry.hashes);
        self.per_step_full_states.truncate(entry.fulls);
        self.retirement_counter = entry.retired;
        self.status = UnitStatus::Faulted;
    }

    /// Roll the batch back and yield a guest fault carrying `code`.
    ///
    /// `diag` is a parameter because the rollback erases the fault
    /// site's registers; the caller captures it first.
    fn fault_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        diag: LocalDiagnostics,
        code: u32,
    ) -> ExecutionStepResult {
        self.discard_batch(entry, effects);
        ExecutionStepResult {
            yield_reason: YieldReason::Fault,
            consumed_cost: InstructionCost::ZERO,
            local_diagnostics: diag,
            fault: Some(FaultKind::Guest(code)),
            syscall_args: None,
        }
    }
}

/// What a batch rewinds to when a step inside it faults.
///
/// `snapshot` is `None` for a single-step batch: no step retires ahead
/// of a fault there, so the rollback needs no state clone.
struct BatchEntry {
    snapshot: Option<state::PpuState>,
    hashes: usize,
    fulls: usize,
    retired: u64,
}

/// The tap a unit with none installed runs its batch loop with.
struct NoTap;

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
    fn run_batch<T: PpuTap + ?Sized>(
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

impl ExecutionUnit for PpuExecutionUnit {
    type Snapshot = PpuSnapshot;

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        self.status
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        match self.tap.take() {
            None => self.run_batch(budget, ctx, effects, &NoTap),
            Some(tap) => {
                let result = self.run_batch(budget, ctx, effects, tap.as_ref());
                self.tap = Some(tap);
                result
            }
        }
    }

    fn snapshot(&self) -> PpuSnapshot {
        PpuSnapshot {
            gpr: *self.state.gpr.as_array(),
            fpr: *self.state.fpr.as_array(),
            vr: *self.state.vr.as_array(),
            pc: self.state.pc,
            cr: self.state.cr(),
            lr: self.state.lr(),
            ctr: self.state.ctr(),
            xer: self.state.xer(),
            tb: self.state.tb,
            reservation_line: self.state.reservation().map(|l| l.addr()),
        }
    }

    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        std::mem::take(&mut self.per_step_hashes)
    }

    fn drain_retired_state_full(&mut self) -> Vec<(u64, u64, cellgov_exec::PpuFingerprint)> {
        std::mem::take(&mut self.per_step_full_states)
    }

    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        let map = std::mem::take(&mut self.profile_insns);
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        let map = std::mem::take(&mut self.profile_pairs);
        let mut v: Vec<_> = map.into_iter().collect();
        v.sort_by_key(|e| std::cmp::Reverse(e.1));
        v
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        if let Some(s) = self.instruction_shadow.as_mut() {
            s.invalidate_range(addr, len);
        }
    }

    fn caches_code(&self) -> bool {
        self.instruction_shadow.is_some()
    }

    fn shadow_stats(&self) -> (u64, u64) {
        (self.shadow_hits, self.shadow_misses)
    }

    fn register_dump(&self) -> Option<cellgov_exec::FaultRegisterDump> {
        Some(cellgov_exec::FaultRegisterDump {
            gprs: *self.state.gpr.as_array(),
            lr: self.state.lr(),
            ctr: self.state.ctr(),
            xer: self.state.xer(),
            cr: self.state.cr(),
        })
    }
}

#[cfg(test)]
#[path = "tests/ppu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/break_pc_tests.rs"]
mod break_pc_tests;

#[cfg(test)]
#[path = "tests/batch_fault_tests.rs"]
mod batch_fault_tests;

#[cfg(test)]
#[path = "tests/tap_tests.rs"]
mod tap_tests;

#[cfg(test)]
#[path = "tests/clock_read_tests.rs"]
mod clock_read_tests;
