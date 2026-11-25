//! The runtime hands the external interrupt of a hypercall's return to
//! the thread's unit, clears it when the unit takes it, and wakes a
//! paused thread whose plug a call raised. A return to a thread whose
//! `sc` saved EE = 0 raises the mediated external exception, and the
//! hypervisor tests the thread's take of it.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    BoundaryRegisters, ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord,
    LocalDiagnostics, SaveRestore, UnitStatus, YieldReason,
};
use cellgov_lv1::HypercallRefusal;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt::{SRR1_MEDIATED_EXTERNAL, VECTOR_DECREMENTER};
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_EE;
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT, PAUSE,
    SEND_EVENT_LOCALLY,
};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;
use cellgov_ps3_abi::lv1::interrupt_mask::MEDIATED_UNDELIVERED_PC;
use cellgov_time::{Budget, InstructionCost};

use crate::registry::UnitRegistry;
use crate::runtime::types::{RuntimeMode, StepError};
use crate::runtime::Runtime;
use crate::scheduler::{LowestClockScheduler, Scheduler};

#[path = "hypercall_external_wake_tests.rs"]
mod wake;

#[path = "hypercall_external_take_tests.rs"]
mod take;

#[path = "hypercall_external_decrementer_tests.rs"]
mod decrementer;

#[path = "hypercall_external_time_base_tests.rs"]
mod time_base;

#[path = "hypercall_external_clock_tests.rs"]
mod clock;

#[path = "hypercall_external_resume_tests.rs"]
mod resume;

#[path = "hypercall_decrementer_wake_tests.rs"]
mod decrementer_wake;

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

/// The kernel's MSR at a hypercall it makes with EE clear.
const EE_OFF_MSR: u64 = KERNEL_MSR & !MSR_EE;

/// The runtime's step cap, which bounds every run here.
const STEP_CAP: usize = 200;

/// lv1's raise enters 0x500 [OFW-4.93 lv1 a:0x2C3DB8] with bits 48 to 63
/// of the saved MSR cleared and SF set [OFW-4.93 lv1 a:0x18FC-0x1914].
const VECTOR: u64 = 0x500;
const RAISED_MSR: u64 = 0x8000_0000_0000_0000;

/// HSRR1 bit 33, which the PPE sets on every external interrupt
/// [CBE-Handbook p:277 s:9.8]. Only a take that LPCR\[MER\] caused also
/// sets bit 42.
const SRR1_EXTERNAL: u64 = 1 << (63 - 33);
const _: () = assert!(SRR1_MEDIATED_EXTERNAL & SRR1_EXTERNAL == SRR1_EXTERNAL);

/// One step of a scripted thread.
#[derive(Clone, Copy)]
enum Step {
    /// A hypercall from `sc` at the address, with r11 and r3 onward.
    Call(u64, [u64; 9]),
    /// A [`Step::Call`] whose step costs the given number of ticks.
    CallCosting(u64, u64, [u64; 9]),
    /// A step with no hypercall, until the other thread paused.
    AwaitPause,
    /// A step that takes the decrementer interrupt.
    Decrementer,
}

/// What a scripted thread saw, which the test reads.
#[derive(Default)]
struct Seen {
    /// The register writes of each step, in step order.
    writes: Vec<Vec<(u8, u64)>>,
    /// Whether the thread's external exception exists.
    external: bool,
    /// Whether the thread's mediated external exception exists.
    mediated: bool,
    /// The steps that took the direct external interrupt.
    interrupts: u32,
    /// The steps that took the mediated external interrupt.
    mediated_takes: u32,
    /// The mediated takes that the runtime moved to the vector.
    vector_entries: u32,
    /// The mediated takes that the runtime returned to the interrupted
    /// instruction.
    resumes: u32,
    /// Whether the thread yielded its `pause`.
    paused: bool,
    /// The decrementer that the thread's boundary registers give at time
    /// base 0. It counts down by one for each time base tick.
    decrementer: u32,
    /// The `pause` calls that the thread yielded.
    pauses: usize,
    /// The tick at which the decrementer reaches 0 for each `pause`, in
    /// call order. A `pause` with no entry gives no deadline.
    decrementer_deadlines: Vec<u64>,
    /// Each decrementer write, as the value and the raw tick.
    decrementer_writes: Vec<(u32, u64)>,
}

impl Seen {
    /// Records the thread's hypercall `number`.
    fn called(&mut self, number: u64) {
        if number == PAUSE {
            self.paused = true;
            self.pauses += 1;
        }
    }
}

type Shared = Rc<RefCell<Seen>>;

/// A PPE hardware thread with supervisor registers that runs a script.
/// It takes a raised external exception ahead of its next step, as the
/// PPU does with `MSR[EE]` set. It takes a mediated one at its next step
/// too, as if the kernel had set EE again. Each take sets SRR0 to
/// `take_srr0`. A direct take sets SRR1 bit 33, and a mediated take sets
/// bits 33 and 42.
#[derive(Clone)]
struct Thread {
    id: UnitId,
    thread: u8,
    script: Vec<Step>,
    next: usize,
    seen: Shared,
    other: Option<Shared>,
    /// The MSR that the thread's `sc` saves in SRR1.
    msr: u64,
    /// The SRR0 of the thread's take of an external interrupt.
    take_srr0: u64,
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
    fn boundary_registers(
        &self,
        _tick: cellgov_time::GuestTicks,
        time_base: u64,
    ) -> Option<BoundaryRegisters> {
        Some(BoundaryRegisters {
            pc: self.take_srr0,
            msr: self.msr,
            dec: self
                .seen
                .borrow()
                .decrementer
                .wrapping_sub(time_base as u32),
        })
    }
    fn decrementer_zero(
        &self,
        tick: cellgov_time::GuestTicks,
        _time_base: u64,
    ) -> Option<cellgov_time::GuestTicks> {
        let seen = self.seen.borrow();
        let deadline = seen
            .decrementer_deadlines
            .get(seen.pauses.checked_sub(1)?)?;
        Some(cellgov_time::GuestTicks::new(*deadline).max(tick))
    }
    fn write_decrementer(
        &mut self,
        value: u32,
        tick: cellgov_time::GuestTicks,
        _time_base: u64,
    ) -> bool {
        self.seen
            .borrow_mut()
            .decrementer_writes
            .push((value, tick.raw()));
        true
    }
    fn system_call_save_restore(&self) -> Option<SaveRestore> {
        Some(SaveRestore {
            srr0: 0,
            srr1: self.msr,
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
    fn enter_mediated_external_vector(&mut self) -> bool {
        self.seen.borrow_mut().vector_entries += 1;
        true
    }
    fn resume_from_mediated_take(&mut self) -> bool {
        self.seen.borrow_mut().resumes += 1;
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
            return interrupt(VECTOR, self.take_srr0, KERNEL_MSR | SRR1_EXTERNAL);
        }
        if seen.mediated {
            seen.mediated_takes += 1;
            return interrupt(VECTOR, self.take_srr0, KERNEL_MSR | SRR1_MEDIATED_EXTERNAL);
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
                seen.called(args[0]);
                result(
                    YieldReason::Syscall,
                    LocalDiagnostics::with_pc_lr_syscall_lev(pc, 0, 1),
                    Some(args),
                )
            }
            Some(Step::CallCosting(ticks, pc, args)) => {
                self.next += 1;
                seen.called(args[0]);
                ExecutionStepResult {
                    consumed_cost: InstructionCost::new(ticks),
                    ..result(
                        YieldReason::Syscall,
                        LocalDiagnostics::with_pc_lr_syscall_lev(pc, 0, 1),
                        Some(args),
                    )
                }
            }
            Some(Step::AwaitPause) => {
                self.next += 1;
                result(
                    YieldReason::BudgetExhausted,
                    LocalDiagnostics::empty(),
                    None,
                )
            }
            Some(Step::Decrementer) => {
                self.next += 1;
                interrupt(VECTOR_DECREMENTER, 0, KERNEL_MSR)
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

/// A step that took the interrupt at `vector` with SRR0 `srr0` and SRR1
/// `srr1`.
fn interrupt(vector: u64, srr0: u64, srr1: u64) -> ExecutionStepResult {
    let interrupt = InterruptRecord {
        vector,
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
    let mut rt = Runtime::new(memory, Budget::new(4), STEP_CAP);
    rt.set_mode(RuntimeMode::FullTrace);
    rt.set_hypervisor_firmware("4.93");
    rt
}

/// Registers PPE thread `thread` running `script`, whose `sc` saves
/// `msr` and whose takes set SRR0 to `take_srr0`.
fn spawn(
    rt: &mut Runtime,
    thread: u8,
    script: Vec<Step>,
    other: Option<Shared>,
    msr: u64,
    take_srr0: u64,
) -> (UnitId, Shared) {
    let seen = Shared::default();
    let mine = Rc::clone(&seen);
    let unit = rt.register_unit_with(move |id| Thread {
        id,
        thread,
        script,
        next: 0,
        seen: mine,
        other,
        msr,
        take_srr0,
    });
    (unit, seen)
}

/// Registers PPE thread `thread` running `script`, whose `sc` saves the
/// kernel's MSR with EE set.
fn register(rt: &mut Runtime, thread: u8, script: Vec<Step>, other: Option<Shared>) -> Shared {
    spawn(rt, thread, script, other, KERNEL_MSR, 0).1
}

/// Steps `rt` until a step returns an error, and returns that error.
///
/// # Panics
///
/// Panics when no error comes within the step cap.
fn run_to_error(rt: &mut Runtime) -> StepError {
    for _ in 0..=STEP_CAP {
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(error) => return error,
        };
        rt.commit_step(&mut step).expect("commit");
    }
    panic!("no step error within {STEP_CAP} steps");
}

/// Steps `rt` until every unit finished.
fn run(rt: &mut Runtime) {
    assert_eq!(run_to_error(rt), StepError::NoRunnableUnit);
}

/// The send on the caller's own unmasked plug delivers the interrupt on
/// its return: r11 and r12 hold the vector and the new MSR
/// [OFW-4.93 lv1 a:0x1038-0x103C], and the unit takes the direct
/// interrupt once: four calls, the take, and the finish.
#[test]
fn a_return_with_an_unmasked_bit_raises_the_caller_s_exception_until_it_takes_it() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let seen = register(&mut rt, 1, script, None);
    run(&mut rt);
    let seen = seen.borrow();
    assert_eq!(seen.interrupts, 1);
    assert_eq!(seen.mediated_takes, 0, "EE = 1 at the send");
    assert!(!seen.external);
    assert_eq!(seen.writes.len(), 6);
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
    assert_eq!(seen.writes.len(), 6);
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

/// The direct and the mediated exception that earlier raises left end at
/// the next return that finds no shared bit, before the unit's next step
/// takes either.
#[test]
fn a_later_return_with_no_shared_bit_ends_an_earlier_raise() {
    let mut rt = runtime(0);
    let script = vec![call(SEND_SC, CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, BITMAP])];
    let seen = register(&mut rt, 1, script, None);
    let mut step = rt.step().expect("the call");
    {
        let mut seen = seen.borrow_mut();
        seen.external = true;
        seen.mediated = true;
    }
    rt.commit_step(&mut step).expect("commit");
    let seen = seen.borrow();
    assert!(!seen.external);
    assert!(!seen.mediated);
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
    assert_eq!(thread_1.mediated_takes, 0, "EE = 1 at the pause");
    assert!(!thread_1.external);
    assert_eq!(thread_1.writes.len(), 6);
    let after_pause = &thread_1.writes[4];
    assert_eq!(
        &after_pause[after_pause.len() - 2..],
        &[(11, VECTOR), (12, RAISED_MSR)]
    );
}

/// The runtime's own scheduler, which also keeps the unit, the yield
/// reason and the `woke_others` of each step it hears of.
struct WakeLog {
    inner: LowestClockScheduler,
    steps: Rc<RefCell<Vec<(UnitId, YieldReason, bool)>>>,
}

impl Scheduler for WakeLog {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        self.inner.select_next(registry)
    }
    fn notify_yielded(
        &mut self,
        unit: UnitId,
        yield_reason: YieldReason,
        woke_others: bool,
        _holds_critical_section: bool,
    ) {
        self.steps
            .borrow_mut()
            .push((unit, yield_reason, woke_others));
    }
}

/// The send of thread 0 that wakes paused thread 1 makes another unit
/// runnable, so the scheduler hears that the sender's step woke others.
/// No other step wakes a unit.
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
    let (sender, _) = spawn(&mut rt, 0, send, Some(thread_1), KERNEL_MSR, 0);
    run(&mut rt);
    let woke: Vec<(UnitId, YieldReason)> = steps
        .borrow()
        .iter()
        .filter(|(_, _, woke_others)| *woke_others)
        .map(|(unit, reason, _)| (*unit, *reason))
        .collect();
    assert_eq!(woke, vec![(sender, YieldReason::Syscall)]);
}

/// A return to a thread whose `sc` saved EE = 0, with a bit that the
/// status shadow and the mask share, sets LPCR\[MER\]; see
/// `cellgov_lv1::ExternalRaise::Mediated`. The runtime raises the
/// mediated exception and no direct one. The take sets SRR1 bits 33 and
/// 42, lv1's handler finds the bit again, and the take ends the
/// exception: four calls, the take, and the finish.
#[test]
fn a_return_with_ee_off_raises_the_mediated_exception_until_the_take_ends_it() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let (_, seen) = spawn(&mut rt, 1, script, None, EE_OFF_MSR, SEND_SC + 4);
    run(&mut rt);
    let seen = seen.borrow();
    assert_eq!(seen.mediated_takes, 1);
    assert_eq!(seen.interrupts, 0, "EE = 0 at the send");
    assert!(!seen.mediated);
    assert!(!seen.external);
    assert_eq!(seen.writes.len(), 6);
}

/// lv1's handler enters no vector for a take at its trampoline slot
/// (`MEDIATED_UNDELIVERED_PC`). The hypervisor keeps the refusal, which
/// names the unit and the SRR0 of the take, and the run stops at the
/// step after the take: four calls and the take. The runtime moves the
/// thread of the refused take to the vector.
#[test]
fn a_refused_mediated_take_stops_the_run_at_the_next_step() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let (unit, seen) = spawn(
        &mut rt,
        1,
        script,
        None,
        EE_OFF_MSR,
        MEDIATED_UNDELIVERED_PC,
    );
    assert_eq!(run_to_error(&mut rt), StepError::HypercallRefused);
    let refusal = rt.hypervisor().refusal();
    assert!(
        matches!(
            refusal,
            Some(HypercallRefusal::MediatedExternal {
                unit: refused,
                pc: MEDIATED_UNDELIVERED_PC,
                ..
            }) if refused == unit
        ),
        "{refusal:?}"
    );
    let seen = seen.borrow();
    assert_eq!(seen.mediated_takes, 1);
    assert_eq!((seen.resumes, seen.vector_entries), (0, 1));
    assert_eq!(seen.writes.len(), 5);
}

/// Only a take at the external vector ends the external exceptions: a
/// step that takes the decrementer interrupt leaves both.
#[test]
fn a_taken_decrementer_interrupt_leaves_both_external_exceptions() {
    let mut rt = runtime(0);
    let seen = register(&mut rt, 1, vec![Step::Decrementer], None);
    let mut step = rt.step().expect("the decrementer step");
    {
        let mut seen = seen.borrow_mut();
        seen.external = true;
        seen.mediated = true;
    }
    rt.commit_step(&mut step).expect("commit");
    let seen = seen.borrow();
    assert!(seen.external);
    assert!(seen.mediated);
}
