use super::interrupt_support::*;
use cellgov_exec::{BoundaryRegisters, ExecutionUnit};
use cellgov_time::GuestTicks;

/// The decrementer that the tests write, at time base 0.
const DEC: u32 = 0x1000;

/// A thread at [`CODE`] whose decrementer held [`DEC`] at time base 0,
/// and whose last time base read was `tb`.
fn thread(tb: u64) -> crate::PpuExecutionUnit {
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().update_supervisor(|s| {
        s.dec = DEC;
        s.dec_tb = 0;
    });
    unit.state_mut().tb = tb;
    unit
}

/// [PPC-Book3 p:81 s:6.3] The decrementer counts down.
#[test]
fn boundary_registers_give_the_pc_the_msr_and_the_decrementer_at_the_tick() {
    let ticks = 100 * cellgov_time::TICKS_PER_TB_TICK;
    let unit = thread(0);
    assert_eq!(
        unit.boundary_registers(GuestTicks::new(ticks)),
        Some(BoundaryRegisters {
            pc: CODE,
            msr: KERNEL_MSR,
            dec: DEC - 100,
        })
    );
}

#[test]
fn the_decrementer_reads_the_thread_s_own_time_base_where_that_one_is_later() {
    let unit = thread(0x10);
    let registers = unit
        .boundary_registers(GuestTicks::ZERO)
        .expect("supervisor registers");
    assert_eq!(registers.dec, DEC - 0x10);
}

#[test]
fn a_thread_with_no_supervisor_registers_gives_no_boundary_registers() {
    let unit = crate::PpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    assert_eq!(unit.boundary_registers(GuestTicks::ZERO), None);
}
