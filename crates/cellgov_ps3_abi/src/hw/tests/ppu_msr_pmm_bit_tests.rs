use super::*;

/// [PPC-Book3 p:108 s:E.1] MSR bit 61 is PMM.
#[test]
fn the_performance_monitor_mark_is_msr_bit_61() {
    assert_eq!(MSR_PMM, 0x4);
}
