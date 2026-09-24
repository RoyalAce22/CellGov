use super::registry::*;
use super::types::*;
use crate::instruction::ops::*;
use crate::instruction::{PpuInstruction, PpuInstructionKind};
use crate::state::PpuState;

fn descriptor_with(relation: PpuMetamorphicRelation) -> PpuGenerationDescriptor {
    generation_descriptors()
        .into_iter()
        .find(|descriptor| {
            crate::decode::decode(descriptor.canonical_word).is_ok_and(|instruction| {
                instruction
                    .fuzz_descriptor(descriptor.canonical_word)
                    .relations
                    .contains(&relation)
            })
        })
        .unwrap()
}

#[test]
fn record_relations_derive_cr0_cr1_and_cr6_partners_from_typed_metadata() {
    for (relation, delta) in [
        (PpuMetamorphicRelation::RecordCr0, PpuPermittedDelta::Cr0),
        (PpuMetamorphicRelation::RecordCr1, PpuPermittedDelta::Cr1),
        (PpuMetamorphicRelation::RecordCr6, PpuPermittedDelta::Cr6),
    ] {
        let descriptor = descriptor_with(relation);
        let raw = descriptor.canonical_word
            & match relation {
                PpuMetamorphicRelation::RecordCr6 => !0x0000_0400,
                _ => !0x0000_0001,
            };
        let instruction = crate::decode::decode(raw).unwrap();
        let case = instruction
            .metamorphic_case(raw, &PpuState::new(), relation)
            .unwrap();
        assert_eq!(case.permitted_delta, delta);
        assert_ne!(case.partner_word, raw);
        assert_eq!(
            crate::decode::decode(case.partner_word)
                .unwrap()
                .fuzz_descriptor(case.partner_word)
                .kind,
            instruction.fuzz_descriptor(raw).kind
        );
    }
}

#[test]
fn enabled_and_undeclared_relations_are_refused_before_comparison() {
    let descriptor = descriptor_with(PpuMetamorphicRelation::RecordCr0);
    let raw = descriptor.canonical_word & !1;
    let instruction = crate::decode::decode(raw).unwrap();

    assert!(matches!(
        instruction.metamorphic_case(raw | 1, &PpuState::new(), PpuMetamorphicRelation::RecordCr0),
        Err(PpuRelationRefusal::AlreadyEnabled { .. })
    ));
    assert!(matches!(
        instruction.metamorphic_case(raw, &PpuState::new(), PpuMetamorphicRelation::RecordCr6),
        Err(PpuRelationRefusal::Undeclared { .. })
    ));
}

#[test]
fn overflow_relation_requires_rc_clear_and_preserves_the_exact_kind() {
    let descriptor = descriptor_with(PpuMetamorphicRelation::OverflowEnable);
    let raw = descriptor.canonical_word & !0x0000_0401;
    let instruction = crate::decode::decode(raw).unwrap();
    let case = instruction
        .metamorphic_case(
            raw,
            &PpuState::new(),
            PpuMetamorphicRelation::OverflowEnable,
        )
        .unwrap();

    assert_eq!(case.permitted_delta, PpuPermittedDelta::XerOverflow);
    assert_eq!(case.partner_word, raw | 0x0000_0400);
    assert!(matches!(
        instruction.metamorphic_case(
            raw | 1,
            &PpuState::new(),
            PpuMetamorphicRelation::OverflowEnable,
        ),
        Err(PpuRelationRefusal::IncompatibleControls { .. })
    ));
}

#[test]
fn undefined_instruction_state_is_refused_before_partner_execution() {
    let raw = generation_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(PpuInstructionKind::Divd))
        .unwrap()
        .canonical_word
        & !0x0000_0401;
    let instruction = crate::decode::decode(raw).unwrap();
    let state = PpuState::new();

    assert!(matches!(
        instruction.metamorphic_case(raw, &state, PpuMetamorphicRelation::RecordCr0),
        Err(PpuRelationRefusal::ArchitecturallyUndefined { .. })
    ));
}

#[test]
fn slw_declares_its_record_cr0_relation() {
    let instruction = PpuInstruction::Slw {
        ra: 1,
        rs: 2,
        rb: 3,
        rc: false,
    };

    assert!(instruction
        .fuzz_descriptor(0)
        .relations
        .contains(&PpuMetamorphicRelation::RecordCr0));
}

#[test]
fn fp63_forms_with_a_reserved_low_bit_do_not_declare_a_record_relation() {
    for op in [Fp63Op::Fcmpu, Fp63Op::Fcmpo, Fp63Op::Mcrfs] {
        let instruction = PpuInstruction::Fp63 {
            op,
            frt: 0,
            fra: 0,
            frb: 0,
            frc: 0,
            rc: false,
        };

        assert!(!instruction
            .fuzz_descriptor(0)
            .relations
            .contains(&PpuMetamorphicRelation::RecordCr1));
    }
}

#[test]
fn undefined_fp_outputs_are_refused_before_partner_execution() {
    for op in [Fp63Op::Fctiw, Fp63Op::Fctiwz, Fp63Op::Mffs] {
        let instruction = PpuInstruction::Fp63 {
            op,
            frt: 0,
            fra: 0,
            frb: 0,
            frc: 0,
            rc: false,
        };

        assert!(matches!(
            instruction.metamorphic_case(0, &PpuState::new(), PpuMetamorphicRelation::RecordCr1),
            Err(PpuRelationRefusal::ArchitecturallyUndefined { .. })
        ));
    }
}
