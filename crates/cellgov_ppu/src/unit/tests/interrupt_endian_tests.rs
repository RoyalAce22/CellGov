//! [CBE-Handbook p:739 s:A.2.5] The PPE supports only big-endian mode, and a write to MSR\[LE\] or MSR\[ILE\] has no effect.

use super::interrupt_support::*;
use cellgov_exec::{InterruptRecord, YieldReason};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_ILE, MSR_LE, MSR_ME, MSR_SF};

/// [PPC-Book3 p:71 s:5.5.9] A program interrupt sets SRR0 to the instruction that caused it.
/// [PPC-Book3 p:72 s:5.5.9] A program interrupt sets SRR1 bit 46 for a trap.
/// [CBE-Handbook p:292 s:9.12] Table 9-40: an interrupt clears FP and RI, sets SF and keeps ME.
#[test]
fn an_interrupt_after_an_mtmsrd_that_asks_for_le_saves_and_enters_with_ile_and_le_0() {
    let mem = memory(&[mtmsrd(3, false), TRAP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().set_gpr(3, KERNEL_MSR | MSR_ILE | MSR_LE);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.fault, None);
    assert_eq!(sup(&unit).msr, KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(
        r.local_diagnostics.interrupt,
        Some(InterruptRecord {
            vector: irq::VECTOR_PROGRAM,
            srr0: CODE + 4,
            srr1: KERNEL_MSR | irq::SRR1_PROGRAM_TRAP,
            dsisr: None,
            dar: None,
        })
    );
    assert_eq!(sup(&unit).srr1, KERNEL_MSR | irq::SRR1_PROGRAM_TRAP);
    assert_eq!(sup(&unit).msr, MSR_SF | MSR_ME);
}
