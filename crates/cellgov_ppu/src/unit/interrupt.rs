//! Interrupt delivery for a thread with supervisor registers.
//!
//! Such a thread runs the console's kernel, which handles its own
//! interrupts. The thread takes each interrupt at the kernel's vector:
//! - an interrupt its instruction causes, except a machine check and
//!   `sc` with LEV = 1;
//! - the system call interrupt of `sc` with any other LEV;
//! - the external and decrementer interrupts, between two instructions;
//! - the trace interrupt, after the instruction that it traces.
//!
//! `sc` with LEV = 1 yields a hypercall in either processor state.
//! Interrupt entry ends the batch. The next batch follows the new MSR,
//! whose IR and DR are 0. A thread with no supervisor registers keeps
//! the terminal fault of every interrupt.

use cellgov_effects::Effect;
use cellgov_exec::{
    CodeEvent, ExecutionContext, ExecutionStepResult, InterruptRecord, LocalDiagnostics,
    YieldReason,
};
use cellgov_ps3_abi::hw::ppu_interrupt::{
    SRR1_INTERRUPT_INFO, SRR1_MEDIATED_EXTERNAL, SRR1_TRACE, VECTOR_DECREMENTER, VECTOR_EXTERNAL,
    VECTOR_TRACE,
};
use cellgov_ps3_abi::hw::ppu_supervisor::{
    MSR_BE, MSR_EE, MSR_HV, MSR_INTERRUPT_CLEARED, MSR_PR, MSR_SE, MSR_SF,
};

use super::batch::BatchEntry;
use super::ppu_unit::PpuExecutionUnit;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::instruction::PpuInstruction;

/// The MSR at the vector of an interrupt taken with MSR `msr`.
///
/// CellGov declares LPCR\[LPES\] = 0b11 for the interrupts the kernel
/// takes at its own vectors. So such an interrupt does not change
/// MSR\[HV\]. ME does not change either. A mediated external interrupt
/// differs: on the console it enters lv1 (LPES bit 0 = 0), and lv1
/// passes it to the kernel's external vector. CellGov models only that
/// second step, with the SRR1 bits that lv1 passes to the kernel.
///
/// [CBE-Handbook p:292 s:9.12] Table 9-40: an interrupt clears BE, EE, FE0, FE1, FP, PR, SE, DR, IR and RI, sets SF, keeps ME, and keeps HV when LPCR\[LPES\] bits 0 and 1 are 1.
fn entry_msr(msr: u64) -> u64 {
    (msr & !MSR_INTERRUPT_CLEARED) | MSR_SF
}

/// The SRR1 of an interrupt taken with MSR `msr`, whose own information
/// is `cause`.
///
/// [PPC-Book3 p:62 s:5.4] SRR1 bits 33:36 and 42:47 hold the interrupt's information, and every other bit holds the MSR.
pub(super) fn entry_srr1(msr: u64, cause: u64) -> u64 {
    (msr & !SRR1_INTERRUPT_INFO) | (cause & SRR1_INTERRUPT_INFO)
}

/// Whether a thread with MSR `msr` may take a mediated external
/// interrupt.
///
/// [CBE-Handbook p:276 s:9.7] Mediated external interrupts are enabled when MSR\[EE\] & (^(MSR\[HV\]) | MSR\[PR\]) is 1.
pub(super) fn mediated_enabled(msr: u64) -> bool {
    msr & MSR_EE != 0 && (msr & MSR_HV == 0 || msr & MSR_PR != 0)
}

/// The SRR0 of the trace interrupt that `insn` at `pc` causes, or `None`.
///
/// `insn` completed with no other interrupt. `msr_before` and `msr_after`
/// are the MSR before and after `insn`. The thread runs `next_pc` next.
///
/// [PPC-Book3 p:73 s:5.5.14] With MSR\[SE\] = 1 every instruction that completes successfully, except rfid and hrfid, causes a trace interrupt, and with MSR\[BE\] = 1 every completed branch; SRR0 is the next instruction.
/// [CBE-Handbook p:257 s:9.5.14] The PPE uses MSR\[SE\] after an mtmsrd, and an mtmsrd that changes SE from 0 to 1 sets SRR0 to the mtmsrd.
pub(super) fn trace_srr0(
    insn: &PpuInstruction,
    pc: u64,
    next_pc: u64,
    msr_before: u64,
    msr_after: u64,
) -> Option<u64> {
    let (rfid_or_hrfid, mtmsrd) = match *insn {
        PpuInstruction::PpePrivileged { raw } => {
            let form = (raw >> 26, (raw >> 1) & 0x3FF);
            (matches!(form, (19, 18 | 274)), form == (31, 178))
        }
        _ => (false, false),
    };
    if rfid_or_hrfid {
        return None;
    }
    let single_step = msr_after & MSR_SE != 0;
    let branch = msr_after & MSR_BE != 0 && insn.is_branch();
    if !single_step && !branch {
        return None;
    }
    let sets_se = mtmsrd && msr_before & MSR_SE == 0 && single_step;
    Some(if sets_se { pc } else { next_pc })
}

/// An interrupt that no instruction causes, which the thread takes
/// between two instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BoundaryInterrupt {
    /// An external exception exists.
    External,
    /// A mediated external exception exists: CellGov's hypervisor set
    /// LPCR\[MER\] for the thread.
    Mediated,
    /// The decrementer is negative.
    Decrementer,
}

impl BoundaryInterrupt {
    /// The interrupt registers when the thread takes this interrupt
    /// ahead of the instruction at `pc`.
    ///
    /// [PPC-Book3 p:69 s:5.5.7] An external interrupt sets SRR0 to the instruction the processor would execute next, and SRR1 bits 33:36 and 42:47 to 0.
    /// [CBE-Handbook p:255 s:9.5.11] A decrementer interrupt sets SRR0 to the instruction the processor would execute next.
    ///
    /// A mediated external interrupt enters lv1, which passes it to the
    /// kernel's own external vector with SRR0 = the interrupted address
    /// and SRR1 = lv1's saved HSRR1. That SRR1 holds bits 33 and 42.
    ///
    /// [OFW-4.93 lv1 a:0x18AC-0x18C0] lv1 sets the guest's SRR0 and SRR1 from the saved HSRR0 and HSRR1 and enters the vector at +496 of the thread's context.
    fn record(self, pc: u64) -> InterruptRecord {
        InterruptRecord {
            vector: match self {
                Self::External | Self::Mediated => VECTOR_EXTERNAL,
                Self::Decrementer => VECTOR_DECREMENTER,
            },
            srr0: pc,
            srr1: match self {
                Self::Mediated => SRR1_MEDIATED_EXTERNAL,
                Self::External | Self::Decrementer => 0,
            },
            dsisr: None,
            dar: None,
        }
    }
}

impl PpuExecutionUnit {
    /// The interrupt the thread takes at the kernel's vector for `fault`,
    /// or `None` when `fault` ends the thread.
    ///
    /// These faults end the thread:
    /// - every fault of a thread with no supervisor registers;
    /// - a CellGov stop;
    /// - a machine check: the 4.93 kernel's machine check vector loops on
    ///   itself, so CellGov keeps the stop.
    ///
    /// `sc` with LEV = 1 reaches here only from a thread with no
    /// supervisor registers. A thread with them yields a hypercall in
    /// either processor state.
    pub(super) fn delivered_interrupt(&self, fault: &PpuFault) -> Option<PpuInterrupt> {
        if !self.translates() {
            return None;
        }
        match fault.interrupt()? {
            PpuInterrupt::MachineCheck | PpuInterrupt::SystemCall { .. } => None,
            interrupt => Some(*interrupt),
        }
    }

    /// Takes `interrupt`, which the instruction at `pc` caused, at its
    /// vector, and yields at the interrupt boundary.
    ///
    /// The instruction is the first of its batch. The unit restores the
    /// state at batch entry, so the instruction leaves no state. The
    /// reservation stays. The instruction's only effects are:
    /// - the page table reads and the R and C writes of its table searches;
    /// - the time base read of the boundary test.
    ///
    /// [PPC-Book3 p:63 s:5.4] Interrupts do not clear reservations.
    pub(super) fn interrupt_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        interrupt: &PpuInterrupt,
    ) -> ExecutionStepResult {
        debug_assert_eq!(
            self.retirement_counter, entry.retired,
            "a retired instruction would be rewound"
        );
        // The boundary test ahead of the instruction read the time base,
        // and the read stays.
        let clock = (self.state.clock_read, self.state.tb);
        self.rewind_batch(entry, effects);
        (self.state.clock_read, self.state.tb) = clock;
        self.hand_back_break_skip(pc);
        self.note_entry_context_sync(entry);
        let record = interrupt.record(pc);
        self.boundary_yield(entry, effects, pc, record)
    }

    /// Takes the system call interrupt of the `sc` at `pc`, which
    /// retired with LEV `lev`, and yields at the interrupt boundary.
    ///
    /// The kernel runs the call in its own instructions, so the issue
    /// model charges no LV2 round trip.
    pub(super) fn system_call_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        lev: u8,
    ) -> ExecutionStepResult {
        self.timing.drop_pending_syscall();
        let record = PpuInterrupt::SystemCall { lev }.record(pc);
        self.boundary_yield(entry, effects, pc, record)
    }

    /// The interrupt the thread takes ahead of the instruction at `pc`,
    /// or `None`.
    ///
    /// The external and decrementer exceptions wait while MSR\[EE\] = 0.
    /// A move of the MSR ends the batch. After a move that sets EE, the
    /// thread takes a waiting exception ahead of the first instruction of
    /// the next batch.
    ///
    /// When both exceptions exist, the thread takes the external one
    /// first, as the PPE does. A mediated external exception also waits
    /// while the thread is in hypervisor state with MSR\[PR\] = 0. CellGov
    /// takes a direct external exception before a mediated one; the PPE
    /// may take either. A thread that waits for the decrementer
    /// does not wake at its deadline: CellGov's hypervisor owns that wait.
    ///
    /// [PPC-Book3 p:76 s:5.8] External, decrementer and hypervisor decrementer exceptions have equal priority, and the processor may take any one of them.
    /// [CBE-Handbook p:291 s:9.10] Table 9-39: the PPE ranks the external interrupt above the decrementer interrupt.
    /// [PPC-Book3 p:81 s:6.3] A decrementer exception exists while DEC bit 0 is 1.
    /// [CBE-Handbook p:276 s:9.7] Mediated external interrupts are enabled when MSR\[EE\] & (^(MSR\[HV\]) | MSR\[PR\]) is 1, and there is no relative priority between direct and mediated external exceptions.
    pub(super) fn boundary_interrupt(
        &mut self,
        ctx: &ExecutionContext<'_>,
        entry: &BatchEntry,
        pc: u64,
    ) -> Option<BoundaryInterrupt> {
        let sup = self.state.supervisor()?;
        if sup.msr & MSR_EE == 0 {
            return None;
        }
        if sup.external {
            return Some(BoundaryInterrupt::External);
        }
        if sup.mediated && mediated_enabled(sup.msr) {
            return Some(BoundaryInterrupt::Mediated);
        }
        let tb = self.boundary_time_base(ctx, entry, pc);
        // The test reads the time base, so the step publishes a clock read.
        self.state.tb = tb;
        self.state.clock_read = true;
        let dec = self.state.supervisor()?.dec_at(tb);
        ((dec as i32) < 0).then_some(BoundaryInterrupt::Decrementer)
    }

    /// Takes `interrupt` ahead of the instruction at `pc`, and yields at
    /// the interrupt boundary. The instructions the batch retired commit.
    pub(super) fn boundary_interrupt_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        interrupt: BoundaryInterrupt,
    ) -> ExecutionStepResult {
        self.note_entry_context_sync(entry);
        self.boundary_yield(entry, effects, pc, interrupt.record(pc))
    }

    /// Takes the trace interrupt of the instruction at `pc`, and yields at
    /// the interrupt boundary.
    ///
    /// The interrupt sets SRR0 to `srr0`. The instruction and the
    /// instructions the batch retired before it commit.
    ///
    /// [CBE-Handbook p:291 s:9.10] Table 9-39: the PPE ranks the trace interrupt above the external and decrementer interrupts.
    pub(super) fn trace_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        srr0: u64,
    ) -> ExecutionStepResult {
        self.note_entry_context_sync(entry);
        let record = InterruptRecord {
            vector: VECTOR_TRACE,
            srr0,
            srr1: SRR1_TRACE,
            dsisr: None,
            dar: None,
        };
        self.boundary_yield(entry, effects, pc, record)
    }

    /// Records the context synchronization of interrupt entry, when the
    /// batch records code events. The `sc` of a system call interrupt
    /// records its own.
    ///
    /// [PPC-Book3 p:61 s:5.2] Every interrupt except system reset and machine check is context synchronizing.
    fn note_entry_context_sync(&mut self, entry: &BatchEntry) {
        if entry.trace {
            self.code_events.push(CodeEvent::ContextSync);
        }
    }

    /// The time base that an `mftb` at `pc` reads.
    fn boundary_time_base(&self, ctx: &ExecutionContext<'_>, entry: &BatchEntry, pc: u64) -> u64 {
        let (ops, _) = crate::timing::classify(&PpuInstruction::Mftb { rt: 0 }, &self.state);
        self.time_base_at_issue(ctx, entry, pc, &ops[0])
    }

    /// Sets the interrupt registers from `record`, whose `srr1` holds
    /// the cause bits only, and moves the thread to the vector. Returns
    /// the record with the whole SRR1.
    fn enter_interrupt(&mut self, mut record: InterruptRecord) -> InterruptRecord {
        let Some(msr) = self.state.supervisor().map(|s| s.msr) else {
            debug_assert!(
                false,
                "interrupt entry on a thread with no supervisor registers"
            );
            return record;
        };
        record.srr1 = entry_srr1(msr, record.srr1);
        self.state.update_supervisor(|s| {
            s.srr0 = record.srr0;
            s.srr1 = record.srr1;
            if let Some(dar) = record.dar {
                s.dar = dar;
            }
            if let Some(dsisr) = record.dsisr {
                s.dsisr = dsisr;
            }
            s.msr = entry_msr(msr);
        });
        // With IR = 0 the fetch at the vector goes through the real mode
        // offset.
        self.state.pc = record.vector;
        record
    }

    /// Publishes the block, takes the interrupt `record` names, and
    /// yields at the interrupt boundary with the whole interrupt record.
    fn boundary_yield(
        &mut self,
        entry: &BatchEntry,
        effects: &mut Vec<Effect>,
        pc: u64,
        record: InterruptRecord,
    ) -> ExecutionStepResult {
        self.close_block(effects);
        let record = self.enter_interrupt(record);
        ExecutionStepResult {
            yield_reason: YieldReason::InterruptBoundary,
            consumed_cost: self.cost(entry),
            local_diagnostics: LocalDiagnostics {
                pc: Some(pc),
                lr: Some(self.state.lr()),
                syscall_lev: None,
                faulting_ea: None,
                fault_regs: None,
                data_access: None,
                interrupt: Some(record),
            },
            fault: None,
            syscall_args: None,
        }
    }

    /// Hands back a break skip spent at `pc`, whose instruction did not
    /// retire and runs again after the interrupt handler returns.
    fn hand_back_break_skip(&mut self, pc: u64) {
        if self.break_pc == Some(pc) {
            self.break_skip += 1;
        }
    }

    /// Raises an external exception for the thread. Returns `false`, and
    /// changes nothing, when the thread has no supervisor registers.
    ///
    /// The exception exists until [`Self::clear_external_exception`].
    #[must_use]
    pub fn raise_external_exception(&mut self) -> bool {
        self.state
            .update_supervisor(|s| s.external = true)
            .is_some()
    }

    /// Clears the thread's external exception. Returns `false`, and
    /// changes nothing, when the thread has no supervisor registers.
    #[must_use]
    pub fn clear_external_exception(&mut self) -> bool {
        self.state
            .update_supervisor(|s| s.external = false)
            .is_some()
    }

    /// Raises a mediated external exception for the thread, as the
    /// hypervisor's LPCR\[MER\] = 1 does. Returns `false`, and changes
    /// nothing, when the thread has no supervisor registers.
    ///
    /// The exception exists until
    /// [`Self::clear_mediated_external_exception`].
    #[must_use]
    pub fn raise_mediated_external_exception(&mut self) -> bool {
        self.state
            .update_supervisor(|s| s.mediated = true)
            .is_some()
    }

    /// Clears the thread's mediated external exception, as the
    /// hypervisor's LPCR\[MER\] = 0 does. Returns `false`, and changes
    /// nothing, when the thread has no supervisor registers.
    #[must_use]
    pub fn clear_mediated_external_exception(&mut self) -> bool {
        self.state
            .update_supervisor(|s| s.mediated = false)
            .is_some()
    }
}

#[cfg(test)]
#[path = "tests/interrupt_entry_tests.rs"]
mod interrupt_entry_tests;

#[cfg(test)]
#[path = "tests/interrupt_support.rs"]
mod interrupt_support;

#[cfg(test)]
#[path = "tests/interrupt_class_tests.rs"]
mod interrupt_class_tests;

#[cfg(test)]
#[path = "tests/interrupt_bp_vr_tests.rs"]
mod interrupt_bp_vr_tests;

#[cfg(test)]
#[path = "tests/interrupt_hypervisor_spr_tests.rs"]
mod interrupt_hypervisor_spr_tests;

#[cfg(test)]
#[path = "tests/interrupt_batch_tests.rs"]
mod interrupt_batch_tests;

#[cfg(test)]
#[path = "tests/interrupt_boundary_tests.rs"]
mod interrupt_boundary_tests;

#[cfg(test)]
#[path = "tests/interrupt_mediated_tests.rs"]
mod interrupt_mediated_tests;

#[cfg(test)]
#[path = "tests/interrupt_system_call_tests.rs"]
mod interrupt_system_call_tests;

#[cfg(test)]
#[path = "tests/interrupt_code_event_tests.rs"]
mod interrupt_code_event_tests;

#[cfg(test)]
#[path = "tests/hypercall_yield_tests.rs"]
mod hypercall_yield_tests;

#[cfg(test)]
#[path = "tests/hypercall_return_tests.rs"]
mod hypercall_return_tests;

#[cfg(test)]
#[path = "tests/interrupt_facility_tests.rs"]
mod interrupt_facility_tests;

#[cfg(test)]
#[path = "tests/interrupt_trace_tests.rs"]
mod interrupt_trace_tests;

#[cfg(test)]
#[path = "tests/interrupt_endian_tests.rs"]
mod interrupt_endian_tests;
