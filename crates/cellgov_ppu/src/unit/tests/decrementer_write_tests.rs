use super::interrupt_support::*;
use cellgov_exec::ExecutionUnit;
use cellgov_time::{GuestTicks, TICKS_PER_TB_TICK};

/// A thread whose decrementer was negative at time base 0, and whose last
/// time base read was `tb`.
fn negative_thread(tb: u64) -> crate::PpuExecutionUnit {
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().update_supervisor(|s| {
        s.dec = 0x8000_0000;
        s.dec_tb = 0;
    });
    unit.state_mut().tb = tb;
    unit
}

fn dec_at(unit: &crate::PpuExecutionUnit, tick: GuestTicks, time_base: u64) -> u32 {
    unit.boundary_registers(tick, time_base)
        .expect("supervisor registers")
        .dec
}

/// [PPC-Book3 p:80 s:6.3] The DEC counts down at the time-base frequency.
#[test]
fn a_written_decrementer_reads_the_value_at_the_boundary_time_base_then_counts_down() {
    let tick = GuestTicks::new(100 * TICKS_PER_TB_TICK);
    let mut unit = negative_thread(0);
    assert!(unit.write_decrementer(0, tick, 0));
    assert_eq!(dec_at(&unit, tick, 0), 0);
    let next = GuestTicks::new(101 * TICKS_PER_TB_TICK);
    assert_eq!(dec_at(&unit, next, 0), u32::MAX);
}

#[test]
fn a_decrementer_write_holds_at_the_run_time_base_when_it_leads_the_tick() {
    let mut unit = negative_thread(0);
    assert!(unit.write_decrementer(0, GuestTicks::ZERO, 0x800));
    assert_eq!(dec_at(&unit, GuestTicks::ZERO, 0x800), 0);
}

#[test]
fn a_decrementer_write_holds_at_the_time_base_the_thread_last_read_when_it_leads_the_tick() {
    let mut unit = negative_thread(0x800);
    assert!(unit.write_decrementer(0, GuestTicks::ZERO, 0));
    assert_eq!(dec_at(&unit, GuestTicks::ZERO, 0), 0);
}

#[test]
fn a_thread_with_no_supervisor_registers_takes_no_decrementer_write() {
    let mut unit = crate::PpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    assert!(!unit.write_decrementer(0, GuestTicks::ZERO, 0));
    assert!(unit.state().supervisor().is_none());
}
