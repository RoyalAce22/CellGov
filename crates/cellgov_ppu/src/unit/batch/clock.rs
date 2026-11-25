//! The batch clock and the placement of a PPE thread beside its siblings:
//! - the cost of a batch and the yield at a spent budget;
//! - the time base at an issue cycle;
//! - the sibling grid;
//! - the CTRL value a thread reads.

use super::run::BatchEntry;
use crate::instruction;
use crate::unit::ppu_unit::PpuExecutionUnit;
use cellgov_effects::Effect;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_ps3_abi::hw::ppu::PPU_TIME_BASE_ORIGIN;
use cellgov_time::InstructionCost;

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
pub(in crate::unit) const SIBLING_LEAD_CYCLES: u64 = 256;

impl PpuExecutionUnit {
    /// Guest ticks for the instructions this batch retired: the core
    /// cycles the issue model advanced.
    ///
    /// A tick is one core cycle. The second instruction of a pair that
    /// issues with the first costs nothing.
    pub(in crate::unit) fn cost(&self, entry: &BatchEntry) -> InstructionCost {
        InstructionCost::new(self.timing.now() - entry.cycle)
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
    pub(super) fn leads_sibling(&self, ctx: &ExecutionContext<'_>, entry: &BatchEntry) -> bool {
        if ctx.ppe_siblings() == 0 {
            return false;
        }
        let into_line = ctx.current_tick().raw() % SIBLING_LEAD_CYCLES;
        into_line.saturating_add(self.timing.now() - entry.cycle) >= SIBLING_LEAD_CYCLES
    }

    /// Gives an `mftb` / `mftbu`, or a move to or from the decrementer,
    /// the time base at its issue cycle.
    #[inline]
    pub(super) fn align_clock_read_to_issue(
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
    pub(in crate::unit) fn time_base_at_issue(
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

    /// Closes the block and yields with the budget spent at `pc`.
    #[inline]
    pub(super) fn budget_yield(
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
pub(super) fn ctrl_for_placement(siblings: u32, older_sibling: bool) -> u32 {
    use cellgov_ps3_abi::hw::ppu_spr::{
        CTRL_CT_THREAD0, CTRL_CT_THREAD1, CTRL_TE_THREAD0, CTRL_TE_THREAD1, PS3_PROCESS_CTRL,
    };
    match (siblings, older_sibling) {
        (0, _) => PS3_PROCESS_CTRL,
        (_, false) => CTRL_CT_THREAD1 | CTRL_TE_THREAD0 | CTRL_TE_THREAD1,
        (_, true) => CTRL_CT_THREAD0 | CTRL_TE_THREAD0 | CTRL_TE_THREAD1,
    }
}
