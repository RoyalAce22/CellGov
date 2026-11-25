//! The hypercall entry: CellGov's hypervisor answers the `sc` with LEV =
//! 1 of a thread with supervisor registers, in either processor state,
//! and the runtime applies the answer.
//!
//! The answer reaches the guest in two parts:
//! - its memory writes land at dispatch time through
//!   [`Runtime::host_write`];
//! - the caller's next step gets the status in r3 and the outputs from
//!   r4 onward as committed inputs, as with an LV2 call's return. The
//!   thread then steps past `sc`.
//!
//! A refusal answers nothing. The hypervisor keeps it, and the next
//! [`Runtime::step`] stops the run with it.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::ExecutionStepResult;
use cellgov_lv2::hypervisor::{
    cost, HypercallAnswer, HypercallDisposition, HypercallRefusal, HypercallRequest,
};
use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_SEND_MAX, MAX_OUTPUTS};
use cellgov_trace::{HostWriter, TraceRecord, TracedHypercallDisposition, CONSOLE_RECORD_BYTES};

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
        let disposition = self.hypervisor.dispatch(&request);
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
        debug_assert!(
            answer.outputs.len() <= usize::from(MAX_OUTPUTS),
            "hypercall {} answered {} outputs",
            request.number,
            answer.outputs.len()
        );
        let mut outputs = [0u64; MAX_OUTPUTS as usize];
        for (slot, (reg, value)) in outputs.iter_mut().zip((4u8..).zip(&answer.outputs)) {
            *slot = *value;
            self.registry.push_register_write(unit, reg, *value);
        }
        let status = answer.r3;
        if tracing {
            self.trace.record(&TraceRecord::HypercallReturned {
                unit,
                status,
                outputs,
                written: answer.outputs.len().min(outputs.len()) as u8,
                untimed: cost(request.number).is_none(),
                time: self.time,
            });
        }
        self.registry.set_syscall_return(unit, status);
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
            | HypercallRefusal::RepositorySecondValueUnobserved { .. },
        ) => TracedHypercallDisposition::Refused,
    }
}

#[cfg(test)]
#[path = "tests/hypercall_dispatch_tests.rs"]
mod hypercall_dispatch_tests;

#[cfg(test)]
#[path = "tests/hypercall_vector_tests.rs"]
mod hypercall_vector_tests;
