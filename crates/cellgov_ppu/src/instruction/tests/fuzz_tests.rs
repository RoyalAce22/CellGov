use super::*;

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
            &[PpuOutcomeClass::Continue, PpuOutcomeClass::BufferFull]
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
        &[PpuOutcomeClass::Continue, PpuOutcomeClass::BufferFull]
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
