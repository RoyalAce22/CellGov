use super::bits::*;
use super::classify::*;
use super::fields::*;
use super::registry::*;
use super::support::*;
use super::types::*;
use crate::instruction::{SpuInstruction, SpuInstructionKind};
use cellgov_effects::EffectKind;
use cellgov_exec::operand::extract_bits;
use cellgov_ps3_abi::hw::spu;
use std::collections::BTreeSet;

#[test]
fn declared_spu_relations_have_executed_witnesses_and_detect_seeded_state_leaks() {
    use crate::observation::{SpuObservation, SpuObservationComponent};
    use crate::state::SpuState;
    use cellgov_event::UnitId;

    for (kind, relation) in [
        (
            SpuInstructionKind::Nop,
            SpuMetamorphicRelation::NopFalseTarget,
        ),
        (
            SpuInstructionKind::Rotqbyi,
            SpuMetamorphicRelation::RotateByteCountHighBit,
        ),
    ] {
        let descriptor = generation_descriptors()
            .into_iter()
            .find(|descriptor| descriptor.kind == kind)
            .expect("every declared relation needs a reachable instruction kind");
        let raw = if kind == SpuInstructionKind::Rotqbyi {
            let mut parameters = descriptor.canonical_parameters();
            let immediate = descriptor
                .operands
                .iter()
                .position(|field| field.class == SpuOperandClass::Immediate)
                .expect("rotation must carry an immediate");
            parameters[immediate] = 3;
            descriptor
                .encode(&parameters)
                .expect("three-byte rotation must encode")
        } else {
            descriptor.canonical_word
        };
        let instruction = crate::decode::decode(raw).expect("canonical word must decode");
        let case = instruction
            .metamorphic_case(raw, relation)
            .expect("relation must have a witness");
        assert_ne!(case.partner_word, raw);
        let partner = crate::decode::decode(case.partner_word).expect("partner must decode");
        let mut left = SpuState::new();
        left.regs[0] = std::array::from_fn(|index| index as u8 + 1);
        left.regs[1] = [0xa5; 16];
        let mut right = left.clone();
        let baseline = crate::exec::execute(&instruction, &mut left, UnitId::new(0));
        let alternate = crate::exec::execute(&partner, &mut right, UnitId::new(0));
        let baseline = SpuObservation::capture(&left, &baseline);
        let alternate = SpuObservation::capture(&right, &alternate);
        assert!(
            baseline.compare(&alternate).differences.is_empty(),
            "{kind:?}"
        );
        if kind == SpuInstructionKind::Rotqbyi {
            assert_eq!(
                baseline.state.regs[0][0], 4,
                "rotation must move nonzero bytes"
            );
            let wrong =
                crate::decode::decode(raw ^ 0x0000_4000).expect("adjacent count must decode");
            let mut wrong_state = SpuState::new();
            wrong_state.regs[0] = std::array::from_fn(|index| index as u8 + 1);
            let wrong_outcome = crate::exec::execute(&wrong, &mut wrong_state, UnitId::new(0));
            assert!(baseline
                .compare(&SpuObservation::capture(&wrong_state, &wrong_outcome))
                .differences
                .contains(&SpuObservationComponent::Registers));
        }
        let mut defective = alternate.clone();
        defective.state.regs[0][0] ^= 1;
        assert!(baseline
            .compare(&defective)
            .differences
            .contains(&SpuObservationComponent::Registers));
        assert!(matches!(
            instruction.metamorphic_case(raw ^ 0xffff_ffff, relation),
            Err(SpuRelationRefusal::InvalidPartner { .. })
        ));
        assert!(matches!(
            instruction.metamorphic_case(raw, SpuMetamorphicRelation::Deterministic),
            Err(SpuRelationRefusal::Undeclared { .. })
        ));
    }
}

#[test]
fn generation_registry_covers_every_instruction_kind() {
    let actual = generation_descriptors()
        .iter()
        .map(|descriptor| descriptor.kind)
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected_generation_kinds());
}

#[test]
fn every_supported_spu_kind_has_a_decodable_executable_witness() {
    use crate::state::SpuState;
    use cellgov_event::UnitId;

    let mut unreachable = BTreeSet::new();
    for descriptor in generation_descriptors() {
        let candidate = if descriptor.kind == SpuInstructionKind::Heq
            || descriptor.kind == SpuInstructionKind::Stop
        {
            unreachable.insert(descriptor.kind);
            continue;
        } else if let Some((index, field)) = descriptor
            .operands
            .iter()
            .enumerate()
            .find(|(_, field)| field.class == SpuOperandClass::Channel)
        {
            let mut operands = descriptor.canonical_parameters();
            operands[index] = *descriptor
                .channel_values
                .first()
                .expect("channel kind needs a supported selector")
                & field.maximum();
            descriptor
                .encode(&operands)
                .expect("supported channel selector must encode")
        } else {
            descriptor.canonical_word
        };
        assert!(
            !encoding_has_undefined_operands(candidate),
            "{:?}",
            descriptor.kind
        );
        assert!(
            encoding_execution_is_supported(candidate),
            "{:?}",
            descriptor.kind
        );
        let instruction = crate::decode::decode(candidate).expect("witness must decode");
        assert_eq!(SpuInstructionKind::from(instruction), descriptor.kind);
        let mut state = SpuState::new();
        let outcome = crate::exec::execute(&instruction, &mut state, UnitId::new(0));
        assert!(
            instruction
                .fuzz_descriptor()
                .outcomes
                .contains(&SpuOutcomeClass::from_outcome(&outcome)),
            "{:?}",
            descriptor.kind
        );
    }
    // HEQ lacks its source operands in the decoded model; STOP's signal is not emitted.
    // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] HEQ compares two source operands.
    // [SPU-ISA p:238 s:10 Control Instructions] STOP signals its encoded value externally.
    assert_eq!(
        unreachable,
        BTreeSet::from([SpuInstructionKind::Heq, SpuInstructionKind::Stop])
    );
}

#[test]
fn sequence_flow_keeps_state_and_control_ownership_in_the_descriptor() {
    let descriptors = generation_descriptors();
    let flow = |kind| {
        descriptors
            .iter()
            .find(|descriptor| descriptor.kind == kind)
            .map(|descriptor| descriptor.sequence_flow)
            .expect("instruction must have a generation descriptor")
    };

    assert_eq!(flow(SpuInstructionKind::Ai), SpuSequenceFlow::Linear);
    assert_eq!(
        flow(SpuInstructionKind::Lqd),
        SpuSequenceFlow::StateDependent
    );
    assert_eq!(
        flow(SpuInstructionKind::Br),
        SpuSequenceFlow::ControlTransfer
    );
    assert_eq!(flow(SpuInstructionKind::Stop), SpuSequenceFlow::Terminal);
    assert_eq!(
        flow(SpuInstructionKind::Heq),
        SpuSequenceFlow::StateDependent
    );
    let wrch = descriptors
        .iter()
        .find(|descriptor| descriptor.kind == SpuInstructionKind::Wrch)
        .expect("WRCH must have a generation descriptor");
    let mut parameters = wrch.canonical_parameters();
    let channel = wrch
        .operands
        .iter()
        .position(|field| field.class == SpuOperandClass::Channel)
        .expect("WRCH must expose its channel operand");
    parameters[channel] = u32::from(spu::MFC_CMD);
    let word = wrch
        .encode(&parameters)
        .expect("MFC command channel must preserve the WRCH kind");
    let mfc_command =
        generation_descriptor(word).expect("MFC command write must have a generation descriptor");
    assert_eq!(mfc_command.sequence_flow, SpuSequenceFlow::StateDependent);
}

#[test]
fn channel_operands_prefer_interpreter_owned_architected_selectors() {
    let descriptors = generation_descriptors();
    let channels = |kind| {
        descriptors
            .iter()
            .find(|descriptor| descriptor.kind == kind)
            .map(|descriptor| descriptor.channel_values)
            .expect("channel instruction must have a generation descriptor")
    };

    assert_eq!(
        channels(SpuInstructionKind::Rdch),
        &[
            spu::MFC_RD_TAG_STAT as u32,
            spu::MFC_RD_ATOMIC_STAT as u32,
            spu::SPU_RD_IN_MBOX as u32,
            spu::SPU_RD_MACH_STAT as u32,
        ]
    );
    assert_eq!(
        channels(SpuInstructionKind::Wrch),
        &[
            spu::MFC_LSA as u32,
            spu::MFC_EAH as u32,
            spu::MFC_EAL as u32,
            spu::MFC_SIZE as u32,
            spu::MFC_TAG_ID as u32,
            spu::MFC_CMD as u32,
            spu::MFC_WR_TAG_MASK as u32,
            spu::MFC_WR_TAG_UPDATE as u32,
        ]
    );
    assert_eq!(
        channels(SpuInstructionKind::Rchcnt),
        &[spu::SPU_RD_MACH_STAT as u32]
    );
    assert!(channels(SpuInstructionKind::Ai).is_empty());
}

#[test]
fn generated_witnesses_and_structural_operations_preserve_kind() {
    let mut saw_alias = false;
    let mut saw_immediate_boundary = false;
    let mut saw_reserved_bit = false;
    for descriptor in generation_descriptors() {
        let operand_mask = descriptor
            .operands
            .iter()
            .fold(0, |mask, field| mask | field.mask);
        let decoded = crate::decode::decode(descriptor.canonical_word)
            .expect("canonical SPU generation word must decode");
        assert_eq!(SpuInstructionKind::from(decoded), descriptor.kind);
        let classified = descriptor
            .operands
            .iter()
            .fold(0, |mask, field| mask | field.mask);
        let decoded_but_ignored_fields = match descriptor.kind {
            SpuInstructionKind::Nop => 0x0000_007f,
            SpuInstructionKind::Bi
            | SpuInstructionKind::Bisl
            | SpuInstructionKind::Biz
            | SpuInstructionKind::Binz
            | SpuInstructionKind::Bihz
            | SpuInstructionKind::Bihnz => 0x000c_0000,
            SpuInstructionKind::Heq => 0x001f_ffff,
            SpuInstructionKind::Hbr => 0x0010_ffff,
            SpuInstructionKind::Hbra | SpuInstructionKind::Hbrr => 0x01ff_ffff,
            SpuInstructionKind::Sync => 0x0010_0000,
            _ => 0,
        };
        assert_eq!(
            classified,
            active_bits(descriptor.canonical_word, decoded, descriptor.kind)
                | decoded_but_ignored_fields,
            "unclassified operand bits for {:?}",
            descriptor.kind
        );
        assert_eq!(
            descriptor.encode(&descriptor.canonical_parameters()),
            Ok(descriptor.canonical_word)
        );
        if let Some(field) = descriptor.operands.first() {
            let mut too_wide = descriptor.canonical_parameters();
            too_wide[0] = field.maximum() + 1;
            assert_eq!(
                descriptor.encode(&too_wide),
                Err(SpuGenerationError::InvalidOperands)
            );
        }
        let zero_values = descriptor.operands.iter().map(|_| 0).collect::<Vec<_>>();
        let mut probes = vec![zero_values.clone()];
        for (index, field) in descriptor.operands.iter().enumerate() {
            let mut values = zero_values.clone();
            values[index] = field.maximum();
            probes.push(values);
        }
        for values in probes {
            let word = descriptor
                .encode(&values)
                .expect("typed SPU operands must encode their selected kind");
            assert_eq!(exact_kind(word), Some(descriptor.kind));
        }
        if let Some(alias) = descriptor.alias_word(7) {
            saw_alias = true;
            for field in descriptor
                .operands
                .iter()
                .filter(|field| field.class == SpuOperandClass::Register)
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
            if field.class != SpuOperandClass::Immediate {
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
            assert_eq!(
                crate::decode::decode(word),
                crate::decode::decode(descriptor.canonical_word)
            );
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
fn stop_generation_uses_one_fourteen_bit_immediate() {
    let descriptor = generation_descriptor(0).expect("STOP must have a generation descriptor");

    assert_eq!(
        descriptor.operands,
        vec![SpuOperandField {
            class: SpuOperandClass::Immediate,
            mask: 0x0000_3fff,
        }]
    );
    assert_eq!(descriptor.operands[0].maximum(), 0x3fff);
    assert_eq!(descriptor.alias_word(7), None);
}

#[test]
fn sync_generation_separates_the_channel_sync_flag_from_reserved_bits() {
    let descriptor =
        generation_descriptor(0x0040_0000).expect("SYNC must have a generation descriptor");

    assert_eq!(
        descriptor.operands,
        vec![SpuOperandField {
            class: SpuOperandClass::Flag,
            mask: 0x0010_0000,
        }]
    );
    let syncc = descriptor
        .encode(&[1])
        .expect("the C option must preserve the SYNC kind");
    assert_eq!(syncc, descriptor.canonical_word | 0x0010_0000);
    assert!(!descriptor.reserved_bit_words().contains(&syncc));
}

#[test]
fn control_generation_keeps_architectural_fields_that_decode_ignores() {
    for (raw, kind, expected) in [
        (
            0x4020_0000,
            SpuInstructionKind::Nop,
            vec![(SpuOperandClass::Register, 0x0000_007f)],
        ),
        (
            0x7b00_0000,
            SpuInstructionKind::Heq,
            vec![
                (SpuOperandClass::Register, 0x0000_007f),
                (SpuOperandClass::Register, 0x0000_3f80),
                (SpuOperandClass::Register, 0x001f_c000),
            ],
        ),
        (
            0x1000_0000,
            SpuInstructionKind::Hbra,
            vec![
                (SpuOperandClass::Immediate, 0x0180_007f),
                (SpuOperandClass::Immediate, 0x007f_ff80),
            ],
        ),
        (
            0x1200_0000,
            SpuInstructionKind::Hbrr,
            vec![
                (SpuOperandClass::Immediate, 0x0180_007f),
                (SpuOperandClass::Immediate, 0x007f_ff80),
            ],
        ),
    ] {
        let descriptor =
            generation_descriptor(raw).expect("control kind must have a generation descriptor");
        assert_eq!(descriptor.kind, kind);
        let actual = descriptor
            .operands
            .iter()
            .map(|field| (field.class, field.mask))
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "wrong fields for {kind:?}");
    }
}

#[test]
fn indirect_branch_generation_rejects_the_reserved_interrupt_pair() {
    for raw in [
        0x3500_0000,
        0x3520_0000,
        0x2500_0000,
        0x2520_0000,
        0x2540_0000,
        0x2560_0000,
    ] {
        let descriptor =
            generation_descriptor(raw).expect("indirect branch must have a generation descriptor");
        let enable = descriptor
            .operands
            .iter()
            .position(|field| field.mask == 0x0004_0000)
            .expect("indirect branch must expose E");
        let disable = descriptor
            .operands
            .iter()
            .position(|field| field.mask == 0x0008_0000)
            .expect("indirect branch must expose D");
        assert_eq!(descriptor.operands[enable].class, SpuOperandClass::Flag);
        assert_eq!(descriptor.operands[disable].class, SpuOperandClass::Flag);

        let mut parameters = descriptor.canonical_parameters();
        parameters[enable] = 1;
        let enable_only = descriptor
            .encode(&parameters)
            .expect("the enable-only option must encode");
        assert!(!encoding_execution_is_supported(enable_only));
        parameters[enable] = 0;
        parameters[disable] = 1;
        let disable_only = descriptor
            .encode(&parameters)
            .expect("the disable-only option must encode");
        assert!(!encoding_execution_is_supported(disable_only));
        parameters[enable] = 1;
        let undefined = descriptor
            .encode(&parameters)
            .expect_err("the reserved interrupt pair must not encode");
        assert_eq!(undefined, SpuGenerationError::InvalidOperands);
        let raw_with_reserved_pair = descriptor.canonical_word | 0x000c_0000;
        assert!(encoding_has_undefined_operands(raw_with_reserved_pair));
    }
}

#[test]
fn hbr_generation_rejects_prefetch_with_a_nonzero_offset() {
    let descriptor =
        generation_descriptor(0x3580_0000).expect("HBR must have a generation descriptor");
    assert_eq!(
        descriptor
            .operands
            .iter()
            .map(|field| (field.class, field.mask))
            .collect::<Vec<_>>(),
        vec![
            (SpuOperandClass::Immediate, 0x0000_c07f),
            (SpuOperandClass::Register, 0x0000_3f80),
            (SpuOperandClass::Flag, 0x0010_0000),
        ]
    );

    assert!(descriptor.encode(&[1, 0, 0]).is_ok());
    assert!(descriptor.encode(&[0, 0, 1]).is_ok());
    assert_eq!(
        descriptor.encode(&[1, 0, 1]),
        Err(SpuGenerationError::InvalidOperands)
    );
    assert!(encoding_has_undefined_operands(
        descriptor.canonical_word | 0x0010_0001
    ));
}

#[test]
fn single_bit_shrinks_are_non_vacuous_and_preserve_kind() {
    for (raw, expected) in [
        (0x1c00_4103, SpuInstructionKind::Ai),
        (0x4020_0001, SpuInstructionKind::Nop),
        (0x3500_0180, SpuInstructionKind::Bi),
    ] {
        let instruction = crate::decode::decode(raw).expect("known word must decode");
        let kind = SpuInstructionKind::from(instruction);
        assert_eq!(kind, expected);
        let shrunk = shrink_instruction(raw);
        assert!(
            !shrunk.is_empty(),
            "known words with clearable bits must produce shrinks"
        );
        for candidate in shrunk {
            let decoded = crate::decode::decode(candidate).expect("shrink must decode");
            assert_eq!(SpuInstructionKind::from(decoded), kind);
        }
    }
}

#[test]
fn known_single_bit_simplifications_preserve_kind() {
    for (raw, bit_index, expected) in [
        (0x1c00_4103, 0, SpuInstructionKind::Ai),
        (0x4020_0001, 0, SpuInstructionKind::Nop),
        (0x3500_0180, 7, SpuInstructionKind::Bi),
    ] {
        let candidate = simplify_instruction_bit(raw, bit_index)
            .expect("known simplification must remain valid");
        assert_ne!(candidate, raw);
        let decoded = crate::decode::decode(candidate).expect("simplification must decode");
        assert_eq!(SpuInstructionKind::from(decoded), expected);
    }
    assert_eq!(simplify_instruction_bit(0x1c00_4103, 31), None);
    assert_eq!(simplify_instruction_bit(0x1c00_4103, u32::BITS), None);
}

#[test]
fn outcome_and_effect_contracts_are_instruction_specific() {
    let rdch = SpuInstruction::Rdch {
        rt: 0,
        channel: spu::SPU_RD_IN_MBOX,
    }
    .fuzz_descriptor();
    assert_eq!(rdch.effects, &[EffectKind::MailboxReceiveAttempt]);
    assert_eq!(rdch.outcomes, &[SpuOutcomeClass::Yield]);

    let wrch = SpuInstruction::Wrch {
        channel: spu::MFC_CMD,
        rt: 0,
    }
    .fuzz_descriptor();
    assert_eq!(
        wrch.effects,
        &[EffectKind::DmaEnqueue, EffectKind::ConditionalStore]
    );
    assert_eq!(
        wrch.outcomes,
        &[
            SpuOutcomeClass::Continue,
            SpuOutcomeClass::Yield,
            SpuOutcomeClass::MemoryRead,
            SpuOutcomeClass::Fault,
        ]
    );
    let input = wrch
        .state_input
        .expect("MFC command writes need an executable command value");
    assert_eq!(input.register, 0);
    assert_eq!(input.preferred, Some(spu::MFC_PUTLLC));
    assert_eq!(
        input.values,
        &[
            spu::MFC_PUT,
            spu::MFC_GET,
            spu::MFC_GETLLAR,
            spu::MFC_PUTLLC
        ]
    );

    let tag_update = SpuInstruction::Wrch {
        channel: spu::MFC_WR_TAG_UPDATE,
        rt: 7,
    }
    .fuzz_descriptor();
    let tag_update_input = tag_update
        .state_input
        .expect("tag-update writes need a defined request selector");
    assert_eq!(tag_update_input.register, 7);
    assert_eq!(
        tag_update_input.preferred,
        Some(spu::MFC_TAG_UPDATE_IMMEDIATE)
    );
    assert_eq!(
        tag_update_input.values,
        &[
            spu::MFC_TAG_UPDATE_IMMEDIATE,
            spu::MFC_TAG_UPDATE_ANY,
            spu::MFC_TAG_UPDATE_ALL,
        ]
    );

    let wrch_parameter = SpuInstruction::Wrch {
        channel: spu::MFC_LSA,
        rt: 0,
    }
    .fuzz_descriptor();
    assert_eq!(wrch_parameter.effects, &[]);
    assert_eq!(wrch_parameter.outcomes, &[SpuOutcomeClass::Continue]);

    let rchcnt = SpuInstruction::Rchcnt {
        rt: 0,
        channel: spu::SPU_RD_MACH_STAT,
    }
    .fuzz_descriptor();
    assert_eq!(rchcnt.effects, &[]);
    assert_eq!(rchcnt.outcomes, &[SpuOutcomeClass::Continue]);

    let unsupported = SpuInstruction::Rdch { rt: 0, channel: 0 }.fuzz_descriptor();
    assert_eq!(unsupported.effects, &[]);
    assert_eq!(unsupported.outcomes, &[SpuOutcomeClass::Fault]);
    assert!(!unsupported.decoded_execution_supported);

    let unmodeled_architected_channel = SpuInstruction::Wrch {
        channel: spu::SPU_WR_OUT_INTR_MBOX,
        rt: 0,
    }
    .fuzz_descriptor();
    assert!(!unmodeled_architected_channel.decoded_execution_supported);
    let unmodeled_outbound_mailbox = SpuInstruction::Wrch {
        channel: spu::SPU_WR_OUT_MBOX,
        rt: 0,
    }
    .fuzz_descriptor();
    assert!(!unmodeled_outbound_mailbox.decoded_execution_supported);
    assert!(wrch.decoded_execution_supported);
    assert!(
        !SpuInstruction::Heq
            .fuzz_descriptor()
            .decoded_execution_supported
    );
    let stop = SpuInstruction::Stop { signal: 0 }.fuzz_descriptor();
    assert!(!stop.decoded_execution_supported);

    assert_eq!(
        SpuInstruction::Br { offset: 0 }.fuzz_descriptor().outcomes,
        &[SpuOutcomeClass::Branch]
    );
    assert_eq!(
        SpuInstruction::Brz { rt: 0, offset: 0 }
            .fuzz_descriptor()
            .outcomes,
        &[SpuOutcomeClass::Continue, SpuOutcomeClass::Branch]
    );
    assert_eq!(
        SpuInstruction::Stop { signal: 0 }
            .fuzz_descriptor()
            .outcomes,
        &[SpuOutcomeClass::Yield]
    );
}
