//! The shufb control-class relation: its partner, its refusals, and the
//! observation it leaves equal.

use super::types::*;
use crate::instruction::SpuInstruction;
use crate::observation::SpuObservation;
use crate::state::SpuState;
use cellgov_event::UnitId;

/// The RRR word `shufb rt, ra, rb, rc`: opcode 0b1011, then RT, RB, RA, RC.
fn shufb_word(rt: u32, ra: u32, rb: u32, rc: u32) -> u32 {
    0xB000_0000 | rt << 21 | rb << 14 | ra << 7 | rc
}

fn initial() -> SpuState {
    let mut state = SpuState::new();
    state.set_reg(1, std::array::from_fn(|i| 0xA0 + i as u8));
    state.set_reg(2, std::array::from_fn(|i| 0xB0 + i as u8));
    // Every class: 10xxxxxx, 110xxxxx, 111xxxxx, RA selectors, RB selectors.
    state.set_reg(
        3,
        [
            0x80, 0xBF, 0xC0, 0xDF, 0xE0, 0xFF, 0x00, 0x0F, 0x10, 0x1F, 0x3F, 0x7F, 0x45, 0x9A,
            0xD3, 0xEC,
        ],
    );
    state
}

fn observe(instruction: &SpuInstruction, mut state: SpuState) -> (SpuObservation, SpuState) {
    let outcome = crate::exec::execute(instruction, &mut state, UnitId::new(0));
    (SpuObservation::capture(&state, &outcome), state)
}

#[test]
fn shufb_declares_the_control_class_relation() {
    let raw = shufb_word(4, 1, 2, 3);
    let instruction = crate::decode::decode(raw).expect("shufb decodes");
    assert!(instruction
        .fuzz_descriptor()
        .relations
        .contains(&SpuMetamorphicRelation::ShufbControlClass));
}

#[test]
fn every_rc_byte_moves_within_its_class_and_the_result_holds() {
    let raw = shufb_word(4, 1, 2, 3);
    let instruction = crate::decode::decode(raw).expect("shufb decodes");
    let case = instruction
        .metamorphic_case(raw, SpuMetamorphicRelation::ShufbControlClass)
        .expect("distinct RC has a partner");
    assert_eq!(case.partner_word, raw);
    assert_eq!(
        case.varied_input,
        Some(SpuVariedInput {
            register: 3,
            restore: true,
        })
    );

    let original = initial();
    let partner_initial = case.partner_initial(&original);
    for (before, after) in original.regs[3].iter().zip(partner_initial.regs[3]) {
        assert_ne!(*before, after, "every control byte is rewritten");
        if before & 0x80 != 0 {
            assert_eq!(before & 0xE0, after & 0xE0, "constant class kept");
        } else {
            assert_eq!(before & 0x1F, after & 0x1F, "selected byte kept");
            assert_eq!(after & 0x80, 0, "selector stays a selector");
        }
    }

    let (baseline, _) = observe(&instruction, original.clone());
    let (_, partner_final) = observe(&instruction, partner_initial);
    assert_ne!(baseline.state.regs[3], partner_final.regs[3]);
    let mut partner_regs = *partner_final.regs.as_array();
    case.settle_partner(&original, &mut partner_regs);
    assert_eq!(baseline.state.regs, partner_regs);
}

#[test]
fn an_rc_that_writes_rt_keeps_the_partner_result() {
    let raw = shufb_word(3, 1, 2, 3);
    let instruction = crate::decode::decode(raw).expect("shufb decodes");
    let case = instruction
        .metamorphic_case(raw, SpuMetamorphicRelation::ShufbControlClass)
        .expect("RC = RT still has a partner");
    assert_eq!(
        case.varied_input,
        Some(SpuVariedInput {
            register: 3,
            restore: false,
        })
    );
    let original = initial();
    let (baseline, _) = observe(&instruction, original.clone());
    let (_, partner_final) = observe(&instruction, case.partner_initial(&original));
    let mut partner_regs = *partner_final.regs.as_array();
    case.settle_partner(&original, &mut partner_regs);
    assert_eq!(baseline.state.regs, partner_regs);
}

#[test]
fn an_rc_that_is_also_a_data_input_has_no_partner() {
    for raw in [shufb_word(4, 3, 2, 3), shufb_word(4, 1, 3, 3)] {
        let instruction = crate::decode::decode(raw).expect("shufb decodes");
        assert_eq!(
            instruction.metamorphic_case(raw, SpuMetamorphicRelation::ShufbControlClass),
            Err(SpuRelationRefusal::NoPartner {
                relation: SpuMetamorphicRelation::ShufbControlClass,
            })
        );
    }
}

#[test]
fn a_word_relation_runs_from_the_original_state() {
    let nop = crate::decode::decode(0x4020_0000).expect("nop decodes");
    let case = nop
        .metamorphic_case(0x4020_0000, SpuMetamorphicRelation::NopFalseTarget)
        .expect("nop has a partner");
    assert_eq!(case.varied_input, None);
    let original = initial();
    assert_eq!(case.partner_initial(&original).regs, original.regs);
}
