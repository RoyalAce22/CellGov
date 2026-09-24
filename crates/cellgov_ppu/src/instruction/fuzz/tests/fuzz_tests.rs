use super::bits::*;
use super::classify::*;
use super::registry::*;
use super::types::*;
use crate::instruction::ops::*;
use crate::instruction::{PpuInstruction, PpuInstructionKind};
use cellgov_effects::EffectKind;
use std::collections::BTreeSet;

#[test]
fn generation_registry_covers_every_standalone_exact_kind() {
    let descriptors = generation_descriptors();
    let actual = descriptors
        .iter()
        .map(|descriptor| descriptor.kind)
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected_generation_kinds());

    let ori = descriptors
        .into_iter()
        .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(PpuInstructionKind::Ori))
        .expect("ori must have a generation descriptor");
    assert_eq!(ori.canonical_word >> 26, 24);
    assert_eq!(ori.form, PpuEncodingForm::D);
    assert_eq!(ori.operands.len(), 3);
}

#[test]
fn sequence_classes_keep_xer_ownership_in_the_descriptor() {
    let lswx = generation_descriptor((31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (533 << 1))
        .expect("lswx must have a generation descriptor");
    let mtxer = generation_descriptor((31 << 26) | (3 << 21) | (1 << 16) | (467 << 1))
        .expect("mtxer must have a generation descriptor");
    let addi = generation_descriptor(14 << 26).expect("addi must have a generation descriptor");

    assert_eq!(lswx.sequence_class, PpuSequenceClass::ReadsXerByteCount);
    assert_eq!(mtxer.sequence_class, PpuSequenceClass::ReplacesXer);
    assert_eq!(addi.sequence_class, PpuSequenceClass::Independent);
}

#[test]
fn sequence_flow_keeps_state_and_control_ownership_in_the_descriptor() {
    let descriptors = generation_descriptors();
    let flow = |kind| {
        descriptors
            .iter()
            .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(kind))
            .map(|descriptor| descriptor.sequence_flow)
            .expect("instruction must have a generation descriptor")
    };

    assert_eq!(flow(PpuInstructionKind::Addi), PpuSequenceFlow::Linear);
    assert_eq!(
        flow(PpuInstructionKind::Lwz),
        PpuSequenceFlow::StateDependent
    );
    assert_eq!(
        flow(PpuInstructionKind::B),
        PpuSequenceFlow::ControlTransfer
    );
    assert_eq!(flow(PpuInstructionKind::Sc), PpuSequenceFlow::Terminal);
}

#[test]
fn sequence_dependencies_name_only_read_write_register_forms() {
    let descriptors = generation_descriptors();
    let dependency = |kind| {
        descriptors
            .iter()
            .find(|descriptor| descriptor.kind == PpuFuzzKind::Ordinary(kind))
            .map(|descriptor| descriptor.sequence_dependency)
            .expect("instruction must have a generation descriptor")
    };

    assert_eq!(
        dependency(PpuInstructionKind::Ori),
        Some(PpuSequenceDependency::GeneralPurposeRegister)
    );
    assert_eq!(dependency(PpuInstructionKind::Addi), None);
    assert_eq!(dependency(PpuInstructionKind::Lfs), None);
}

#[test]
fn generated_witnesses_and_structural_operations_preserve_exact_kind() {
    let mut saw_alias = false;
    let mut saw_immediate_boundary = false;
    let mut saw_reserved_bit = false;
    for descriptor in generation_descriptors() {
        let operand_mask = descriptor
            .operands
            .iter()
            .fold(0, |mask, field| mask | field.mask);
        let decoded = crate::decode::decode(descriptor.canonical_word)
            .expect("canonical generation word must decode");
        assert_eq!(
            decoded.fuzz_descriptor(descriptor.canonical_word).kind,
            descriptor.kind
        );
        let mut classified = 0;
        for field in &descriptor.operands {
            assert_ne!(field.mask, 0, "empty operand for {:?}", descriptor.kind);
            assert_eq!(
                classified & field.mask,
                0,
                "overlapping operands for {:?}",
                descriptor.kind
            );
            classified |= field.mask;
        }
        assert_eq!(
            descriptor.encode(&descriptor.canonical_parameters()),
            Ok(descriptor.canonical_word)
        );
        if let Some(field) = descriptor.operands.first() {
            let mut too_wide = descriptor.canonical_parameters();
            too_wide[0] = field.maximum() + 1;
            assert_eq!(
                descriptor.encode(&too_wide),
                Err(PpuGenerationError::InvalidOperands)
            );
        }
        for values in [
            descriptor.operands.iter().map(|_| 0).collect::<Vec<_>>(),
            descriptor
                .operands
                .iter()
                .map(|field| field.maximum())
                .collect::<Vec<_>>(),
            descriptor
                .operands
                .iter()
                .enumerate()
                .map(|(index, field)| (index as u32 * 7 + 3) & field.maximum())
                .collect::<Vec<_>>(),
        ] {
            if let Ok(word) = descriptor.encode(&values) {
                assert_eq!(exact_kind(word), Some(descriptor.kind));
            }
        }
        if let Some(alias) = descriptor.alias_word(7) {
            saw_alias = true;
            for field in descriptor
                .operands
                .iter()
                .filter(|field| field.class == PpuOperandClass::Register)
            {
                assert_eq!(extract_bits(alias, field.mask), 7 & field.maximum());
            }
            assert_eq!(exact_kind(alias), Some(descriptor.kind));
        }
        let immediate_boundaries = descriptor.immediate_boundary_words();
        for word in &immediate_boundaries {
            saw_immediate_boundary = true;
            assert_eq!(exact_kind(*word), Some(descriptor.kind));
        }
        for (index, field) in descriptor.operands.iter().enumerate() {
            if field.class != PpuOperandClass::Immediate {
                continue;
            }
            for value in field.boundary_values() {
                let mut parameters = descriptor.canonical_parameters();
                parameters[index] = value;
                if let Ok(word) = descriptor.encode(&parameters) {
                    assert!(immediate_boundaries.contains(&word));
                }
            }
        }
        for word in descriptor.reserved_bit_words() {
            saw_reserved_bit = true;
            let changed_bits = word ^ descriptor.canonical_word;
            assert_eq!(changed_bits.count_ones(), 1);
            assert_eq!(changed_bits & operand_mask, 0);
            assert_eq!(exact_kind(word), Some(descriptor.kind));
        }
        for word in descriptor.shrink(descriptor.canonical_word) {
            let changed_bits = word ^ descriptor.canonical_word;
            assert_eq!(changed_bits.count_ones(), 1);
            assert_eq!(changed_bits & operand_mask, changed_bits);
            assert_eq!(word & !descriptor.canonical_word, 0);
            assert_eq!(exact_kind(word), Some(descriptor.kind));
        }
    }
    assert!(saw_alias);
    assert!(saw_immediate_boundary);
    assert!(saw_reserved_bit);
}

#[test]
fn typed_fields_follow_architected_form_layouts() {
    use PpuOperandClass as C;

    let cmpwi = generation_descriptor((11 << 26) | (3 << 23) | (4 << 16) | 7)
        .expect("cmpwi must have a generation descriptor");
    assert_eq!(
        cmpwi.operands,
        vec![
            PpuOperandField {
                class: C::Immediate,
                mask: 0x0000_ffff,
            },
            PpuOperandField {
                class: C::Register,
                mask: 0x001f_0000,
            },
            PpuOperandField {
                class: C::Condition,
                mask: 0x0380_0000,
            },
        ]
    );

    let bclr = generation_descriptor((19 << 26) | (20 << 21) | (3 << 16) | (16 << 1))
        .expect("bclr must have a generation descriptor");
    assert_eq!(
        bclr.operands,
        vec![
            PpuOperandField {
                class: C::Flag,
                mask: 0x0000_0001,
            },
            PpuOperandField {
                class: C::Selector,
                mask: 0x0000_1800,
            },
            PpuOperandField {
                class: C::Condition,
                mask: 0x001f_0000,
            },
            PpuOperandField {
                class: C::Condition,
                mask: 0x03e0_0000,
            },
        ]
    );

    let sradi = generation_descriptor((31 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (413 << 2))
        .expect("sradi must have a generation descriptor");
    assert!(sradi.operands.contains(&PpuOperandField {
        class: C::Immediate,
        mask: 0x0000_f802,
    }));

    let rldicl =
        generation_descriptor((30 << 26) | (4 << 21) | (3 << 16) | (5 << 11) | (6 << 6) | (1 << 5))
            .expect("rldicl must have a generation descriptor");
    assert!(rldicl.operands.contains(&PpuOperandField {
        class: C::Immediate,
        mask: 0x0000_f802,
    }));
    assert!(rldicl.operands.contains(&PpuOperandField {
        class: C::Immediate,
        mask: 0x0000_07e0,
    }));

    let vx_compare = generation_descriptor(
        (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | VxOp::Vcmpequb as u32,
    )
    .expect("vcmpequb must have a generation descriptor");
    assert_eq!(vx_compare.operands[0].class, C::Flag);
    assert_eq!(vx_compare.operands[0].mask, 0x0000_0400);

    let vector_splat = generation_descriptor(
        (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | VxOp::Vspltisb as u32,
    )
    .expect("vspltisb must have a generation descriptor");
    assert_eq!(
        vector_splat.operands,
        vec![
            PpuOperandField {
                class: C::Immediate,
                mask: 0x001f_0000,
            },
            PpuOperandField {
                class: C::Register,
                mask: 0x03e0_0000,
            },
        ]
    );

    let vsldoi =
        generation_descriptor((4 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (6 << 6) | 44)
            .expect("vsldoi must have a generation descriptor");
    assert!(vsldoi
        .reserved_bit_words()
        .contains(&(vsldoi.canonical_word ^ (1 << 10))));

    let fsqrts = generation_descriptor(
        (59 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (6 << 6) | (22 << 1),
    )
    .expect("fsqrts must have a generation descriptor");
    assert_eq!(
        fsqrts.operands,
        vec![
            PpuOperandField {
                class: C::Flag,
                mask: 0x0000_0001,
            },
            PpuOperandField {
                class: C::Register,
                mask: 0x0000_f800,
            },
            PpuOperandField {
                class: C::Register,
                mask: 0x03e0_0000,
            },
        ]
    );
    let reserved = fsqrts.reserved_bit_words();
    assert!(reserved.contains(&(fsqrts.canonical_word ^ (1 << 6))));
    assert!(reserved.contains(&(fsqrts.canonical_word ^ (1 << 16))));

    let mftb = generation_descriptor((31 << 26) | (3 << 21) | (12 << 16) | (8 << 11) | (339 << 1))
        .expect("mftb must have a generation descriptor");
    assert!(!mftb
        .reserved_bit_words()
        .contains(&(mftb.canonical_word ^ (1 << 6))));

    let fcmpu = generation_descriptor((63 << 26) | (3 << 23) | (4 << 16) | (5 << 11))
        .expect("fcmpu must have a generation descriptor");
    assert!(!fcmpu.operands.iter().any(|field| field.mask & 1 != 0));
    assert!(fcmpu
        .reserved_bit_words()
        .contains(&(fcmpu.canonical_word ^ 1)));
}

#[test]
fn structural_encoding_rejects_architecturally_invalid_register_relations() {
    let lwzu = generation_descriptor((33 << 26) | (3 << 21) | (4 << 16))
        .expect("lwzu must have a generation descriptor");
    assert_eq!(
        lwzu.encode(&[0, 0, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert_eq!(
        lwzu.encode(&[0, 3, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(lwzu.encode(&[0, 4, 3]).is_ok());
    let valid_lwzu = (33 << 26) | (3 << 21) | (4 << 16);
    assert!(!lwzu.shrink(valid_lwzu).contains(&(valid_lwzu & !(1 << 18))));

    let stwu = generation_descriptor((37 << 26) | (3 << 21) | (4 << 16))
        .expect("stwu must have a generation descriptor");
    assert_eq!(
        stwu.encode(&[0, 0, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(stwu.encode(&[0, 3, 3]).is_ok());

    let lfsu = generation_descriptor((49 << 26) | (3 << 21) | (4 << 16))
        .expect("lfsu must have a generation descriptor");
    assert_eq!(
        lfsu.encode(&[0, 0, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(lfsu.encode(&[0, 3, 3]).is_ok());

    let lmw = generation_descriptor((46 << 26) | (3 << 21) | (2 << 16))
        .expect("lmw must have a generation descriptor");
    assert_eq!(
        lmw.encode(&[0, 0, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert_eq!(
        lmw.encode(&[0, 3, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(lmw.encode(&[0, 2, 3]).is_ok());

    let bcctr = generation_descriptor((19 << 26) | (4 << 21) | (6 << 16) | (528 << 1))
        .expect("bcctr must have a generation descriptor");
    assert_eq!(
        bcctr.encode(&[0, 0, 6, 0]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(bcctr.encode(&[0, 0, 6, 4]).is_ok());

    let lswi = generation_descriptor((31 << 26) | (30 << 21) | (1 << 16) | (8 << 11) | (597 << 1))
        .expect("lswi must have a generation descriptor");
    assert_eq!(
        lswi.encode(&[8, 0, 30]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert_eq!(
        lswi.encode(&[8, 31, 30]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(lswi.encode(&[8, 1, 30]).is_ok());

    let lswx = generation_descriptor((31 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (533 << 1))
        .expect("lswx must have a generation descriptor");
    assert_eq!(
        lswx.encode(&[5, 3, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert_eq!(
        lswx.encode(&[3, 4, 3]),
        Err(PpuGenerationError::InvalidOperands)
    );
    assert!(lswx.encode(&[5, 4, 3]).is_ok());
}

#[test]
fn known_words_have_non_vacuous_exact_kind_simplifications() {
    for raw in [
        0x3860_0001,
        0x7c63_2214,
        0x8063_0004,
        0x4e80_0020,
        (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11),
        (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (6 << 6) | 42,
        (59 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (21 << 1),
        (63 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (21 << 1),
    ] {
        let instruction = crate::decode::decode(raw).expect("known word must decode");
        let kind = instruction.fuzz_descriptor(raw).kind;
        let simplified = simplify_encoding(raw);
        assert!(!simplified.is_empty(), "0x{raw:08x} did not simplify");
        for candidate in simplified {
            assert_eq!((raw ^ candidate).count_ones(), 1);
            assert_eq!(candidate & !raw, 0);
            let decoded = crate::decode::decode(candidate).expect("simplification must decode");
            assert_eq!(decoded.fuzz_descriptor(candidate).kind, kind);
        }
    }
}

#[test]
fn family_dispatch_kinds_include_the_exact_operation() {
    let vx = (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11);
    let va = (4 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (6 << 6) | 42;
    let fp59 = (59 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (21 << 1);
    let fp63 = (63 << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (21 << 1);
    assert_eq!(
        crate::decode::decode(vx)
            .expect("VX word must decode")
            .fuzz_descriptor(vx)
            .kind,
        PpuFuzzKind::Vx(VxOp::Vaddubm)
    );
    assert_eq!(
        crate::decode::decode(va)
            .expect("VA word must decode")
            .fuzz_descriptor(va)
            .kind,
        PpuFuzzKind::Va(VaOp::Vsel)
    );
    assert_eq!(
        crate::decode::decode(fp59)
            .expect("primary-59 word must decode")
            .fuzz_descriptor(fp59)
            .kind,
        PpuFuzzKind::Fp59(Fp59Op::Fadds)
    );
    assert_eq!(
        crate::decode::decode(fp63)
            .expect("primary-63 word must decode")
            .fuzz_descriptor(fp63)
            .kind,
        PpuFuzzKind::Fp63(Fp63Op::Fadd)
    );
}

#[test]
fn srdi_uses_the_synthetic_form_of_the_other_quickenings() {
    let instruction = PpuInstruction::Srdi { ra: 3, rs: 4, n: 5 };
    assert_eq!(
        instruction.fuzz_descriptor(30 << 26).form,
        PpuEncodingForm::Synthetic
    );
}

#[test]
fn load_reserve_descriptors_allow_both_emitted_effects() {
    for instruction in [
        PpuInstruction::Lwarx {
            rt: 3,
            ra: 4,
            rb: 5,
        },
        PpuInstruction::Ldarx {
            rt: 3,
            ra: 4,
            rb: 5,
        },
    ] {
        let effects = instruction.fuzz_descriptor(31 << 26).effects;
        assert_eq!(
            effects,
            &[EffectKind::SharedReadIntent, EffectKind::ReservationAcquire,]
        );
    }
}

#[test]
fn synthetic_memory_descriptors_match_their_fused_operations() {
    for instruction in [
        PpuInstruction::LwzCmpwi {
            rt: 3,
            ra_load: 4,
            offset: 0,
            bf: 0,
            cmp_imm: 0,
        },
        PpuInstruction::LwzMtlr {
            rt: 3,
            ra_load: 4,
            offset: 0,
        },
        PpuInstruction::LdMtlr {
            rt: 3,
            ra_load: 4,
            offset: 0,
        },
    ] {
        let descriptor = instruction.fuzz_descriptor(0);
        assert_eq!(descriptor.effects, &[EffectKind::SharedReadIntent]);
        assert_eq!(
            descriptor.outcomes,
            &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault]
        );
    }

    for instruction in [
        PpuInstruction::LiStw {
            rt: 3,
            imm: 1,
            ra_store: 4,
            store_offset: 0,
        },
        PpuInstruction::MflrStw {
            rt: 3,
            ra_store: 4,
            store_offset: 0,
        },
        PpuInstruction::MflrStd {
            rt: 3,
            ra_store: 4,
            store_offset: 0,
        },
        PpuInstruction::StdStd {
            rs1: 3,
            rs2: 4,
            ra: 5,
            offset1: 0,
        },
    ] {
        let descriptor = instruction.fuzz_descriptor(0);
        assert_eq!(descriptor.effects, &[EffectKind::SharedWriteIntent]);
        assert_eq!(
            descriptor.outcomes,
            &[
                PpuOutcomeClass::Continue,
                PpuOutcomeClass::MemoryFault,
                PpuOutcomeClass::BufferFull
            ]
        );
    }
}

#[test]
fn descriptors_reject_verdicts_the_executor_cannot_return() {
    let addi = PpuInstruction::Addi {
        rt: 3,
        ra: 4,
        imm: 1,
    };
    assert_eq!(
        addi.fuzz_descriptor(14 << 26).outcomes,
        &[PpuOutcomeClass::Continue]
    );

    let lwz = PpuInstruction::Lwz {
        rt: 3,
        ra: 4,
        imm: 0,
    };
    assert_eq!(
        lwz.fuzz_descriptor(32 << 26).outcomes,
        &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault]
    );

    let stw = PpuInstruction::Stw {
        rs: 3,
        ra: 4,
        imm: 0,
    };
    assert_eq!(
        stw.fuzz_descriptor(36 << 26).outcomes,
        &[
            PpuOutcomeClass::Continue,
            PpuOutcomeClass::MemoryFault,
            PpuOutcomeClass::BufferFull
        ]
    );

    let branch = PpuInstruction::Bc {
        bo: 20,
        bi: 0,
        offset: 4,
        aa: false,
        link: false,
    };
    assert_eq!(
        branch.fuzz_descriptor(16 << 26).outcomes,
        &[PpuOutcomeClass::Continue, PpuOutcomeClass::Branch]
    );

    let unconditional_branch = PpuInstruction::B {
        offset: 4,
        aa: false,
        link: false,
    };
    assert_eq!(
        unconditional_branch.fuzz_descriptor(18 << 26).outcomes,
        &[PpuOutcomeClass::Branch]
    );

    let syscall = PpuInstruction::Sc { lev: 0 };
    assert_eq!(
        syscall.fuzz_descriptor(17 << 26).outcomes,
        &[PpuOutcomeClass::Syscall]
    );

    let popcntb = PpuInstruction::Popcntb { ra: 3, rs: 4 };
    assert_eq!(
        popcntb.fuzz_descriptor(31 << 26).outcomes,
        &[PpuOutcomeClass::Fault]
    );
}
