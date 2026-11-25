use super::interrupt_support::*;
use cellgov_exec::{InterruptRecord, YieldReason};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_FP, MSR_ME, MSR_SF, MSR_VXU};

/// `fadd f1,f2,f3`.
const FADD: u32 = 0xFC22_182A;
/// `vaddubm v1,v2,v3`.
const VADDUBM: u32 = 0x1022_1800;

/// [PPC-Book3 p:72 s:5.5.10] The floating-point unavailable interrupt sets SRR0 to the instruction, clears SRR1 bits 33:36 and 42:47, and resumes at 0x800.
/// [CBE-Handbook p:258 s:9.5.15] Table 9-17: the VXU unavailable interrupt sets SRR0 to the instruction the processor would execute next, and clears SRR1 bits 33:36 and 42:47.
#[test]
fn an_unavailable_unit_vectors_with_srr0_at_the_instruction() {
    // Each case turns its own unit off and the other unit on.
    for (insn, msr, vector) in [
        (
            FADD,
            (KERNEL_MSR & !MSR_FP) | MSR_VXU,
            irq::VECTOR_FP_UNAVAILABLE,
        ),
        (VADDUBM, KERNEL_MSR, irq::VECTOR_VXU_UNAVAILABLE),
    ] {
        let mem = memory(&[insn]);
        let mut unit = kernel_unit(msr);
        // Non-zero sources, so a result written to f1 or v1 shows.
        for k in [2, 3] {
            unit.state_mut().set_fpr(k, 1.0f64.to_bits());
            unit.state_mut()
                .set_vr(k, 0x0101_0101_0101_0101_0101_0101_0101_0101);
        }
        let (r, _) = run(&mut unit, &mem, 1);
        assert_eq!(r.yield_reason, YieldReason::InterruptBoundary, "{insn:08x}");
        assert_eq!(
            r.local_diagnostics.interrupt,
            Some(InterruptRecord {
                vector,
                srr0: CODE,
                srr1: msr,
                dsisr: None,
                dar: None,
            })
        );
        let s = sup(&unit);
        assert_eq!((s.srr0, s.srr1), (CODE, msr));
        // [CBE-Handbook p:292 s:9.12] Table 9-40: entry clears FP and RI, sets SF, and keeps ME and VXU.
        assert_eq!(s.msr, MSR_SF | MSR_ME | (msr & MSR_VXU), "{insn:08x}");
        assert_eq!(unit.state().pc, vector, "{insn:08x}");
        assert_eq!(unit.state().fpr[1], 0, "{insn:08x}");
        assert_eq!(unit.state().vr[1], 0, "{insn:08x}");
    }
}

#[test]
fn an_available_unit_runs_on_past_the_instruction() {
    for insn in [FADD, VADDUBM] {
        let mem = memory(&[insn, NOP]);
        let mut unit = kernel_unit(KERNEL_MSR | MSR_VXU);
        let (r, _) = run(&mut unit, &mem, 2);
        assert_eq!(r.yield_reason, YieldReason::BudgetExhausted, "{insn:08x}");
        assert_eq!(unit.state().pc, CODE + 8, "{insn:08x}");
    }
}

#[test]
fn an_unavailable_unit_after_a_retired_instruction_commits_it_and_vectors_next_batch() {
    let mem = memory(&[addi(3, 0, 7), FADD]);
    let mut unit = kernel_unit(KERNEL_MSR & !MSR_FP);
    let (first, _) = run(&mut unit, &mem, 2);
    assert_eq!(first.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!((unit.state().pc, unit.state().gpr[3]), (CODE + 4, 7));
    let (second, _) = run(&mut unit, &mem, 2);
    assert_eq!(second.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.state().pc, irq::VECTOR_FP_UNAVAILABLE);
    assert_eq!(sup(&unit).srr0, CODE + 4);
}
