use super::*;

use crate::ppu::generate::DATA_LEN;
use crate::seeded::{seed, SeededDefect};

/// `fadd. f3,f1,f2`: primary 63, extended opcode 21, Rc set.
const FADD_RECORD_F3_F1_F2: u32 = (63 << 26) | (3 << 21) | (1 << 16) | (2 << 11) | (21 << 1) | 1;

fn operands() -> PpuState {
    let mut state = PpuState::new();
    state.set_fpr(1, 1.5f64.to_bits());
    state.set_fpr(2, 2.25f64.to_bits());
    state
}

fn sequence_frt() -> u64 {
    let run = run_sequence(&[FADD_RECORD_F3_F1_F2], &operands(), &[0; DATA_LEN]).unwrap();
    assert_eq!(run.executed, 1);
    run.observed.observation.state.fpr[3]
}

fn step_frt() -> u64 {
    let instruction = cellgov_ppu::decode::decode(FADD_RECORD_F3_F1_F2).unwrap();
    run_once(&instruction, &operands(), &[0; DATA_LEN])
        .unwrap()
        .observation
        .state
        .fpr[3]
}

#[test]
fn a_seeded_relation_defect_lands_on_the_sequence_step_as_on_the_single_step() {
    assert!(matches!(
        cellgov_ppu::decode::decode(FADD_RECORD_F3_F1_F2),
        Ok(PpuInstruction::Fp63 {
            frt: 3,
            rc: true,
            ..
        })
    ));
    let clean = sequence_frt();
    assert_eq!(clean, 3.75f64.to_bits());
    assert_eq!(step_frt(), clean);

    let _guard = seed(SeededDefect::PpuRecordFormFpr);
    let seeded = sequence_frt();
    assert_ne!(seeded, clean, "the sequence step carries the seeded FRT");
    assert_eq!(step_frt(), seeded);
}
