//! The taken flag the timing model gives a conditional branch.

use super::classify;
use super::op::Kind;
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;

#[test]
fn a_bcctr_with_bo_2_clear_tests_the_decremented_ctr() {
    let mut state = PpuState::new();
    state.set_ctr(0x4008);
    state.set_cr_field(0, 0b0010);
    // BO 0x08 branches on EQ while CTR != 0; BO 0x0a on EQ while CTR == 0.
    for (bo, taken) in [(0x08, true), (0x0a, false)] {
        let (ops, _) = classify(
            &I::Bcctr {
                bo,
                bi: 2,
                link: false,
            },
            &state,
        );
        let Kind::Branch { taken: t, .. } = ops[0].kind else {
            panic!("bo {bo:#x}: {:?}", ops[0].kind);
        };
        assert_eq!(t, taken, "bo {bo:#x}");
    }
}
