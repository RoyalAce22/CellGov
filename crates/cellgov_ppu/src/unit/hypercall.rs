//! The hypercall yield of a thread with supervisor registers.
//!
//! On such a thread, `sc` with LEV = 1 is a hypercall in either
//! processor state. The thread yields it to the runtime, which answers
//! as the hypervisor. The answer arrives as the runtime's committed
//! inputs of the next batch:
//!
//! - r3
//! - the outputs from r4
//! - r0, r11 and r12
//! - the CR fields of the hypervisor's return
//! - the step past `sc`
//!
//! The runtime names the request's PPE hardware thread from the
//! thread's CTRL.

use cellgov_effects::Effect;
use cellgov_exec::{
    BoundaryRegisters, CompareFields, CompareResult, ExecutionStepResult, LocalDiagnostics,
    SaveRestore, YieldReason,
};
use cellgov_ps3_abi::hw::ppu::PPU_TIME_BASE_ORIGIN;
use cellgov_ps3_abi::hw::ppu_spr::CTRL_CT_THREAD1;
use cellgov_time::GuestTicks;

use super::batch::BatchEntry;
use super::interrupt::entry_srr1;
use super::ppu_unit::PpuExecutionUnit;
use crate::exec::PpuInterrupt;
use crate::state::{self, cmp_cr_field, PpuState};

/// Gives `state` the CR fields of `fields`, each with XER\[SO\] as its SO
/// bit, as the compares that set them do.
pub(super) fn take_compare_fields(state: &mut PpuState, fields: CompareFields) {
    let so = state.xer_so();
    for (field, result) in (0u8..).zip(fields.0) {
        let Some(result) = result else {
            continue;
        };
        let nibble = cmp_cr_field(
            result == CompareResult::Less,
            result == CompareResult::Greater,
            so,
        );
        state.set_cr_field(field, nibble);
    }
}

impl PpuExecutionUnit {
    /// Ends the batch at the `sc` at `pc` on a thread with supervisor registers.
    ///
    /// The `sc` retired with LEV `lev`:
    /// - LEV = 1 yields a hypercall;
    /// - any other LEV takes the system call interrupt at the kernel's
    ///   vector.
    ///
    /// The hypervisor reads the processor state of a LEV = 1 call from
    /// `ExecutionUnit::in_problem_state`.
    pub(super) fn supervisor_sc_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        lev: u8,
    ) -> ExecutionStepResult {
        if lev == 1 {
            return self.hypercall_yield(entry, effects, pc);
        }
        self.system_call_yield(entry, effects, pc, lev)
    }

    /// Yields the hypercall of the `sc` at `pc`: its number from r11 and
    /// its inputs from r3 to r10.
    ///
    /// The `sc` stays the thread's pc until the runtime's answer moves it
    /// past. The issue model charges nothing past the `sc`, because no
    /// hypercall number has a measured cost.
    fn hypercall_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
    ) -> ExecutionStepResult {
        self.timing.drop_pending_syscall();
        self.close_block(effects);
        ExecutionStepResult {
            yield_reason: YieldReason::Syscall,
            consumed_cost: self.cost(entry),
            local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(pc, self.state.lr(), 1),
            fault: None,
            syscall_args: Some(state::ppu_syscall_args(&self.state)),
        }
    }

    /// The SRR0 and SRR1 that the system call interrupt of the `sc` at
    /// the thread's pc sets, or `None` for a thread with no supervisor
    /// registers.
    ///
    /// The hypervisor's return through `hrfid` puts these values in r11
    /// and r12. CellGov does not write them to the thread's own SRR0 and
    /// SRR1.
    pub(super) fn save_restore_at_sc(&self) -> Option<SaveRestore> {
        let msr = self.state.supervisor()?.msr;
        let record = PpuInterrupt::SystemCall { lev: 1 }.record(self.state.pc);
        Some(SaveRestore {
            srr0: record.srr0,
            srr1: entry_srr1(msr, record.srr1),
        })
    }

    /// The registers at the thread's next instruction boundary, or `None`
    /// for a thread with no supervisor registers.
    ///
    /// The decrementer reads the latest of three time bases, as the
    /// thread's own read at `tick` does:
    /// - the time base of `tick`;
    /// - the time base that the thread last read;
    /// - `time_base`, the highest time base that any unit read.
    pub(super) fn registers_at_boundary(
        &self,
        tick: GuestTicks,
        time_base: u64,
    ) -> Option<BoundaryRegisters> {
        let sup = self.state.supervisor()?;
        let tb = self.boundary_read_time_base(tick, time_base);
        Some(BoundaryRegisters {
            pc: self.state.pc,
            msr: sup.msr,
            dec: sup.dec_at(tb),
        })
    }

    /// The guest tick at which the thread's decrementer reaches 0, or
    /// `None` for a thread with no supervisor registers.
    ///
    /// The decrementer reads at the time base of
    /// [`Self::registers_at_boundary`]. The deadline is that time base plus
    /// the decrementer, sign-extended, as lv1 arms the wake of a paused
    /// thread [OFW-4.93 lv1 a:0x2C6110-0x2C613C]. The answer is the first
    /// tick whose time base reaches the deadline, and never before `tick`.
    ///
    /// A decrementer that reads 0 or negative gives `tick`: the thread
    /// already reads its time base at or past the deadline, also when that
    /// time base leads the time base of `tick`.
    ///
    /// [PPC-Book3 p:80 s:6.3] The DEC counts down at the time-base frequency.
    pub(super) fn decrementer_zero_tick(
        &self,
        tick: GuestTicks,
        time_base: u64,
    ) -> Option<GuestTicks> {
        let sup = self.state.supervisor()?;
        let tb = self.boundary_read_time_base(tick, time_base);
        let dec = i64::from(sup.dec_at(tb) as i32);
        let Some(ahead) = u64::try_from(dec).ok().filter(|&ahead| ahead > 0) else {
            return Some(tick);
        };
        let deadline = tb.saturating_add(ahead);
        let ticks = cellgov_time::tb_to_ticks(deadline.saturating_sub(PPU_TIME_BASE_ORIGIN), 1);
        Some(GuestTicks::new(ticks.max(tick.raw())))
    }

    /// Writes `value` to the thread's decrementer at the time base of
    /// [`Self::registers_at_boundary`], and returns whether the thread took
    /// it. A thread with no supervisor registers takes no write.
    ///
    /// [PPC-Book3 p:80 s:6.3] The DEC counts down at the time-base frequency.
    pub(super) fn write_decrementer_at_boundary(
        &mut self,
        value: u32,
        tick: GuestTicks,
        time_base: u64,
    ) -> bool {
        let tb = self.boundary_read_time_base(tick, time_base);
        self.state
            .update_supervisor(|s| {
                s.dec = value;
                s.dec_tb = tb;
            })
            .is_some()
    }

    /// The time base that the thread reads at its next instruction
    /// boundary at `tick`: the latest of these time bases:
    /// - the time base of `tick`;
    /// - the time base that the thread last read;
    /// - `time_base`, the highest time base that any unit read.
    fn boundary_read_time_base(&self, tick: GuestTicks, time_base: u64) -> u64 {
        PPU_TIME_BASE_ORIGIN
            .wrapping_add(cellgov_time::ticks_to_tb(tick.raw()))
            .max(self.state.tb)
            .max(time_base)
    }

    /// The PPE hardware thread the thread's CTRL names: 1 when CT names
    /// thread 1, else 0.
    ///
    /// [CBE-Handbook p:307 s:10.4.4] CTRL bits 0:1 (CT) name the thread that reads CTRL.
    pub(super) fn ctrl_thread(&self) -> u8 {
        u8::from(self.state.ctrl & CTRL_CT_THREAD1 != 0)
    }
}
