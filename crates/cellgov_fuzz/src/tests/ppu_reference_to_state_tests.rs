use std::collections::BTreeMap;

use super::*;

fn input() -> PpuReferenceInputState {
    PpuReferenceInputState {
        base: PpuReferenceStateBase::Zeroed,
        gpr: BTreeMap::new(),
        fpr: BTreeMap::new(),
        vr_hex: BTreeMap::new(),
        pc: 0,
        cr: 0,
        lr: 0,
        ctr: 0,
        xer: 0,
        fpscr: None,
        vrsave: 0,
        vscr: None,
        tb: 0,
        reservation: None,
    }
}

#[test]
fn a_register_past_its_bank_is_refused_and_one_inside_it_is_applied() {
    let mut fpr = input();
    fpr.fpr.insert(32, 1);
    assert!(matches!(
        fpr.to_state(),
        Err(PpuReferenceError::RegisterIndex {
            bank: "FPR",
            index: 32
        })
    ));
    let mut vr = input();
    vr.vr_hex.insert(40, "0".repeat(32));
    assert!(matches!(
        vr.to_state(),
        Err(PpuReferenceError::RegisterIndex {
            bank: "VR",
            index: 40
        })
    ));
    let mut inside = input();
    inside.fpr.insert(31, 0x3ff0_0000_0000_0000);
    inside.vr_hex.insert(31, format!("{:032x}", 5u128));
    let state = inside.to_state().expect("indices inside the banks apply");
    let snapshot = cellgov_ppu::differential::PpuStateSnapshot::capture(&state);
    assert_eq!(snapshot.fpr[31], 0x3ff0_0000_0000_0000);
    assert_eq!(snapshot.vr[31], 5);
}
