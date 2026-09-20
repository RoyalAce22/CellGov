use super::*;

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
