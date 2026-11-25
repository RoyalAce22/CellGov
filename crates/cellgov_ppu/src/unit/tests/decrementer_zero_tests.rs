use super::interrupt_support::*;
use cellgov_exec::ExecutionUnit;
use cellgov_time::{GuestTicks, TICKS_PER_TB_TICK};

/// The decrementer that the tests write, at time base 0.
const DEC: u32 = 0x1000;

/// A thread whose decrementer held `dec` at time base 0, and whose last
/// time base read was `tb`.
fn thread(dec: u32, tb: u64) -> crate::PpuExecutionUnit {
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().update_supervisor(|s| {
        s.dec = dec;
        s.dec_tb = 0;
    });
    unit.state_mut().tb = tb;
    unit
}

/// [PPC-Book3 p:80 s:6.3] The DEC counts down at the time-base frequency.
#[test]
fn the_decrementer_reaches_0_at_the_first_tick_of_the_time_base_it_counts_down_to() {
    let read = GuestTicks::new(100 * TICKS_PER_TB_TICK);
    assert_eq!(
        thread(DEC, 0).decrementer_zero(read, 0),
        Some(GuestTicks::new(u64::from(DEC) * TICKS_PER_TB_TICK))
    );
}

#[test]
fn a_later_run_time_base_reads_the_decrementer_lower_and_keeps_its_deadline_in_ticks() {
    let at_zero = Some(GuestTicks::new(u64::from(DEC) * TICKS_PER_TB_TICK));
    assert_eq!(
        thread(DEC, 0).decrementer_zero(GuestTicks::ZERO, 0x800),
        at_zero
    );
    assert_eq!(
        thread(DEC, 0x800).decrementer_zero(GuestTicks::ZERO, 0),
        at_zero
    );
}

/// A negative decrementer gives a deadline before the read, as lv1's
/// sign extension does [OFW-4.93 lv1 a:0x2C6110-0x2C613C].
#[test]
fn a_decrementer_at_0_or_negative_reaches_0_at_the_read() {
    let read = GuestTicks::new(10 * TICKS_PER_TB_TICK);
    assert_eq!(thread(10, 0).decrementer_zero(read, 0), Some(read));
    assert_eq!(thread(0, 0).decrementer_zero(read, 0), Some(read));
    assert_eq!(thread(0x8000_000A, 0).decrementer_zero(read, 0), Some(read));
}

#[test]
fn a_decrementer_at_0_reaches_0_at_the_read_when_the_read_time_base_leads_the_tick() {
    assert_eq!(
        thread(0x800, 0x800).decrementer_zero(GuestTicks::ZERO, 0),
        Some(GuestTicks::ZERO)
    );
    assert_eq!(
        thread(0x800, 0).decrementer_zero(GuestTicks::ZERO, 0x800),
        Some(GuestTicks::ZERO)
    );
}

#[test]
fn a_thread_with_no_supervisor_registers_gives_no_decrementer_deadline() {
    let unit = crate::PpuExecutionUnit::new(cellgov_event::UnitId::new(0));
    assert_eq!(unit.decrementer_zero(GuestTicks::ZERO, 0), None);
}
