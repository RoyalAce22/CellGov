//! A thread in supervisor state takes a program interrupt for an access
//! to a hypervisor SPR.

use super::interrupt_support::*;
use cellgov_exec::{InterruptRecord, YieldReason};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;

/// HSPRG0, which only hypervisor state can read or write.
const HSPRG0: u16 = 304;

/// [PPC-Book3 p:71 s:5.5.9] A program interrupt sets SRR0 to the instruction that caused it.
/// [PPC-Book3 p:72 s:5.5.9] A program interrupt sets SRR1 bit 45 for a privileged instruction.
/// [CBE-Handbook p:254 s:9.5.9] In supervisor state the PPE takes a privileged-instruction interrupt for an access to a hypervisor SPR.
#[test]
fn a_supervisor_state_hsprg0_move_takes_the_privileged_program_interrupt() {
    for (raw, gpr) in [(mfspr(3, HSPRG0), 0x1234), (mtspr(HSPRG0, 3), 0x5678)] {
        let mem = memory(&[raw]);
        let mut unit = kernel_unit(KERNEL_MSR);
        unit.state_mut().set_gpr(3, gpr);
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(
            r.yield_reason,
            YieldReason::InterruptBoundary,
            "0x{raw:08x}"
        );
        assert_eq!(r.fault, None, "0x{raw:08x}");
        assert_eq!(
            r.local_diagnostics.interrupt,
            Some(InterruptRecord {
                vector: irq::VECTOR_PROGRAM,
                srr0: CODE,
                srr1: KERNEL_MSR | irq::SRR1_PROGRAM_PRIVILEGED,
                dsisr: None,
                dar: None,
            }),
            "0x{raw:08x}"
        );
        assert_eq!(unit.state().pc, irq::VECTOR_PROGRAM, "0x{raw:08x}");
        assert_eq!(unit.state().gpr[3], gpr, "0x{raw:08x}");
    }
}
