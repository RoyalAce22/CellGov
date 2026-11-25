use super::*;

/// [PPC-Book3 p:21 s:3.4.2] Figure 18 marks HSPRG0, HSPRG1, RMOR, HRMOR, HSRR0, HSRR1, LPCR and LPIDR "hypv" for mfspr.
/// [CBE-Handbook p:98 s:4.2.7.8] Only the hypervisor can read the TLB.
const HYPERVISOR_READS: [u16; 12] = [304, 305, 312, 313, 314, 315, 318, 319, 946, 947, 948, 949];

/// [PPC-Book3 p:19 s:3.4.2] Figure 17 marks SDR1, TBL, TBU, HSPRG0, HSPRG1, PURR, HDEC, RMOR, HRMOR, HSRR0, HSRR1, LPCR, LPIDR, DABR and DABRX "hypv" for mtspr.
/// [CBE-Handbook p:332 s:11.2.3] Only the hypervisor can access the TLB facilities.
const HYPERVISOR_WRITES: [u16; 19] = [
    25, 284, 285, 304, 305, 309, 310, 312, 313, 314, 315, 318, 319, 946, 947, 948, 949, 1013, 1015,
];

#[test]
fn exactly_the_documented_hypervisor_resources_are_of_the_hypervisor_class() {
    let reads: Vec<u16> = (0..1024)
        .filter(|&spr| spr_read_class(spr) == SprClass::Hypervisor)
        .collect();
    let writes: Vec<u16> = (0..1024)
        .filter(|&spr| spr_write_class(spr) == SprClass::Hypervisor)
        .collect();
    assert_eq!(reads, HYPERVISOR_READS);
    assert_eq!(writes, HYPERVISOR_WRITES);
}

/// [CBE-Handbook p:152 s:6.3] Supervisor-state software controls the L2 and TLB replacement through the RMTs.
/// [CBE-Handbook p:153 s:6.3.1.3] The privileged mtspr and mfspr access the address range registers.
#[test]
fn the_rmt_and_address_range_sprs_stay_privileged_both_ways() {
    let sprs: Vec<u16> = (950..=957).chain(976..=981).collect();
    assert_eq!(sprs.len(), 14);
    for spr in sprs {
        assert_eq!(
            (spr_read_class(spr), spr_write_class(spr)),
            (SprClass::Privileged, SprClass::Privileged),
            "{spr}"
        );
    }
}

/// [CBE-Handbook p:254 s:9.5.9] In problem state the PPE takes a privileged-instruction interrupt for a supervisor or hypervisor SPR, and an illegal-instruction interrupt for a reserved one.
#[test]
fn the_privileged_and_hypervisor_classes_trap_as_privileged_in_problem_state() {
    assert!(SprClass::Privileged.privileged_in_problem_state());
    assert!(SprClass::Hypervisor.privileged_in_problem_state());
    assert!(!SprClass::User.privileged_in_problem_state());
    assert!(!SprClass::Reserved.privileged_in_problem_state());
}
