//! The runtime hands the hypervisor the decrementer of a mediated take at
//! the taking unit's clock, the end of the step that took the interrupt.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    BoundaryRegisters, ExecutionContext, ExecutionStepResult, ExecutionUnit, SaveRestore,
    UnitStatus,
};
use cellgov_lv1::HypercallRefusal;
use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_time::{ticks_to_tb, Budget, GuestTicks, InstructionCost, TICKS_PER_TB_TICK};

use super::{
    call, run, run_to_error, runtime, setup, Shared, Thread, EE_OFF_MSR, PLUG_BIT, PORT, SEND_SC,
};
use crate::runtime::types::StepError;
use crate::runtime::Runtime;

/// The time base ticks that each step of a [`Clocked`] thread lasts.
const STEP_TB: u64 = 0x10;

/// The steps up to the end of the take: three setup calls, the send, and
/// the take.
const STEPS_TO_TAKE: u64 = 5;

/// The time base of the taking unit's clock at the end of its take.
const TAKE_TB: u64 = STEP_TB * STEPS_TO_TAKE;

/// A [`Thread`] whose steps each last [`STEP_TB`] time base ticks, and
/// whose decrementer counts down from `Seen::decrementer` at time base 0.
/// The decrementer counts down at the time base rate
/// [PPC-Book3 p:81 s:6.3], so it reads the later of the tick's time base
/// and the run's.
#[derive(Clone)]
struct Clocked(Thread);

impl ExecutionUnit for Clocked {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.0.unit_id()
    }
    fn status(&self) -> UnitStatus {
        self.0.status()
    }
    fn ppe_thread(&self) -> Option<u8> {
        self.0.ppe_thread()
    }
    fn boundary_registers(&self, tick: GuestTicks, time_base: u64) -> Option<BoundaryRegisters> {
        let tb = ticks_to_tb(tick.raw()).max(time_base);
        let elapsed = u32::try_from(tb).expect("a time base that fits the decrementer");
        let dec = self.0.seen.borrow().decrementer.wrapping_sub(elapsed);
        let regs = self.0.boundary_registers(tick, 0)?;
        Some(BoundaryRegisters { dec, ..regs })
    }
    fn system_call_save_restore(&self) -> Option<SaveRestore> {
        self.0.system_call_save_restore()
    }
    fn raise_external_exception(&mut self) -> bool {
        self.0.raise_external_exception()
    }
    fn clear_external_exception(&mut self) -> bool {
        self.0.clear_external_exception()
    }
    fn raise_mediated_external_exception(&mut self) -> bool {
        self.0.raise_mediated_external_exception()
    }
    fn clear_mediated_external_exception(&mut self) -> bool {
        self.0.clear_mediated_external_exception()
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        let result = self.0.run_until_yield(budget, ctx, effects);
        ExecutionStepResult {
            consumed_cost: InstructionCost::new(STEP_TB * TICKS_PER_TB_TICK),
            ..result
        }
    }
    fn snapshot(&self) {}
}

/// Registers PPE thread 1 as a [`Clocked`] thread that sends on its own
/// plug with EE = 0, so it takes the mediated interrupt at its fifth
/// step. Its decrementer at time base 0 is `dec_at_zero`.
fn spawn_sender(rt: &mut Runtime, dec_at_zero: u32) -> (UnitId, Shared) {
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let seen = Shared::default();
    seen.borrow_mut().decrementer = dec_at_zero;
    let mine = std::rc::Rc::clone(&seen);
    let unit = rt.register_unit_with(move |id| {
        Clocked(Thread {
            id,
            thread: 1,
            script,
            next: 0,
            seen: mine,
            other: None,
            msr: EE_OFF_MSR,
            take_srr0: SEND_SC + 4,
        })
    });
    (unit, seen)
}

/// The decrementer is positive at every clock before the take's step
/// ends, and negative at the end of the take. lv1's handler tests it with
/// a signed compare [OFW-4.93 lv1 a:0x2C632C-0x2C6334], so the hypervisor
/// refuses the take.
#[test]
fn a_take_whose_decrementer_goes_negative_at_the_unit_s_clock_stops_the_run() {
    let mut rt = runtime(PLUG_BIT);
    let dec_at_zero = u32::try_from(TAKE_TB - STEP_TB / 2).expect("a decrementer");
    let (unit, seen) = spawn_sender(&mut rt, dec_at_zero);
    assert_eq!(run_to_error(&mut rt), StepError::HypercallRefused);
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::MediatedExternal {
            unit,
            pc: SEND_SC + 4,
            what: "a take whose decrementer is negative, which lv1's handler resets and can \
                   send to its trampoline slot 0x28B8",
        })
    );
    let seen = seen.borrow();
    assert_eq!(seen.mediated_takes, 1);
    assert_eq!(
        seen.writes.len(),
        usize::try_from(STEPS_TO_TAKE).expect("a count")
    );
}

/// The decrementer reaches 0 at the end of the take. lv1's signed compare
/// [OFW-4.93 lv1 a:0x2C632C-0x2C6334] finds it not negative, so the take
/// enters the vector and the thread finishes: four calls, the take, and
/// the finish.
#[test]
fn a_take_whose_decrementer_reaches_zero_at_the_unit_s_clock_is_accepted() {
    let mut rt = runtime(PLUG_BIT);
    let dec_at_zero = u32::try_from(TAKE_TB).expect("a decrementer");
    let (_, seen) = spawn_sender(&mut rt, dec_at_zero);
    run(&mut rt);
    assert_eq!(rt.hypervisor().refusal(), None);
    let seen = seen.borrow();
    assert_eq!(seen.mediated_takes, 1);
    assert_eq!(seen.writes.len(), 6);
}
