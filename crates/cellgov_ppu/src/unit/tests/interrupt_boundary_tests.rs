//! The decrementer and external interrupts a thread with supervisor
//! registers takes between two instructions.

use super::interrupt_support::*;
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionUnit, InterruptRecord, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_EE, MSR_ME, MSR_RI, MSR_SF};
use cellgov_time::GuestTicks;

/// Words of `nop` at [`CODE`]: more than any run here retires.
const NOPS: usize = 1024;

/// A DEC value that stays non-negative for every run here.
const FAR_DEC: u32 = 0x7FFF_FFFF;

/// A DEC value with bit 0 set: a decrementer exception exists at once.
const NEGATIVE_DEC: u32 = 0x8000_0000;

fn nops() -> GuestMemory {
    memory(&[NOP; NOPS])
}

/// A thread at [`CODE`] with MSR `msr` and DEC `dec` at time base 0.
fn thread(msr: u64, dec: u32) -> PpuExecutionUnit {
    let mut unit = kernel_unit(msr);
    unit.state_mut()
        .update_supervisor(|s| {
            s.dec = dec;
            s.dec_tb = 0;
        })
        .expect("supervisor registers");
    unit
}

/// The record of a decrementer or external interrupt at `pc`, taken
/// with MSR `msr`.
///
/// [PPC-Book3 p:69 s:5.5.7] An external interrupt sets SRR0 to the next instruction, SRR1 bits 33:36 and 42:47 to 0 and the rest to the MSR.
/// [CBE-Handbook p:255 s:9.5.11] A decrementer interrupt sets SRR0 to the next instruction.
fn boundary_record(vector: u64, pc: u64, msr: u64) -> InterruptRecord {
    InterruptRecord {
        vector,
        srr0: pc,
        srr1: msr,
        dsisr: None,
        dar: None,
    }
}

/// Runs batches of `budget` instructions, each at the guest tick the
/// batches before it consumed, until the thread yields at an interrupt
/// boundary. Returns the instructions it retired before the interrupt
/// and the interrupt record.
fn run_to_interrupt(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
) -> (u64, InterruptRecord) {
    let mut tick = 0;
    for _ in 0..NOPS {
        let (r, _) = run_at(unit, mem, budget, tick);
        tick += GuestTicks::from(r.consumed_cost).raw();
        match r.yield_reason {
            YieldReason::BudgetExhausted => {}
            YieldReason::InterruptBoundary => {
                let record = r.local_diagnostics.interrupt.expect("an interrupt record");
                return (unit.retired_instructions(), record);
            }
            other => panic!("budget {budget}: yield {other:?}"),
        }
    }
    panic!("budget {budget}: no interrupt in {NOPS} batches");
}

/// [PPC-Book3 p:81 s:6.3] A decrementer exception exists while DEC bit 0 is 1; [CBE-Handbook p:255 s:9.5.11] the PPE takes it with MSR\[EE\] = 1.
#[test]
fn the_decrementer_vectors_to_0x900_once_dec_turns_negative_with_ee_1() {
    let mem = nops();
    let msr = KERNEL_MSR | MSR_EE;
    let mut unit = thread(msr, 1);
    let (retired, record) = run_to_interrupt(&mut unit, &mem, 256);
    assert!(retired > 0, "DEC = 1 is not negative at time base 0");
    assert_eq!(
        record,
        boundary_record(irq::VECTOR_DECREMENTER, CODE + 4 * retired, msr)
    );
    let s = sup(&unit);
    assert_eq!((s.srr0, s.srr1), (record.srr0, record.srr1));
    assert_eq!(s.msr, MSR_SF | MSR_ME, "entry clears EE, FP and RI");
    assert_eq!(unit.state().pc, irq::VECTOR_DECREMENTER);
    assert!(
        (s.dec_at(unit.state().tb) as i32) < 0,
        "DEC is negative at the interrupt"
    );

    let mut far = thread(msr, FAR_DEC);
    let (r, _) = run(&mut far, &mem, 256);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(far.retired_instructions(), 256);
}

#[test]
fn the_decrementer_interrupt_lands_on_one_instruction_at_budget_1_and_budget_256() {
    let mem = nops();
    let msr = KERNEL_MSR | MSR_EE;
    let (one, one_record) = run_to_interrupt(&mut thread(msr, 1), &mem, 1);
    let (wide, wide_record) = run_to_interrupt(&mut thread(msr, 1), &mem, 256);
    assert!(
        0 < wide && wide < 256,
        "the interrupt falls inside the first wide batch: {wide}"
    );
    assert_eq!(one, wide);
    assert_eq!(one_record, wide_record);
}

/// [PPC-Book3 p:23 s:3.4] An `mtmsrd` that sets EE while an exception exists takes the interrupt before the next instruction.
#[test]
fn a_decrementer_exception_waits_while_ee_is_0_and_is_taken_after_the_move_that_sets_ee() {
    // nop; nop; mtmsrd r7,1 with r7 = EE | RI; nop.
    let mem = memory(&[NOP, NOP, mtmsrd(7, true), NOP]);
    let mut unit = thread(KERNEL_MSR, NEGATIVE_DEC);
    unit.state_mut().set_gpr(7, MSR_EE | MSR_RI);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(
        unit.retired_instructions(),
        3,
        "the MSR move ends the batch"
    );
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(boundary_record(
            irq::VECTOR_DECREMENTER,
            CODE + 12,
            KERNEL_MSR | MSR_EE
        ))
    );
    assert_eq!(unit.retired_instructions(), 3);
}

/// [PPC-Book3 p:69 s:5.5.7] An external interrupt needs MSR\[EE\] = 1, and taking it does not end the external exception.
#[test]
fn an_external_exception_vectors_to_0x500_and_stays_until_cleared() {
    let mut mem = nops();
    put_words(&mut mem, irq::VECTOR_EXTERNAL, &[RFID]);
    let msr = KERNEL_MSR | MSR_EE;
    let mut unit = thread(msr, FAR_DEC);
    assert!(unit.raise_external_exception());

    let (r, _) = run(&mut unit, &mem, 8);
    let want = boundary_record(irq::VECTOR_EXTERNAL, CODE, msr);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(r.local_diagnostics.interrupt, Some(want));
    assert_eq!(unit.state().pc, irq::VECTOR_EXTERNAL);
    assert_eq!(sup(&unit).msr, MSR_SF | MSR_ME);
    assert!(
        sup(&unit).external,
        "taking the interrupt keeps the exception"
    );

    // The handler's rfid returns with EE = 1, and the exception is taken
    // again before the instruction it returned to.
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted, "the rfid");
    assert_eq!(unit.state().pc, CODE);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.local_diagnostics.interrupt, Some(want));

    // After the clear, the exception is gone at the next return.
    assert!(unit.clear_external_exception());
    run(&mut unit, &mem, 8);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(unit.state().pc, CODE + 32);
}

/// [PPC-Book3 p:69 s:5.5.7] An External interrupt occurs when an External exception exists and MSR\[EE\] = 1.
#[test]
fn an_external_exception_waits_while_ee_is_0() {
    let mem = nops();
    let mut unit = thread(KERNEL_MSR, FAR_DEC);
    assert!(unit.raise_external_exception());
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 8);
}

/// [PPC-Book3 p:76 s:5.8] External and decrementer exceptions have equal priority and the processor may take either.
/// [CBE-Handbook p:291 s:9.10] Table 9-39: the PPE ranks the external interrupt above the decrementer interrupt.
#[test]
fn an_external_exception_is_taken_ahead_of_a_decrementer_exception_at_one_boundary() {
    let mem = nops();
    let msr = KERNEL_MSR | MSR_EE;
    let mut unit = thread(msr, NEGATIVE_DEC);
    assert!(unit.raise_external_exception());
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(boundary_record(irq::VECTOR_EXTERNAL, CODE, msr))
    );

    let mut unit = thread(msr, NEGATIVE_DEC);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(boundary_record(irq::VECTOR_DECREMENTER, CODE, msr)),
        "without the external exception the decrementer one is taken"
    );
}

#[test]
fn the_decrementer_test_ahead_of_a_faulting_instruction_publishes_its_time_base_at_any_budget() {
    let mem = memory(&[ILLEGAL]);
    let tick = 1000 * cellgov_time::TICKS_PER_TB_TICK;
    let mut time_bases = [None; 2];
    for (time_base, budget) in time_bases.iter_mut().zip([1, 8]) {
        let mut unit = thread(KERNEL_MSR | MSR_EE, FAR_DEC);
        let (r, effects) = run_at(&mut unit, &mem, budget, tick);
        assert_eq!(r.yield_reason, YieldReason::InterruptBoundary, "{budget}");
        let reads = effects
            .iter()
            .filter(|e| matches!(e, cellgov_effects::Effect::ClockRead { .. }))
            .count();
        assert_eq!(reads, 1, "budget {budget}: {effects:?}");
        *time_base = unit.time_base_read();
    }
    let read = time_bases[0].expect("budget 1 publishes a time base read");
    assert!(
        read >= cellgov_time::ticks_to_tb(tick),
        "the test reads the time base at its tick: {time_bases:?}"
    );
    assert_eq!(
        time_bases[1],
        Some(read),
        "budget 8 publishes the time base the test read, not the one at batch entry"
    );
}

#[test]
fn a_thread_without_supervisor_registers_has_no_external_exception_to_raise() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let hash = unit.state().state_hash();
    assert!(!unit.raise_external_exception());
    assert!(!unit.clear_external_exception());
    assert_eq!(unit.state().state_hash(), hash);
}
