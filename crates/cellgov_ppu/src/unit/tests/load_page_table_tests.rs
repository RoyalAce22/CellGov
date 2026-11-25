use super::*;
use crate::supervisor::SupervisorState;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;

/// A page table at 2^40 with HTABSIZE 2: 1 MB.
const SDR1: u64 = 0x0000_0100_0000_0002;

/// 64 KB pages for LP = 0 and 1 MB pages for LP = 1.
const LB: u8 = 0b1001;

fn supervisor_unit(rm_limit: u64, rm_offset: u64) -> PpuExecutionUnit {
    let mut sup = SupervisorState::new(MSR_SF);
    sup.partition =
        Partition::new(0, rm_limit, rm_offset, Partition::INITIAL.lb()).expect("partition");
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_supervisor(Some(sup));
    unit
}

fn partition(unit: &PpuExecutionUnit) -> Partition {
    unit.state().supervisor().expect("supervisor").partition
}

#[test]
fn a_supervisor_thread_takes_sdr1_and_lb_and_keeps_its_real_mode_block() {
    let mut unit = supervisor_unit(1 << 28, 1 << 28);
    let before = unit.state().state_hash();
    assert!(unit.load_page_table(SDR1, LB));
    let p = partition(&unit);
    assert_eq!((p.sdr1(), p.lb()), (SDR1, LB));
    assert_eq!((p.rm_limit(), p.rm_offset()), (1 << 28, 1 << 28));
    assert!(unit.state().hash_is_current());
    assert_ne!(unit.state().state_hash(), before);
}

#[test]
fn values_that_name_no_page_table_or_a_reserved_size_leave_the_partition() {
    let mut unit = supervisor_unit(1 << 62, 0);
    let before = partition(&unit);
    assert!(!unit.load_page_table(SDR1 | 0x20, LB));
    assert!(!unit.load_page_table(SDR1, 0b1111));
    assert_eq!(partition(&unit), before);
}

#[test]
fn a_thread_without_supervisor_registers_takes_nothing() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    assert!(!unit.load_page_table(SDR1, LB));
    assert!(unit.state().supervisor().is_none());
}
