//! The hypercall entry: CellGov's hypervisor answers the `sc` with LEV =
//! 1 of a thread with supervisor registers, in either processor state,
//! and the runtime applies the answer.
//!
//! The hypervisor reads the page table in the caller's address space.
//! The answer reaches the guest in three parts:
//! - its memory writes land at dispatch time through
//!   [`Runtime::host_write`];
//! - a page table load reaches the caller, and every other unit with
//!   supervisor registers, at dispatch time;
//! - the caller's next step gets these committed inputs, as with an LV2
//!   call's return, and then steps past `sc`:
//!   - the status in r3 and the values for r4 to r10;
//!   - the r0, r11, r12 and CR fields that lv1's return leaves.
//!
//! An answer that pauses the caller also parks it. The caller takes its
//! committed inputs at its first step after a wake makes it runnable. An
//! answer that wakes a paused thread makes that thread's unit runnable.
//! An answer that sends a running thread an interprocessor interrupt
//! gives that thread the exception of lv1's handler before its next step.
//!
//! The hypervisor tests each return for an external interrupt, as lv1's
//! return does. The runtime raises one of two exceptions of the thread:
//! - the external exception, for a return that delivers the interrupt;
//! - the mediated external exception, for a return that sets LPCR\[MER\].
//!
//! The runtime clears both when the thread takes the interrupt. A later
//! return that gives outputs and raises nothing clears them too. The
//! hypervisor tests each take of the mediated one, as lv1's handler does.
//!
//! A refusal answers nothing. The hypervisor keeps it, and the next
//! [`Runtime::step`] stops the run with it.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{CompareFields, CompareResult, ExecutionStepResult, UnitStatus};
use cellgov_lv1::{
    cost, Compare, ExternalRaise, HypercallAnswer, HypercallDisposition, HypercallRefusal,
    HypercallRequest, MediatedTake, PageTableLoad, ReturnPair, RunningThread, RunningThreads,
    SavedMsrs, ThreadInterrupt, ThreadResume,
};
use cellgov_ps3_abi::hw::ppu_interrupt::{SRR1_MEDIATED_EXTERNAL, VECTOR_EXTERNAL};
use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_SEND_MAX, MAX_OUTPUTS};
use cellgov_time::GuestTicks;
use cellgov_trace::{HostWriter, TraceRecord, TracedHypercallDisposition, CONSOLE_RECORD_BYTES};

use crate::registry::UnitRegistry;
use crate::runtime::types::RuntimeMode;
use crate::runtime::Runtime;

// Each console send fits the console record that traces it.
const _: () = assert!(LOGICAL_CONSOLE_SEND_MAX <= CONSOLE_RECORD_BYTES);

impl Runtime {
    /// Answers the hypercall `source` yielded in `result`, as CellGov's
    /// hypervisor, and traces its entry.
    pub(in crate::runtime) fn dispatch_hypercall(
        &mut self,
        result: &ExecutionStepResult,
        source: UnitId,
    ) {
        let Some(args) = result.syscall_args else {
            return;
        };
        // Only `LocalDiagnostics::with_pc_lr_syscall_lev` sets a LEV, and it sets the pc and lr too.
        debug_assert!(
            result.local_diagnostics.pc.is_some(),
            "hypercall yield without a pc"
        );
        let mut inputs = [0u64; 8];
        inputs.copy_from_slice(&args[1..]);
        let request = HypercallRequest {
            unit: source,
            number: args[0],
            inputs,
            pc: result.local_diagnostics.pc.unwrap_or_default(),
            lr: result.local_diagnostics.lr.unwrap_or_default(),
            thread: self.registry.get(source).and_then(|u| u.ppe_thread()),
            problem_state: self
                .registry
                .get(source)
                .is_some_and(|u| u.in_problem_state()),
            tick: self.time,
        };
        let space = self.spaces.space_of(source);
        let storage =
            crate::runtime::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        let msrs = saved_msrs(&self.registry);
        let running = running_threads(&self.registry, request.tick);
        let disposition = self
            .hypervisor
            .dispatch_signalling(&request, storage, &msrs, &running);
        if self.mode != RuntimeMode::FaultDriven {
            self.trace.record(&TraceRecord::HypercallEntered {
                unit: source,
                number: request.number,
                args: request.inputs,
                pc: request.pc,
                thread: request.thread,
                disposition: traced_disposition(&disposition),
                time: request.tick,
            });
        }
        if let HypercallDisposition::Answered(answer) = disposition {
            self.apply_hypercall_answer(&request, answer);
        }
    }

    /// Applies `answer` to the caller of `request`.
    fn apply_hypercall_answer(&mut self, request: &HypercallRequest, answer: HypercallAnswer) {
        let unit = request.unit;
        let tracing = self.mode != RuntimeMode::FaultDriven;
        if let (true, Some(write)) = (tracing, &answer.console) {
            let mut bytes = [0u8; CONSOLE_RECORD_BYTES];
            bytes[..write.bytes().len()].copy_from_slice(write.bytes());
            self.trace.record(&TraceRecord::HypervisorConsole {
                unit,
                console: write.console,
                len: write.len,
                bytes,
            });
        }
        self.apply_hypervisor_writes(unit, &answer.effects);
        if let Some(load) = answer.page_table {
            self.load_page_table(unit, load);
        }
        let written = match answer.outputs() {
            Some(outputs) => {
                for (reg, value) in (4u8..).zip(outputs) {
                    self.registry.push_register_write(unit, reg, value);
                }
                MAX_OUTPUTS
            }
            None => 0,
        };
        self.apply_return_registers(request, &answer);
        let status = answer.r3;
        if tracing {
            self.trace.record(&TraceRecord::HypercallReturned {
                unit,
                status,
                outputs: answer.outputs().unwrap_or_default(),
                written,
                untimed: cost(request.number).is_none(),
                time: self.time,
            });
        }
        self.registry.set_syscall_return(unit, status);
        if answer.pauses_caller {
            self.registry
                .set_status_override(unit, cellgov_exec::UnitStatus::Blocked);
        } else if answer.outputs().is_some() {
            self.set_external_exception(unit, answer.external);
        }
        for resume in &answer.resumed {
            self.resume_paused_thread(resume);
        }
        for interrupt in &answer.interrupted {
            self.interrupt_running_thread(interrupt);
        }
    }

    /// The unit that runs the PPE hardware thread `thread`, or `None`.
    fn unit_of_thread(&self, thread: u8) -> Option<UnitId> {
        self.registry
            .iter()
            .find(|(_, u)| u.ppe_thread() == Some(thread))
            .map(|(id, _)| id)
    }

    /// Gives the running thread of `interrupt` the exception that lv1's
    /// handler raises when the thread takes the interprocessor interrupt.
    ///
    /// The thread takes the interrupt at its next instruction boundary:
    /// the first instruction of its next step. The handler reloads the
    /// LPCR, which clears LPCR\[MER\] [OFW-4.93 lv1 a:0x1A40]
    /// [OFW-4.93 lv1 a:0x1A5C]. So this first clears the mediated external
    /// exception only. An external exception that an earlier interrupt
    /// raised, and that the thread has not yet taken, stays: lv1 entered
    /// the vector for it, and the reload clears no such entry.
    fn interrupt_running_thread(&mut self, interrupt: &ThreadInterrupt) {
        let Some(unit) = self.unit_of_thread(interrupt.thread) else {
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_signalled_thread_missing",
                format_args!("no unit runs the signalled PPE thread {}", interrupt.thread),
            );
            return;
        };
        if let Some(registered) = self.registry.get_mut(unit) {
            // A unit with no supervisor registers holds no exception.
            let _ = registered.clear_mediated_external_exception();
        }
        self.raise_exception_on(unit, interrupt.external);
    }

    /// Wakes the paused thread of `resume`, as lv1's wake ends its pause.
    ///
    /// The thread took the rest of its return when it paused. This call
    /// gives that return the external interrupt that lv1's return raises.
    /// A raise replaces r11 and r12 with the vector and the new MSR, as lv1
    /// loads them after the raise. The runtime gives a unit's register
    /// writes in order, so these writes win.
    fn resume_paused_thread(&mut self, resume: &ThreadResume) {
        let Some(unit) = self.unit_of_thread(resume.thread) else {
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_woken_thread_missing",
                format_args!("no unit runs the woken PPE thread {}", resume.thread),
            );
            return;
        };
        self.registry.clear_status_override(unit);
        // The wake makes another unit runnable, which the scheduler's
        // notice of this step reports.
        self.step_woke_others = true;
        if let Some(delivery) = resume.external.and_then(ExternalRaise::delivery) {
            self.registry.push_register_write(unit, 11, delivery.vector);
            self.registry.push_register_write(unit, 12, delivery.msr);
        }
        self.set_external_exception(unit, resume.external);
    }

    /// Gives `unit` the exception of `raise`, as lv1's return to the
    /// thread reloads the LPCR [OFW-4.93 lv1 a:0x1008] and then raises
    /// the external interrupt.
    ///
    /// The reload clears LPCR\[MER\], so the return first clears both
    /// exceptions. Then it raises one of them:
    /// - the external exception, for a delivery;
    /// - the mediated external exception, for LPCR\[MER\]
    ///   [OFW-4.93 lv1 a:0x191C-0x1928].
    ///
    /// The exception exists until the thread takes the interrupt; see
    /// [`Runtime::end_external_interrupt`]. If the unit takes no exception,
    /// the runtime logs an invariant break and the interrupt is lost.
    ///
    /// # Panics
    ///
    /// Panics in a debug build where the unit takes no raised exception.
    fn set_external_exception(&mut self, unit: UnitId, raise: Option<ExternalRaise>) {
        let Some(registered) = self.registry.get_mut(unit) else {
            return;
        };
        // A unit with no supervisor registers holds no exception.
        let _ = registered.clear_external_exception();
        let _ = registered.clear_mediated_external_exception();
        self.raise_exception_on(unit, raise);
    }

    /// Gives `unit` the exception of `raise`, and clears none.
    ///
    /// If the unit takes no exception, the runtime logs an invariant break
    /// and the interrupt is lost.
    ///
    /// # Panics
    ///
    /// Panics in a debug build where the unit takes no raised exception.
    fn raise_exception_on(&mut self, unit: UnitId, raise: Option<ExternalRaise>) {
        let Some(registered) = self.registry.get_mut(unit) else {
            return;
        };
        let raised = match raise {
            None => return,
            Some(ExternalRaise::Deliver(_)) => registered.raise_external_exception(),
            Some(ExternalRaise::Mediated) => registered.raise_mediated_external_exception(),
        };
        if !raised {
            debug_assert!(false, "{unit:?} took no external exception");
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_external_exception_refused",
                format_args!("{unit:?} took no external exception, so its interrupt is lost"),
            );
        }
    }

    /// Clears both external exceptions of `source` when its step `result`
    /// took the external interrupt, and has the hypervisor test the take
    /// of a mediated one.
    ///
    /// lv1 raises the interrupt once for each return that finds a shared
    /// bit, and keeps no record of it. The next return tests the shadow
    /// again; see [`cellgov_ps3_abi::lv1::interrupt_mask`]. lv1's handler
    /// of a mediated take reloads the LPCR, which clears LPCR\[MER\]
    /// [OFW-4.93 lv1 a:0x1A5C].
    ///
    /// The take of a mediated interrupt sets [`SRR1_MEDIATED_EXTERNAL`] in
    /// SRR1. A take that the hypervisor refuses stops the run at the next
    /// step, after the thread entered the vector.
    pub(in crate::runtime) fn end_external_interrupt(
        &mut self,
        result: &ExecutionStepResult,
        source: UnitId,
    ) {
        let Some(interrupt) = result
            .local_diagnostics
            .interrupt
            .filter(|interrupt| interrupt.vector == VECTOR_EXTERNAL)
        else {
            return;
        };
        let Some(registered) = self.registry.get_mut(source) else {
            return;
        };
        // A unit with no supervisor registers takes no interrupt.
        let _ = registered.clear_external_exception();
        let _ = registered.clear_mediated_external_exception();
        if interrupt.srr1 & SRR1_MEDIATED_EXTERNAL != SRR1_MEDIATED_EXTERNAL {
            return;
        }
        let take = MediatedTake {
            unit: source,
            thread: registered.ppe_thread(),
            srr0: interrupt.srr0,
            srr1: interrupt.srr1,
        };
        let space = self.spaces.space_of(source);
        let storage =
            crate::runtime::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        // The hypervisor keeps a refusal, and the next step stops the run
        // with it.
        let _ = self.hypervisor.take_mediated_external(&take, storage);
    }

    /// Gives the caller of `request` the r0, r11, r12 and CR fields that
    /// lv1's return for `answer` leaves.
    ///
    /// The return through `hrfid` gives r11 and r12 the SRR0 and SRR1 of
    /// the caller's `sc`. A unit with no supervisor registers gives none:
    /// the runtime then logs an invariant break, and r11 and r12 keep
    /// their values.
    ///
    /// # Panics
    ///
    /// Panics in a debug build where the unit gives no SRR0 and SRR1.
    fn apply_return_registers(&mut self, request: &HypercallRequest, answer: &HypercallAnswer) {
        let unit = request.unit;
        let returned = answer.return_registers(request, self.hypervisor.firmware());
        self.registry.push_register_write(unit, 0, returned.r0);
        let pair = match returned.r11_r12 {
            ReturnPair::Values { r11, r12 } => Some((r11, r12)),
            ReturnPair::SaveRestore => self
                .registry
                .get(unit)
                .and_then(|u| u.system_call_save_restore())
                .map(|s| (s.srr0, s.srr1)),
        };
        if let Some((r11, r12)) = pair {
            self.registry.push_register_write(unit, 11, r11);
            self.registry.push_register_write(unit, 12, r12);
        } else {
            debug_assert!(false, "{unit:?} gives no SRR0 and SRR1 for its hypercall");
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_save_restore_missing",
                format_args!("{unit:?} gives no SRR0 and SRR1 for its hypercall"),
            );
        }
        self.registry
            .set_compare_fields(unit, compare_fields(&returned.cr));
    }

    /// Gives `unit` the page table and large page sizes of `load`, as
    /// lv1 loads them on the calling thread, and gives them to every
    /// other unit with supervisor registers too.
    ///
    /// The hypervisor loads only values that name its page table storage,
    /// which a thread with supervisor registers takes. A unit with no
    /// supervisor registers can also reach the hypervisor through `sc`
    /// with LEV = 1, and it refuses them. The runtime then logs an
    /// invariant break, and the hypervisor's selection and the caller's
    /// r3 stand. Another unit that refuses the values keeps its own.
    ///
    /// [CBE-Handbook p:297 s:10.2] The two PPE threads run in one logical-partition context and share its virtual-memory-mapping tables.
    ///
    /// # Panics
    ///
    /// Panics in a debug build where the calling unit refuses the values.
    fn load_page_table(&mut self, unit: UnitId, load: PageTableLoad) {
        let loaded = self
            .registry
            .get_mut(unit)
            .is_some_and(|u| u.load_page_table(load.sdr1, load.lb));
        if !loaded {
            debug_assert!(false, "{unit:?} refused the page table load {load:?}");
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_page_table_refused",
                format_args!("{unit:?} refused the page table load {load:?}"),
            );
            return;
        }
        for (_, other) in self.registry.iter_mut().filter(|(id, _)| *id != unit) {
            // A unit with no supervisor registers refuses, and keeps its state.
            let _ = other.load_page_table(load.sdr1, load.lb);
        }
    }

    /// Lands the memory writes of a hypercall answer for `unit`, all or
    /// none, at their real addresses in `unit`'s space.
    ///
    /// A handler emits only `SharedWriteIntent`. On another effect, or on
    /// a write the space refuses, this drops every write and logs an
    /// invariant break.
    ///
    /// # Panics
    ///
    /// Panics in a debug build where it drops the writes.
    fn apply_hypervisor_writes(&mut self, unit: UnitId, effects: &[Effect]) {
        if effects.is_empty() {
            return;
        }
        let space = self.spaces.space_of(unit);
        let mem = crate::runtime::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        let refused = effects.iter().find(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                mem.validate_write(*range, bytes.len()).is_err()
            }
            _ => true,
        });
        if let Some(effect) = refused {
            debug_assert!(false, "hypervisor answer carries {effect:?}");
            self.lv2_host.log_invariant_break(
                "dispatch.hypervisor_write_refused",
                format_args!(
                    "a hypercall answer for {unit:?} carries {effect:?}, which the space \
                     refuses or no handler emits; none of its writes land"
                ),
            );
            return;
        }
        for effect in effects {
            let Effect::SharedWriteIntent { range, bytes, .. } = effect else {
                continue;
            };
            let written = self.host_write(
                HostWriter::Hypervisor,
                space,
                *range,
                bytes.bytes(),
                Some(unit),
            );
            if let Err(err) = written {
                self.lv2_host.log_invariant_break(
                    "dispatch.hypervisor_write_failed",
                    format_args!(
                        "a validated hypercall write for {unit:?} at {:#x} failed: {err}",
                        range.start().raw()
                    ),
                );
            }
        }
    }
}

/// The SRR1 that each PPE hardware thread's `sc` sets, as lv1 saves it.
///
/// A thread whose unit holds no supervisor registers keeps a `None` slot.
fn saved_msrs(registry: &UnitRegistry) -> SavedMsrs {
    let mut msrs = SavedMsrs::default();
    for (_, unit) in registry.iter() {
        let (Some(thread), Some(save)) = (unit.ppe_thread(), unit.system_call_save_restore())
        else {
            continue;
        };
        if let Some(slot) = msrs.0.get_mut(usize::from(thread)) {
            *slot = Some(save.srr1);
        }
    }
    msrs
}

/// The registers of each PPE hardware thread whose unit runs at an
/// instruction boundary, with the decrementer at the guest tick `tick`
/// of the call, or at the unit's own clock where that one is later.
///
/// The unit's pc and MSR are those at the start of its next step, at its
/// own clock. A unit whose clock is past `tick` reads its decrementer at
/// that clock, as its own boundary test does at the step's start tick.
///
/// A thread gets no slot in these cases:
/// - its unit is not runnable;
/// - its unit holds a syscall return, so its next step steps past `sc`;
/// - its unit holds no supervisor registers.
fn running_threads(registry: &UnitRegistry, tick: GuestTicks) -> RunningThreads {
    let mut running = RunningThreads::default();
    for (id, unit) in registry.iter() {
        let runnable = registry.effective_status(id) == Some(UnitStatus::Runnable);
        if !runnable || registry.has_syscall_return(id) {
            continue;
        }
        let at = tick.max(registry.clock(id));
        let (Some(thread), Some(regs)) = (unit.ppe_thread(), unit.boundary_registers(at)) else {
            continue;
        };
        if let Some(slot) = running.0.get_mut(usize::from(thread)) {
            *slot = Some(RunningThread {
                pc: regs.pc,
                msr: regs.msr,
                decrementer: regs.dec,
            });
        }
    }
    running
}

/// The CR fields of `cr`, as the caller's next step takes them.
fn compare_fields(cr: &[Option<Compare>; 8]) -> CompareFields {
    CompareFields(cr.map(|field| {
        field.map(|result| match result {
            Compare::Less => CompareResult::Less,
            Compare::Greater => CompareResult::Greater,
            Compare::Equal => CompareResult::Equal,
        })
    }))
}

fn traced_disposition(disposition: &HypercallDisposition) -> TracedHypercallDisposition {
    match disposition {
        HypercallDisposition::Answered(_) => TracedHypercallDisposition::Answered,
        HypercallDisposition::Refused(HypercallRefusal::Unmodelled { .. }) => {
            TracedHypercallDisposition::Unmodelled
        }
        HypercallDisposition::Refused(
            HypercallRefusal::ProblemState { .. }
            | HypercallRefusal::UnmodelledInput { .. }
            | HypercallRefusal::RepositoryNode { .. }
            | HypercallRefusal::RepositorySecondValueUnobserved { .. }
            | HypercallRefusal::SystemManagerRequest { .. }
            | HypercallRefusal::MediatedExternal { .. },
        ) => TracedHypercallDisposition::Refused,
    }
}

#[cfg(test)]
#[path = "tests/hypercall_dispatch_tests.rs"]
mod hypercall_dispatch_tests;

#[cfg(test)]
#[path = "tests/hypercall_vector_tests.rs"]
mod hypercall_vector_tests;

#[cfg(test)]
#[path = "tests/hypercall_return_path_tests.rs"]
mod hypercall_return_path_tests;

#[cfg(test)]
#[path = "tests/hypercall_external_tests.rs"]
mod hypercall_external_tests;

#[cfg(test)]
#[path = "tests/hypercall_signal_tests.rs"]
mod hypercall_signal_tests;
