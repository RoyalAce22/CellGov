use super::*;
use crate::supervisor::SupervisorState;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;

/// A page table at 2^40 with HTABSIZE 2: 1 MB.
const SDR1: u64 = 0x0000_0100_0000_0002;

/// 64 KB pages for LP = 0 and 1 MB pages for LP = 1.
const LB: u8 = 0b1001;

#[test]
fn a_supervisor_thread_answers_that_it_takes_the_load_and_then_takes_it() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF)));
    assert!(unit.takes_page_table());
    assert!(unit.load_page_table(SDR1, LB));
}

#[test]
fn a_thread_without_supervisor_registers_answers_that_it_refuses_the_load() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    assert!(!unit.takes_page_table());
    assert!(!unit.load_page_table(SDR1, LB));
}
