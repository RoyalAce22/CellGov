//! A send on a plug of another running thread gives that thread the
//! exception of lv1's handler, which the thread sees at its next step.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    BoundaryRegisters, ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord,
    LocalDiagnostics, SaveRestore, UnitStatus, YieldReason,
};
use cellgov_lv1::{RunningThread, RunningThreads, ThreadInterrupt};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_EE;
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use super::running_threads;
use crate::registry::UnitRegistry;
use crate::runtime::types::{RuntimeMode, StepError};
use crate::runtime::Runtime;

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;
const PORT: u64 = 4;

/// The `sc` of the kernel's send on another thread's port
/// [OFW-4.93 lv2_kernel a:0x8000000000026628].
const SEND_SC: u64 = 0x8000_0000_0002_6628;

/// Thread 1's bitmap, in the real mode region.
const BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG: u64 = 9;
const PLUG_BIT: u64 = 1 << 54;

/// The kernel's MSR with EE set: SF, EE, IR, DR and RI.
const EE_SET_MSR: u64 = 0x8000_0000_0000_8032;

/// Thread 1's pc and MSR at the kernel's send to it, in the 4.93 kernel
/// census.
const TARGET_PC: u64 = 0x8000_0000_0007_e8c4;
const EE_CLEAR_MSR: u64 = EE_SET_MSR & !MSR_EE;

/// A scripted thread's decrementer at tick 0. It counts down one a tick.
const DEC_AT_ZERO: u32 = 0x7FFF_0000;

/// The runtime's step cap, which bounds every run here.
const STEP_CAP: usize = 200;

/// One step of a scripted thread.
#[derive(Clone, Copy)]
enum Step {
    /// A hypercall from `sc` at the address, with r11 and r3 onward.
    Call(u64, [u64; 9]),
    /// A step with no hypercall, after which the thread is ready.
    Spin,
    /// A step with no hypercall, until the other thread is ready.
    AwaitReady,
}

/// One entry of the order in which the threads stepped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// Thread 0 yielded its send.
    Sent,
    /// Thread 1 began a step with these exceptions.
    Step {
        /// Whether the external exception existed.
        external: bool,
        /// Whether the mediated external exception existed.
        mediated: bool,
    },
}

/// What a scripted thread holds, which the test reads.
#[derive(Default)]
struct Seen {
    /// Whether the thread's external exception exists.
    external: bool,
    /// Whether the thread's mediated external exception exists.
    mediated: bool,
    /// Whether the thread finished a step with no hypercall.
    ready: bool,
    /// The steps that took the direct external interrupt.
    interrupts: u32,
}

type Shared = Rc<RefCell<Seen>>;
type Log = Rc<RefCell<Vec<Entry>>>;

/// A PPE hardware thread with supervisor registers that runs a script.
/// It takes a raised external exception ahead of its next step, as the
/// PPU does with `MSR[EE]` set. It holds a mediated one, as the PPU does
/// with `MSR[EE]` clear.
#[derive(Clone)]
struct Thread {
    id: UnitId,
    thread: u8,
    script: Vec<Step>,
    next: usize,
    seen: Shared,
    other: Option<Shared>,
    log: Log,
    msr: u64,
}

impl ExecutionUnit for Thread {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.next > self.script.len() {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn ppe_thread(&self) -> Option<u8> {
        Some(self.thread)
    }
    fn system_call_save_restore(&self) -> Option<SaveRestore> {
        Some(SaveRestore {
            srr0: 0,
            srr1: self.msr,
        })
    }
    fn boundary_registers(&self, tick: GuestTicks) -> Option<BoundaryRegisters> {
        let elapsed = u32::try_from(tick.raw()).expect("a tick that fits the decrementer");
        Some(BoundaryRegisters {
            pc: TARGET_PC,
            msr: self.msr,
            dec: DEC_AT_ZERO.wrapping_sub(elapsed),
        })
    }
    fn raise_external_exception(&mut self) -> bool {
        self.seen.borrow_mut().external = true;
        true
    }
    fn clear_external_exception(&mut self) -> bool {
        self.seen.borrow_mut().external = false;
        true
    }
    fn raise_mediated_external_exception(&mut self) -> bool {
        self.seen.borrow_mut().mediated = true;
        true
    }
    fn clear_mediated_external_exception(&mut self) -> bool {
        self.seen.borrow_mut().mediated = false;
        true
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let mut seen = self.seen.borrow_mut();
        if self.thread == 1 {
            self.log.borrow_mut().push(Entry::Step {
                external: seen.external,
                mediated: seen.mediated,
            });
        }
        if seen.external {
            seen.interrupts += 1;
            return interrupt(TARGET_PC, self.msr);
        }
        let other_ready = self.other.as_ref().is_some_and(|o| o.borrow().ready);
        match self.script.get(self.next).copied() {
            Some(Step::AwaitReady) if !other_ready => idle(),
            Some(Step::Call(pc, args)) => {
                self.next += 1;
                if args[0] == SEND_EVENT_LOCALLY {
                    self.log.borrow_mut().push(Entry::Sent);
                }
                result(
                    YieldReason::Syscall,
                    LocalDiagnostics::with_pc_lr_syscall_lev(pc, 0, 1),
                    Some(args),
                )
            }
            Some(Step::Spin | Step::AwaitReady) => {
                self.next += 1;
                seen.ready = true;
                idle()
            }
            None => {
                self.next += 1;
                result(YieldReason::Finished, LocalDiagnostics::empty(), None)
            }
        }
    }
    fn snapshot(&self) {}
}

fn result(
    yield_reason: YieldReason,
    local_diagnostics: LocalDiagnostics,
    syscall_args: Option<[u64; 9]>,
) -> ExecutionStepResult {
    ExecutionStepResult {
        yield_reason,
        consumed_cost: InstructionCost::new(1),
        local_diagnostics,
        fault: None,
        syscall_args,
    }
}

fn idle() -> ExecutionStepResult {
    result(
        YieldReason::BudgetExhausted,
        LocalDiagnostics::empty(),
        None,
    )
}

/// A step that took the external interrupt at 0x500 ahead of `srr0`.
fn interrupt(srr0: u64, srr1: u64) -> ExecutionStepResult {
    let interrupt = InterruptRecord {
        vector: 0x500,
        srr0,
        srr1,
        dsisr: None,
        dar: None,
    };
    result(
        YieldReason::InterruptBoundary,
        LocalDiagnostics {
            interrupt: Some(interrupt),
            ..LocalDiagnostics::empty()
        },
        None,
    )
}

fn call(pc: u64, number: u64, inputs: &[u64]) -> Step {
    let mut args = [0; 9];
    args[0] = number;
    args[1..=inputs.len()].copy_from_slice(inputs);
    Step::Call(pc, args)
}

/// A 4.93 runtime whose real memory holds [`PLUG_BIT`] in thread 1's
/// first mask doubleword.
fn runtime() -> Runtime {
    let mut memory = GuestMemory::new(0x0037_0000);
    let range = ByteRange::new(GuestAddr::new(BITMAP + IRQ_MASK_OFFSET), 8).expect("a range");
    memory
        .apply_commit(range, &PLUG_BIT.to_be_bytes())
        .expect("the mask");
    let mut rt = Runtime::new(memory, Budget::new(4), STEP_CAP);
    rt.set_mode(RuntimeMode::FullTrace);
    rt.set_hypervisor_firmware("4.93");
    rt
}

/// Steps `rt` until no unit runs.
///
/// # Panics
///
/// Panics on a step error other than [`StepError::NoRunnableUnit`], or
/// when the run outlasts the step cap.
fn run(rt: &mut Runtime) {
    for _ in 0..=STEP_CAP {
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(error) => {
                assert_eq!(error, StepError::NoRunnableUnit);
                return;
            }
        };
        rt.commit_step(&mut step).expect("commit");
    }
    panic!("the run outlasted {STEP_CAP} steps");
}

/// Thread 1 sets up its port on its plug 9 and then spins; thread 0
/// sends on the port once thread 1 spins. Thread 1 runs with `msr`.
/// Returns thread 1's state and the step order.
fn send_to_spinning_thread(msr: u64) -> (Shared, Vec<Entry>) {
    let mut rt = runtime();
    let log = Log::default();
    let thread_1 = Shared::default();
    let target = vec![
        call(SEND_SC, CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, BITMAP]),
        call(RECORD_SC, CONSTRUCT_EVENT_RECEIVE_PORT, &[]),
        call(SEND_SC, CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0]),
        Step::Spin,
        Step::Spin,
        Step::Spin,
    ];
    let (seen, steps) = (Rc::clone(&thread_1), Rc::clone(&log));
    rt.register_unit_with(move |id| Thread {
        id,
        thread: 1,
        script: target,
        next: 0,
        seen,
        other: None,
        log: steps,
        msr,
    });
    let sender = vec![Step::AwaitReady, call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT])];
    let (other, steps) = (Rc::clone(&thread_1), Rc::clone(&log));
    rt.register_unit_with(move |id| Thread {
        id,
        thread: 0,
        script: sender,
        next: 0,
        seen: Shared::default(),
        other: Some(other),
        log: steps,
        msr: EE_SET_MSR,
    });
    run(&mut rt);
    let entries = log.borrow().clone();
    (thread_1, entries)
}

/// The entries of thread 1's steps before and after thread 0's send.
fn around_send(entries: &[Entry]) -> (&[Entry], &[Entry]) {
    let sent = entries
        .iter()
        .position(|e| *e == Entry::Sent)
        .expect("thread 0 sent");
    (&entries[..sent], &entries[sent + 1..])
}

const QUIET: Entry = Entry::Step {
    external: false,
    mediated: false,
};

/// Thread 1 runs with EE clear, so lv1's handler sets LPCR\[MER\]
/// [OFW-4.93 lv1 a:0x187C]. Its first step after the send holds the
/// mediated exception.
#[test]
fn a_send_to_a_thread_with_ee_clear_gives_it_the_mediated_exception_at_its_next_step() {
    let (thread_1, entries) = send_to_spinning_thread(EE_CLEAR_MSR);
    let (before, after) = around_send(&entries);
    assert!(!before.is_empty());
    assert!(before.iter().all(|e| *e == QUIET), "{before:?}");
    assert_eq!(
        after.first(),
        Some(&Entry::Step {
            external: false,
            mediated: true,
        })
    );
    assert_eq!(thread_1.borrow().interrupts, 0);
}

/// Thread 1 runs with EE set, so lv1's handler enters its vector
/// [OFW-4.93 lv1 a:0x188C-0x18C0]. Its first step after the send takes
/// the interrupt, and the take ends the exception.
#[test]
fn a_send_to_a_thread_with_ee_set_makes_its_next_step_take_the_interrupt() {
    let (thread_1, entries) = send_to_spinning_thread(EE_SET_MSR);
    let (_, after) = around_send(&entries);
    assert_eq!(
        after.first(),
        Some(&Entry::Step {
            external: true,
            mediated: false,
        })
    );
    assert!(after[1..].iter().all(|e| *e == QUIET), "{after:?}");
    let thread_1 = thread_1.borrow();
    assert_eq!(thread_1.interrupts, 1);
    assert!(!thread_1.external);
}

/// Thread 1 holds the external and the mediated exception of earlier
/// sends, and has not stepped since. A second send whose handler raises
/// nothing keeps the external exception: the handler's LPCR reload clears
/// only LPCR\[MER\] [OFW-4.93 lv1 a:0x1A40] [OFW-4.93 lv1 a:0x1A5C].
#[test]
fn a_send_that_raises_nothing_clears_the_mediated_exception_and_keeps_an_untaken_external_one() {
    let mut rt = runtime();
    let thread_1 = Shared::default();
    let seen = Rc::clone(&thread_1);
    rt.register_unit_with(move |id| Thread {
        id,
        thread: 1,
        script: vec![Step::Spin],
        next: 0,
        seen,
        other: None,
        log: Log::default(),
        msr: EE_SET_MSR,
    });
    {
        let mut held = thread_1.borrow_mut();
        held.external = true;
        held.mediated = true;
    }
    rt.interrupt_running_thread(&ThreadInterrupt {
        thread: 1,
        external: None,
    });
    let held = thread_1.borrow();
    assert!(held.external, "the untaken external exception was lost");
    assert!(!held.mediated, "the LPCR reload left LPCR[MER]");
}

/// A registry of thread 0 and thread 1, each running with
/// [`EE_CLEAR_MSR`].
fn registry() -> (UnitRegistry, [UnitId; 2]) {
    let mut registry = UnitRegistry::new();
    let ids = [0u8, 1].map(|thread| {
        registry.register_with(|id| Thread {
            id,
            thread,
            script: vec![Step::Spin],
            next: 0,
            seen: Shared::default(),
            other: None,
            log: Log::default(),
            msr: EE_CLEAR_MSR,
        })
    });
    (registry, ids)
}

fn registers() -> Option<RunningThread> {
    registers_at(0)
}

/// The registers of a thread of [`registry`] whose decrementer reads the
/// tick `tick`.
fn registers_at(tick: u32) -> Option<RunningThread> {
    Some(RunningThread {
        pc: TARGET_PC,
        msr: EE_CLEAR_MSR,
        decrementer: DEC_AT_ZERO - tick,
    })
}

#[test]
fn each_runnable_thread_gives_its_registers() {
    let (registry, _) = registry();
    assert_eq!(
        running_threads(&registry, GuestTicks::ZERO),
        RunningThreads([registers(), registers()])
    );
}

/// A unit that holds a syscall return still stands at its `sc`.
#[test]
fn a_thread_that_holds_a_syscall_return_gives_no_registers() {
    let (mut registry, ids) = registry();
    registry.set_syscall_return(ids[1], 0);
    assert_eq!(
        running_threads(&registry, GuestTicks::ZERO),
        RunningThreads([registers(), None])
    );
}

/// Thread 1's clock is past the call's tick, so its pc is that of a later
/// boundary, and its decrementer reads that boundary's tick. Thread 0's
/// clock is behind the call's tick, so its decrementer reads the call's.
#[test]
fn a_thread_whose_clock_is_past_the_call_reads_its_decrementer_at_its_own_clock() {
    let (mut registry, ids) = registry();
    registry.set_clock(ids[0], GuestTicks::new(0x8));
    registry.set_clock(ids[1], GuestTicks::new(0x100));
    assert_eq!(
        running_threads(&registry, GuestTicks::new(0x10)),
        RunningThreads([registers_at(0x10), registers_at(0x100)])
    );
}

#[test]
fn a_blocked_thread_gives_no_registers() {
    let (mut registry, ids) = registry();
    registry.set_status_override(ids[0], UnitStatus::Blocked);
    assert_eq!(
        running_threads(&registry, GuestTicks::ZERO),
        RunningThreads([None, registers()])
    );
}
