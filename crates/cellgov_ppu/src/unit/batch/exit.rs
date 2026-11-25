//! The fault exits of a batch:
//! - the attempt of an instruction and the counts its rollback returns to;
//! - how an instruction faulted and what the loop does next;
//! - the rollback of a faulted attempt;
//! - the yield at the fault.

use super::miss::MissStep;
use super::regions::RegionTable;
use super::run::BatchEntry;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::instruction;
use crate::unit::ppu_unit::PpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_exec::ExecutionStepResult;

/// How an instruction faulted.
pub(in crate::unit) enum FaultExit {
    /// A fault the fetch, the decode or the instruction reported.
    Verdict(PpuFault),
    /// A memory fault the instruction reported, and whether its access
    /// was a store.
    Mem(cellgov_mem::MemError, bool),
}

/// One run of an instruction, and the counts its rollback returns to.
pub(super) struct Attempt {
    /// The instruction, or the fused pair, that the attempt runs.
    pub(super) insn: instruction::PpuInstruction,
    pub(super) step_pc: u64,
    /// Whether `insn` is a fused pair.
    pub(super) pair: bool,
    pub(super) effects_before: usize,
    pub(super) stores_before: usize,
    /// The provisional read count before the attempt; 0 when `insn` is no
    /// fused pair and the thread does not translate.
    pub(super) reads_before: u64,
    /// The `lwarx` and `ldarx` execution counts before the attempt;
    /// `None` when the thread does not translate.
    pub(super) witnesses: Option<(u64, u64)>,
}

impl Attempt {
    /// The effect, buffered store and provisional read counts before the
    /// attempt.
    #[inline(always)]
    pub(super) fn marks(&self) -> (usize, usize, u64) {
        (self.effects_before, self.stores_before, self.reads_before)
    }
}

/// What the batch loop does with an attempt that faulted.
pub(super) enum Retry {
    /// The instruction runs again against the rebuilt views.
    Rerun,
    /// The fused pair runs again split.
    Split,
    /// The batch ends before the instruction, which the next batch runs
    /// again.
    EndBatch,
    /// The batch ends at this fault.
    Stop(FaultExit),
}

impl PpuExecutionUnit {
    /// Records the counts an attempt of `insn` at `step_pc` rolls back to.
    #[inline(always)]
    pub(super) fn begin_attempt(
        &self,
        mem: &cellgov_mem::GuestMemory,
        effects_len: usize,
        insn: instruction::PpuInstruction,
        step_pc: u64,
        translating: bool,
    ) -> Attempt {
        let pair = insn.is_super_pair();
        let stores_before = self.store_buf.len();
        // A pair that refuses runs again split, and
        // `undo_fused_attempt` takes back its reads. An instruction
        // that runs again after a translation miss takes back its
        // reads the same way.
        let reads_before = if pair || translating {
            mem.provisional_read_count()
        } else {
            0
        };
        // An `lwarx` or `ldarx` counts its execution before its access,
        // so an attempt that misses in the translation cache or faults
        // gives its count back before the instruction runs again.
        let witnesses =
            translating.then_some((self.state.lwarx_executed, self.state.ldarx_executed));
        Attempt {
            insn,
            step_pc,
            pair,
            effects_before: effects_len,
            stores_before,
            reads_before,
            witnesses,
        }
    }

    /// The [`Retry`] for `attempt`, which faulted with `exit`.
    #[inline]
    pub(super) fn settle_fault<'m>(
        &mut self,
        mem: &'m cellgov_mem::GuestMemory,
        effects: &mut Vec<Effect>,
        regions: &mut RegionTable<'m>,
        translating: bool,
        attempt: &Attempt,
        exit: FaultExit,
    ) -> Retry {
        let exit = match self.miss_step(mem, effects, regions, translating, attempt, exit) {
            MissStep::Stop(exit) => exit,
            MissStep::EndBatch => return Retry::EndBatch,
            MissStep::Rerun => return Retry::Rerun,
        };
        // The instruction did not retire, and it runs again: in the
        // next batch, or after the kernel's handler returns. So its
        // `lwarx` or `ldarx` count goes back, as after a miss.
        // [PPC-Book3 p:62 s:5.3.1] Depending on the interrupt type, the instruction that causes a precise interrupt may appear not to have begun execution.
        self.restore_witnesses(attempt.witnesses);
        if attempt.pair {
            self.undo_fused_attempt(mem, effects, attempt.marks(), attempt.step_pc);
            return Retry::Split;
        }
        Retry::Stop(exit)
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
    pub(super) fn undo_fused_attempt(
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

    /// Puts back the `lwarx` and `ldarx` execution counts an attempt
    /// changed before its access missed in the translation cache or
    /// faulted.
    ///
    /// The attempt that runs again counts the instruction. So a miss
    /// counts it once, as a hit does, and a faulted attempt counts nothing.
    pub(super) fn restore_witnesses(&mut self, witnesses: Option<(u64, u64)>) {
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
    pub(super) fn end_at_fault(
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
