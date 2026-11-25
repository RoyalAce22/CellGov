//! The hypercall entry: CellGov's hypervisor answers the `sc` with LEV =
//! 1 of a thread with supervisor registers, in either processor state,
//! and the runtime applies the answer.
//!
//! The hypervisor reads the page table in the caller's address space.
//! The answer reaches the guest in three parts:
//! - its memory writes land at dispatch time through
//!   [`Runtime::host_write`];
//! - a page table load reaches the caller at dispatch time;
//! - the caller's next step gets these committed inputs, as with an LV2
//!   call's return, and then steps past `sc`:
//!   - the status in r3 and the values for r4 to r10;
//!   - the r0, r11, r12 and CR fields that lv1's return leaves.
//!
//! A refusal answers nothing. The hypervisor keeps it, and the next
//! [`Runtime::step`] stops the run with it.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{CompareFields, CompareResult, ExecutionStepResult};
use cellgov_lv1::{
    cost, Compare, HypercallAnswer, HypercallDisposition, HypercallRefusal, HypercallRequest,
    PageTableLoad, ReturnPair,
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
        let space = self.spaces.space_of(source);
        let storage =
            crate::runtime::spaces::resolve_space_memory(&self.memory, &self.spaces, space);
        let disposition = self.hypervisor.dispatch_with(&request, storage);
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
    /// lv1 loads them on the calling thread.
    ///
    /// The hypervisor loads only values that name its page table storage,
    /// which a thread with supervisor registers takes. A unit with no
    /// supervisor registers can also reach the hypervisor through `sc`
    /// with LEV = 1, and it refuses them. The runtime then logs an
    /// invariant break, and the hypervisor's selection and the caller's
    /// r3 stand.
    ///
    /// # Panics
    ///
    /// Panics in a debug build where the unit refuses the values.
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

#[cfg(test)]
#[path = "tests/hypercall_return_path_tests.rs"]
mod hypercall_return_path_tests;
