//! The decrementer of a thread's boundary registers and the thread's own
//! `mfdec` read the same time base, where the run's time base is past
//! the thread's clock and its last read.

use super::interrupt_support::*;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_time::{Budget, GuestTicks};

/// The thread's decrementer at time base 0: near 0, and below
/// [`RUN_TIME_BASE`].
const DEC_AT_ZERO: u32 = 0x10;

/// The highest time base that another unit read before the thread's step.
const RUN_TIME_BASE: u64 = 0x20;

/// SPR 22, the decrementer.
const SPR_DEC: u16 = 22;

/// [PPC-Book3 p:81 s:6.3] The decrementer counts down at the time base
/// rate.
#[test]
fn the_boundary_decrementer_reads_the_run_s_time_base_as_the_thread_s_mfdec_does() {
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().update_supervisor(|s| {
        s.dec = DEC_AT_ZERO;
        s.dec_tb = 0;
    });
    let boundary = unit
        .boundary_registers(GuestTicks::ZERO, RUN_TIME_BASE)
        .expect("supervisor registers");

    let mem = memory(&[mfspr(3, SPR_DEC)]);
    let ctx = ExecutionContext::new(&mem)
        .with_current_tick(GuestTicks::ZERO)
        .with_time_base(RUN_TIME_BASE);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    let mfdec = unit.state().gpr[3] as u32;

    let expected = DEC_AT_ZERO.wrapping_sub(RUN_TIME_BASE as u32);
    assert_eq!(mfdec, expected, "the thread's mfdec");
    assert_eq!(boundary.dec, expected, "the boundary registers");
}
