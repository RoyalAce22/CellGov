use super::interrupt_support::*;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_time::{Budget, GuestTicks};

/// The tests' hypercall number, in r11.
const NUMBER: u64 = 91;
/// The tests' hypercall inputs, in r3 to r10.
const INPUTS: [u64; 8] = [2, 0x6269_0000, 0x7075 << 48, 0, 0, 0x55, 0x66, 0x77];

fn hypercall_unit() -> crate::PpuExecutionUnit {
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().set_gpr(11, NUMBER);
    for (i, v) in INPUTS.iter().enumerate() {
        unit.state_mut().set_gpr(3 + i, *v);
    }
    unit
}

#[test]
fn sc_with_lev_1_in_supervisor_state_yields_r11_and_r3_to_r10_and_stays_on_sc() {
    let mem = memory(&[SC_1, NOP]);
    let mut unit = hypercall_unit();
    let (r, effects) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    assert_eq!(r.fault, None);
    assert_eq!(r.local_diagnostics.syscall_lev, Some(1));
    assert_eq!(r.local_diagnostics.pc, Some(CODE));
    let mut expected = [NUMBER; 9];
    expected[1..].copy_from_slice(&INPUTS);
    assert_eq!(r.syscall_args, Some(expected));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, cellgov_effects::Effect::SharedWriteIntent { .. })),
        "{effects:?}"
    );
    assert_eq!(unit.state().pc, CODE);
    // Interrupt entry would clear FP and RI.
    assert_eq!(
        sup(&unit).msr,
        KERNEL_MSR,
        "the hypercall took no interrupt"
    );
}

#[test]
fn the_runtimes_answer_moves_the_thread_past_sc_with_r3_and_the_outputs() {
    let mem = memory(&[SC_1, NOP]);
    let mut unit = hypercall_unit();
    run(&mut unit, &mem, 4);
    let outputs = [(4u8, 0x44u64), (5, 0x45)];
    let ctx = ExecutionContext::with_syscall_return_and_regs(&mem, &[], 0, &outputs);
    let r = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    let s = unit.state();
    assert_eq!(s.pc, CODE + 8, "the nop after sc retired");
    assert_eq!((s.gpr[3], s.gpr[4], s.gpr[5]), (0, 0x44, 0x45));
}

/// No hypercall number has a measured cost.
#[test]
fn a_hypercall_and_the_instruction_after_it_pay_no_lv2_round_trip() {
    let mem = memory(&[SC_1, NOP]);
    let mut unit = hypercall_unit();
    let (r, _) = run(&mut unit, &mem, 4);
    let at_sc = GuestTicks::from(r.consumed_cost).raw();
    let ctx = ExecutionContext::with_syscall_return(&mem, &[], 0);
    let r = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    let after = GuestTicks::from(r.consumed_cost).raw();
    let trip = cellgov_ps3_abi::hw::ppu_issue::PPU_SYSCALL_ROUND_TRIP_CYCLES;
    assert!(at_sc < trip, "the hypercall's batch costs {at_sc} cycles");
    assert!(
        after < trip,
        "the nop after the hypercall costs {after} cycles"
    );
}

/// [CBE-Handbook p:307 s:10.4.4] CTRL bits 0:1 name the thread that reads CTRL.
#[test]
fn the_unit_reports_the_hardware_thread_its_ctrl_names() {
    let mem = memory(&[NOP, NOP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    // Alone on the PPE, a thread reads CTRL as hardware thread 1.
    run(&mut unit, &mem, 1);
    assert_eq!(unit.ppe_thread(), Some(1));
    // Beside an older sibling, it reads CTRL as hardware thread 0.
    let ctx = ExecutionContext::new(&mem)
        .with_ppe_siblings(1)
        .with_ppe_older_sibling(true);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(unit.ppe_thread(), Some(0));
}
