//! The runtime hands the external interrupt of a hypercall's return to
//! the thread's unit, clears it when the unit takes it, and wakes a
//! paused thread whose plug a call raised.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord, LocalDiagnostics,
    SaveRestore, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT, PAUSE,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_time::{Budget, InstructionCost};

use crate::registry::UnitRegistry;
use crate::runtime::types::{RuntimeMode, StepError};
use crate::runtime::Runtime;
use crate::scheduler::{LowestClockScheduler, Scheduler};

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;
const PORT: u64 = 4;

/// The `sc` of the kernel's event send
/// [OFW-4.93 lv2_kernel a:0x8000000000025a34].
const SEND_SC: u64 = 0x8000_0000_0002_5a34;

/// Thread 1's bitmap, in the real mode region.
const BITMAP: u64 = 0x0036_9200;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG: u64 = 9;
const PLUG_BIT: u64 = 1 << 54;

/// The 4.93 kernel's MSR at its hypercalls: SF, EE, IR, DR and RI.
const KERNEL_MSR: u64 = 0x8000_0000_0000_8032;

/// lv1's raise enters 0x500 [OFW-4.93 lv1 a:0x2C3DB8] with bits 48 to 63
/// of the saved MSR cleared and SF set [OFW-4.93 lv1 a:0x18FC-0x1914].
const VECTOR: u64 = 0x500;
const RAISED_MSR: u64 = 0x8000_0000_0000_0000;

/// One step of a scripted thread.
#[derive(Clone, Copy)]
enum Step {
    /// A hypercall from `sc` at the address, with r11 and r3 onward.
    Call(u64, [u64; 9]),
    /// A step with no hypercall, until the other thread paused.
    AwaitPause,
}

/// What a scripted thread saw, which the test reads.
#[derive(Default)]
struct Seen {
    /// The register writes of each step, in step order.
    writes: Vec<Vec<(u8, u64)>>,
    /// Whether the thread's external exception exists.
    external: bool,
    /// The steps that took the external interrupt.
    interrupts: u32,
    /// Whether the thread yielded its `pause`.
    paused: bool,
}

type Shared = Rc<RefCell<Seen>>;

/// A PPE hardware thread with supervisor registers that runs a script.
/// It takes a raised external exception ahead of its next step, as the
/// PPU does with `MSR[EE]` set.
#[derive(Clone)]
struct Thread {
    id: UnitId,
    thread: u8,
    script: Vec<Step>,
    next: usize,
    seen: Shared,
    other: Option<Shared>,
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
            srr1: KERNEL_MSR,
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
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let mut seen = self.seen.borrow_mut();
        seen.writes.push(ctx.register_writes().to_vec());
        if seen.external {
            seen.interrupts += 1;
            let interrupt = InterruptRecord {
                vector: VECTOR,
                srr0: 0,
                srr1: KERNEL_MSR,
                dsisr: None,
                dar: None,
            };
            return result(
                YieldReason::InterruptBoundary,
                LocalDiagnostics {
                    interrupt: Some(interrupt),
                    ..LocalDiagnostics::empty()
                },
                None,
            );
        }
        let step = self.script.get(self.next).copied();
        let other_paused = self.other.as_ref().is_some_and(|o| o.borrow().paused);
        match step {
            Some(Step::AwaitPause) if !other_paused => result(
                YieldReason::BudgetExhausted,
                LocalDiagnostics::empty(),
                None,
            ),
            Some(Step::Call(pc, args)) => {
                self.next += 1;
                seen.paused |= args[0] == PAUSE;
                result(
                    YieldReason::Syscall,
                    LocalDiagnostics::with_pc_lr_syscall_lev(pc, 0, 1),
                    Some(args),
                )
            }
            Some(Step::AwaitPause) => {
                self.next += 1;
                result(
                    YieldReason::BudgetExhausted,
                    LocalDiagnostics::empty(),
                    None,
                )
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

fn call(pc: u64, number: u64, inputs: &[u64]) -> Step {
    let mut args = [0; 9];
    args[0] = number;
    args[1..=inputs.len()].copy_from_slice(inputs);
    Step::Call(pc, args)
}

/// Thread 1's setup: its bitmap, port 4, and the port on plug 9 of
/// thread 1.
fn setup() -> Vec<Step> {
    vec![
        call(SEND_SC, CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, BITMAP]),
        call(RECORD_SC, CONSTRUCT_EVENT_RECEIVE_PORT, &[]),
        call(SEND_SC, CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0]),
    ]
}

/// A 4.93 runtime whose real memory holds `mask` in thread 1's first mask
/// doubleword.
fn runtime(mask: u64) -> Runtime {
    let mut memory = GuestMemory::new(0x0037_0000);
    let range = ByteRange::new(GuestAddr::new(BITMAP + IRQ_MASK_OFFSET), 8).expect("a range");
    memory
        .apply_commit(range, &mask.to_be_bytes())
        .expect("the mask");
    let mut rt = Runtime::new(memory, Budget::new(4), 200);
    rt.set_mode(RuntimeMode::FullTrace);
    rt.set_hypervisor_firmware("4.93");
    rt
}

fn register(rt: &mut Runtime, thread: u8, script: Vec<Step>, other: Option<Shared>) -> Shared {
    let seen = Shared::default();
    let mine = Rc::clone(&seen);
    rt.register_unit_with(move |id| Thread {
        id,
        thread,
        script,
        next: 0,
        seen: mine,
        other,
    });
    seen
}

/// Steps `rt` until every unit finished. The step cap bounds the loop.
fn run(rt: &mut Runtime) {
    loop {
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(StepError::NoRunnableUnit) => return,
            Err(error) => panic!("{error:?}"),
        };
        rt.commit_step(&mut step).expect("commit");
    }
}

/// The send on the caller's own unmasked plug delivers the interrupt on
/// its return: r11 and r12 hold the vector and the new MSR
/// [OFW-4.93 lv1 a:0x1038-0x103C], and the unit takes the interrupt once.
#[test]
fn a_return_with_an_unmasked_bit_raises_the_caller_s_exception_until_it_takes_it() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let seen = register(&mut rt, 1, script, None);
    run(&mut rt);
    let seen = seen.borrow();
    assert_eq!(seen.interrupts, 1);
    assert!(!seen.external);
    let after_send = &seen.writes[4];
    assert_eq!(
        &after_send[after_send.len() - 2..],
        &[(11, VECTOR), (12, RAISED_MSR)]
    );
}

/// The step that takes the interrupt carries no effect, so in the
/// fault-driven mode it commits on the fast path. That path ends the
/// exception too, and the unit takes the interrupt once.
#[test]
fn a_taken_interrupt_ends_on_the_fast_commit_path() {
    let mut rt = runtime(PLUG_BIT);
    rt.set_mode(RuntimeMode::FaultDriven);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let seen = register(&mut rt, 1, script, None);
    run(&mut rt);
    let seen = seen.borrow();
    assert_eq!(seen.interrupts, 1);
    assert!(!seen.external);
}

/// lv1's return reloads the LPCR from the context [OFW-4.93 lv1 a:0x1008]
/// and raises nothing for a masked bit.
#[test]
fn a_return_with_a_masked_bit_raises_nothing() {
    let mut rt = runtime(0);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let seen = register(&mut rt, 1, script, None);
    run(&mut rt);
    let seen = seen.borrow();
    assert_eq!(seen.interrupts, 0);
    assert!(!seen.external);
    assert_eq!(seen.writes.len(), 5);
}

/// An exception that an earlier raise left ends at the next return that
/// finds no shared bit, before the unit's next step takes it.
#[test]
fn a_later_return_with_no_shared_bit_ends_an_earlier_raise() {
    let mut rt = runtime(0);
    let script = vec![call(SEND_SC, CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, BITMAP])];
    let seen = register(&mut rt, 1, script, None);
    let mut step = rt.step().expect("the call");
    seen.borrow_mut().external = true;
    rt.commit_step(&mut step).expect("commit");
    assert!(!seen.borrow().external);
}

/// The send from thread 0 raises plug 9 of paused thread 1, whose mask
/// holds the bit, so lv1 wakes it [OFW-4.93 lv1 a:0x2C6680-0x2C66DC].
/// Thread 1 runs again, and its return from `pause` delivers the
/// interrupt.
#[test]
fn a_raise_that_wakes_a_paused_thread_makes_it_run_into_its_interrupt() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, PAUSE, &[0]));
    let thread_1 = register(&mut rt, 1, script, None);
    let send = vec![Step::AwaitPause, call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT])];
    let thread_0 = register(&mut rt, 0, send, Some(Rc::clone(&thread_1)));
    run(&mut rt);
    assert_eq!(thread_0.borrow().interrupts, 0);
    let thread_1 = thread_1.borrow();
    assert_eq!(thread_1.interrupts, 1);
    assert!(!thread_1.external);
    let after_pause = &thread_1.writes[4];
    assert_eq!(
        &after_pause[after_pause.len() - 2..],
        &[(11, VECTOR), (12, RAISED_MSR)]
    );
}

/// The runtime's own scheduler, which also keeps the yield reason and
/// the `woke_others` of each step it hears of.
struct WakeLog {
    inner: LowestClockScheduler,
    steps: Rc<RefCell<Vec<(YieldReason, bool)>>>,
}

impl Scheduler for WakeLog {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        self.inner.select_next(registry)
    }
    fn notify_yielded(
        &mut self,
        _unit: UnitId,
        yield_reason: YieldReason,
        woke_others: bool,
        _holds_critical_section: bool,
    ) {
        self.steps.borrow_mut().push((yield_reason, woke_others));
    }
}

/// The send that wakes paused thread 1 makes another unit runnable, so
/// the scheduler hears that its step woke others. No other step wakes a
/// unit.
#[test]
fn a_hypercall_that_wakes_a_paused_thread_tells_the_scheduler_it_woke_others() {
    let mut rt = runtime(PLUG_BIT);
    let steps = Rc::new(RefCell::new(Vec::new()));
    rt.set_scheduler(WakeLog {
        inner: LowestClockScheduler::new(),
        steps: Rc::clone(&steps),
    });
    let mut script = setup();
    script.push(call(SEND_SC, PAUSE, &[0]));
    let thread_1 = register(&mut rt, 1, script, None);
    let send = vec![Step::AwaitPause, call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT])];
    register(&mut rt, 0, send, Some(thread_1));
    run(&mut rt);
    let woke: Vec<YieldReason> = steps
        .borrow()
        .iter()
        .filter(|(_, woke_others)| *woke_others)
        .map(|(reason, _)| *reason)
        .collect();
    assert_eq!(woke, vec![YieldReason::Syscall]);
}
