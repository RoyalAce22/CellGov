use super::*;

#[test]
fn generation_registry_covers_every_instruction_kind() {
    let actual = generation_descriptors()
        .iter()
        .map(|descriptor| descriptor.kind)
        .collect::<BTreeSet<_>>();

    assert_eq!(actual, expected_generation_kinds());
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
        assert!(descriptor.encode(&parameters).is_ok());
        parameters[enable] = 0;
        parameters[disable] = 1;
        assert!(descriptor.encode(&parameters).is_ok());
        parameters[enable] = 1;
        assert_eq!(
            descriptor.encode(&parameters),
            Err(SpuGenerationError::InvalidOperands)
        );
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
