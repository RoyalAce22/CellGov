use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, SaveRestore,
    UnitStatus, YieldReason,
};
use cellgov_lv1::{HypercallRefusal, HypercallWitness};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::{HostWriter, TraceReader, TraceRecord, TracedHypercallDisposition};

use crate::runtime::types::{RuntimeMode, StepError};
use crate::runtime::Runtime;

#[path = "hypercall_return_registers_tests.rs"]
mod return_registers;

#[path = "hypercall_pending_hash_tests.rs"]
mod pending_hash;

/// The address of the scripted `sc`.
const PC: u64 = 0x100;

/// What the caller's step after the hypercall received: r3, and the
/// register writes to r4 to r10.
type Received = Rc<RefCell<Option<(Option<u64>, Vec<(u8, u64)>)>>>;

/// A unit that yields one hypercall, then records what its next step
/// receives and finishes.
#[derive(Clone)]
struct Caller {
    id: UnitId,
    args: [u64; 9],
    thread: Option<u8>,
    steps: u32,
    received: Received,
}

impl ExecutionUnit for Caller {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn ppe_thread(&self) -> Option<u8> {
        self.thread
    }
    fn system_call_save_restore(&self) -> Option<SaveRestore> {
        Some(SaveRestore {
            srr0: PC + 4,
            srr1: 0,
        })
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps += 1;
        if self.steps == 1 {
            return ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(3),
                local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(PC, 0, 1),
                fault: None,
                syscall_args: Some(self.args),
            };
        }
        let outputs = ctx.register_writes().iter().copied();
        let outputs = outputs.filter(|(reg, _)| (4..=10).contains(reg)).collect();
        *self.received.borrow_mut() = Some((ctx.syscall_return(), outputs));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

/// A traced runtime over 0x1000 bytes, with one [`Caller`] of hypercall
/// `number` on PPE hardware thread 0, and what its second step receives.
fn runtime(number: u64, inputs: [u64; 8]) -> (Runtime, Received) {
    runtime_on(number, inputs, Some(0))
}

/// [`runtime`] with the caller on PPE hardware thread `thread`.
fn runtime_on(number: u64, inputs: [u64; 8], thread: Option<u8>) -> (Runtime, Received) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.set_mode(RuntimeMode::FullTrace);
    let received: Received = Rc::new(RefCell::new(None));
    let mut args = [number; 9];
    args[1..].copy_from_slice(&inputs);
    let seen = Rc::clone(&received);
    rt.register_unit_with(move |id| Caller {
        id,
        args,
        thread,
        steps: 0,
        received: seen,
    });
    (rt, received)
}

/// Runs one step and commits it.
fn step(rt: &mut Runtime) -> Result<(), StepError> {
    let mut s = rt.step()?;
    rt.commit_step(&mut s).expect("commit");
    Ok(())
}

fn records(rt: &Runtime) -> Vec<TraceRecord> {
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter(|r| {
            matches!(
                r,
                TraceRecord::HypercallEntered { .. }
                    | TraceRecord::HypercallReturned { .. }
                    | TraceRecord::HypervisorConsole { .. }
                    | TraceRecord::SyscallEntered { .. }
                    | TraceRecord::SyscallReturned { .. }
            )
        })
        .collect()
}

#[test]
fn an_answered_hypercall_hands_r3_and_its_outputs_to_the_callers_next_step() {
    let (mut rt, received) = runtime(74, [0; 8]);
    step(&mut rt).expect("the hypercall step");
    step(&mut rt).expect("the caller's next step");
    let writes = vec![(4, 2), (5, 0), (6, 0), (7, 0), (8, 0), (9, 0), (10, 0)];
    assert_eq!(*received.borrow(), Some((Some(0), writes)));
    let unit = UnitId::new(0);
    assert_eq!(
        records(&rt),
        vec![
            TraceRecord::HypercallEntered {
                unit,
                number: 74,
                args: [0; 8],
                pc: PC,
                thread: Some(0),
                disposition: TracedHypercallDisposition::Answered,
                time: GuestTicks::new(3),
            },
            TraceRecord::HypercallReturned {
                unit,
                status: 0,
                outputs: [2, 0, 0, 0, 0, 0, 0],
                written: 7,
                untimed: true,
                time: GuestTicks::new(3),
            },
        ]
    );
}

#[test]
fn an_unmodelled_hypercall_answers_nothing_and_stops_the_run() {
    // Neither copy of lv1call.h lists 5.
    let (mut rt, received) = runtime_on(5, [1, 2, 3, 4, 5, 6, 7, 8], Some(1));
    step(&mut rt).expect("the hypercall step");
    for _ in 0..2 {
        assert_eq!(step(&mut rt), Err(StepError::HypercallRefused));
    }
    assert_eq!(*received.borrow(), None, "the caller never ran again");
    let unit = UnitId::new(0);
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::Unmodelled {
            number: 5,
            unit,
            pc: PC
        })
    );
    assert_eq!(
        rt.hypervisor().witnesses().get(&5),
        Some(&HypercallWitness {
            hits: 1,
            first_tick: GuestTicks::new(3)
        })
    );
    assert_eq!(
        records(&rt),
        vec![TraceRecord::HypercallEntered {
            unit,
            number: 5,
            args: [1, 2, 3, 4, 5, 6, 7, 8],
            pc: PC,
            thread: Some(1),
            disposition: TracedHypercallDisposition::Unmodelled,
            time: GuestTicks::new(3),
        }]
    );
}

#[test]
fn a_console_send_reaches_the_trace_and_the_sink_and_no_guest_memory() {
    let text = u64::from_be_bytes(*b"lv2(0): ");
    let (mut rt, received) = runtime(107, [1, 5, text, 0, 0, 0, 0, 0]);
    let before = rt.memory().content_hash();
    step(&mut rt).expect("the hypercall step");
    assert_eq!(rt.memory().content_hash(), before);
    step(&mut rt).expect("the caller's next step");
    let writes = vec![(4, 5), (5, 0), (6, 0), (7, 0), (8, 0), (9, 0), (10, 0)];
    assert_eq!(*received.borrow(), Some((Some(0), writes)));
    assert_eq!(rt.hypervisor().console_text(), b"lv2(0");
    let mut bytes = [0u8; 32];
    bytes[..5].copy_from_slice(b"lv2(0");
    let console: Vec<_> = records(&rt)
        .into_iter()
        .filter(|r| matches!(r, TraceRecord::HypervisorConsole { .. }))
        .collect();
    assert_eq!(
        console,
        vec![TraceRecord::HypervisorConsole {
            unit: UnitId::new(0),
            console: 1,
            len: 5,
            bytes,
        }]
    );
}

#[test]
fn a_hypercall_answer_writes_memory_through_the_hypervisor_host_writer() {
    let (mut rt, _) = runtime(74, [0; 8]);
    let unit = UnitId::new(0);
    let write = |addr: u64, byte: u8| Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(addr), 1).expect("range"),
        bytes: WritePayload::from_slice(&[byte]),
        source: unit,
        source_time: GuestTicks::ZERO,
    };
    rt.apply_hypervisor_writes(unit, &[write(0x10, 0xAB), write(0x11, 0xCD)]);
    let range = ByteRange::new(GuestAddr::new(0x10), 2).expect("range");
    assert_eq!(&*rt.memory().read(range).expect("mapped"), &[0xAB, 0xCD]);
    let writers: Vec<_> = TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter_map(|r| match r {
            TraceRecord::HostWrite { writer, addr, .. } => Some((writer, addr)),
            _ => None,
        })
        .collect();
    assert_eq!(
        writers,
        vec![
            (HostWriter::Hypervisor, 0x10),
            (HostWriter::Hypervisor, 0x11)
        ]
    );
}

/// A runtime, its caller, and two one-byte writes: one at 0x10, and one
/// at 0x2000, past the 0x1000 bytes of memory.
fn refused_write_answer() -> (Runtime, UnitId, [Effect; 2]) {
    let (rt, _) = runtime(74, [0; 8]);
    let unit = UnitId::new(0);
    let write = |addr: u64| Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(addr), 1).expect("range"),
        bytes: WritePayload::from_slice(&[0xEE]),
        source: unit,
        source_time: GuestTicks::ZERO,
    };
    (rt, unit, [write(0x10), write(0x2000)])
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "hypervisor answer carries SharedWriteIntent")]
fn a_hypercall_answer_with_a_refused_write_is_trapped_in_debug() {
    let (mut rt, unit, writes) = refused_write_answer();
    rt.apply_hypervisor_writes(unit, &writes);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_hypercall_answer_with_a_refused_write_lands_none_of_its_writes_in_release() {
    let (mut rt, unit, writes) = refused_write_answer();
    let before = rt.memory().content_hash();
    rt.apply_hypervisor_writes(unit, &writes);
    assert_eq!(rt.memory().content_hash(), before);
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("dispatch.hypervisor_write_refused"),
        1
    );
}
