use super::*;

use cellgov_ppu::observation::PpuArchitecturalState;

fn kind_of(word: u32) -> PpuFuzzKind {
    cellgov_ppu::decode::decode(word)
        .expect("test word decodes")
        .fuzz_descriptor(word)
        .kind
}

// [PPC-Book1 p:26 s:2.4.2 System Call Instruction] sc SC-form: OPCD 17, LEV at instruction bits 20:26, bit 30 set.
fn sc(lev: u32) -> u32 {
    (17 << 26) | (lev << 5) | 2
}

fn gprs(generated: &PpuGeneratedSequence) -> [u64; 32] {
    PpuArchitecturalState::capture(&generated.initial_state).gpr
}

#[test]
fn encoders_produce_the_documented_words() {
    // [PPC-Book1 p:51 s:3.3.8 Fixed-Point Arithmetic Instructions] addi D-form: OPCD 14, RT, RA, SI.
    assert_eq!(addi(3, 3, 1), 0x3863_0001);
    // [PPC-Book1 p:162 s:B.9] li rT,value is addi rT,0,value.
    assert_eq!(li(3, 7), 0x3860_0007);
    // [PPC-Book1 p:66 s:3.3.13 Fixed-Point Logical Instructions] ori D-form: OPCD 24, RS, RA, UI.
    assert_eq!(ori(3, 3, 0), 0x6063_0000);
    // [PPC-Book1 p:42 s:3.3.3 Fixed-Point Store Instructions] stw D-form: OPCD 36, RS, RA, D.
    assert_eq!(stw(3, 4, 0), 0x9064_0000);
    // [PPC-Book1 p:37 s:3.3 Fixed-Point Load Instructions] lwz D-form: OPCD 32, RT, RA, D.
    assert_eq!(lwz(6, 4, 0), 0x80c4_0000);
    // [PPC-Book1 p:41 s:3.3.3 Fixed-Point Store Instructions] sth D-form: OPCD 44, RS, RA, D.
    assert_eq!(sth(5, 4, 1), 0xb0a4_0001);
    // [PPC-Book2 p:25 s:3.3 Synchronization Instructions] stwcx. X-form: OPCD 31, XO 150, Rc=1.
    assert_eq!(stwcx(3, 4, 5), 0x7c64_292d);
    // [PPC-Book1 p:24 s:2.4.1 Branch Instructions] b I-form: OPCD 18, LI||0b00, AA=0, LK=0.
    assert_eq!(branch_relative(8), 0x4800_0008);
    assert_eq!(branch_relative(11), 0x4800_0008);
}

#[test]
fn all_lists_the_seven_families_in_campaign_order() {
    assert_eq!(
        PpuSequenceFamily::ALL,
        [
            PpuSequenceFamily::RegisterAlias,
            PpuSequenceFamily::OverlappingMemory,
            PpuSequenceFamily::StoreLoadForwarding,
            PpuSequenceFamily::Reservation,
            PpuSequenceFamily::ControlledBranch,
            PpuSequenceFamily::Quickening,
            PpuSequenceFamily::FusionInvalidation,
        ]
    );
    let distinct = PpuSequenceFamily::ALL.iter().collect::<BTreeSet<_>>();
    assert_eq!(distinct.len(), PpuSequenceFamily::ALL.len());
}

#[test]
fn case_index_selects_families_cyclically() {
    for (case_index, family) in [
        (0, PpuSequenceFamily::RegisterAlias),
        (6, PpuSequenceFamily::FusionInvalidation),
        (7, PpuSequenceFamily::RegisterAlias),
        (13, PpuSequenceFamily::FusionInvalidation),
        (14, PpuSequenceFamily::RegisterAlias),
        (u64::MAX - 1, PpuSequenceFamily::RegisterAlias),
        (u64::MAX, PpuSequenceFamily::OverlappingMemory),
    ] {
        assert_eq!(
            generate_dependency_sequence(3, case_index).family,
            family,
            "{case_index}"
        );
    }
}

#[test]
fn each_family_generates_its_documented_words_state_and_features() {
    let seed = 0x1137;
    let value: u16 = 0x1137;
    for (case_index, family) in PpuSequenceFamily::ALL.into_iter().enumerate() {
        let case_index = case_index as u64;
        let generated = generate_dependency_sequence(seed, case_index);
        assert_eq!(generated.case_index, case_index);
        assert_eq!(generated.family, family);
        assert_eq!(generated.data, vec![0; DATA_LEN]);
        assert_eq!(
            generated.reduction_boundaries,
            [PpuReductionBoundary {
                start: 0,
                end: generated.words.len()
            }]
        );
        assert_eq!(generated.assessment.eligibility, CaseEligibility::Eligible);
        assert_eq!(
            generated.assessment.reasons,
            BTreeSet::from([
                EligibilityReason::StatePreconditions,
                EligibilityReason::InterpreterContract
            ])
        );
        let gpr = gprs(&generated);
        assert_eq!(gpr[3], u64::from(value), "{family:?}");
        assert_eq!(gpr[4], DATA_BASE, "{family:?}");
        let mapped = [CaseFeature::MappedMemory, CaseFeature::DependencyChain];
        let (words, features, gpr5, reserved, mutation) = match family {
            PpuSequenceFamily::RegisterAlias => (
                vec![addi(3, 3, 1), addi(3, 3, 1)],
                BTreeSet::from([
                    CaseFeature::MappedMemory,
                    CaseFeature::OperandAlias,
                    CaseFeature::DependencyChain,
                ]),
                u64::from(value ^ 0x55),
                false,
                None,
            ),
            PpuSequenceFamily::OverlappingMemory => (
                vec![stw(3, 4, 0), sth(5, 4, 1)],
                BTreeSet::from(mapped),
                u64::from(value ^ 0x55),
                false,
                None,
            ),
            PpuSequenceFamily::StoreLoadForwarding => (
                vec![stw(3, 4, 0), lwz(6, 4, 0)],
                BTreeSet::from(mapped),
                u64::from(value ^ 0x55),
                false,
                None,
            ),
            PpuSequenceFamily::Reservation => (
                vec![stwcx(3, 4, 5), lwz(6, 4, 0)],
                BTreeSet::from([
                    CaseFeature::MappedMemory,
                    CaseFeature::Reservation,
                    CaseFeature::DependencyChain,
                ]),
                0,
                true,
                None,
            ),
            PpuSequenceFamily::ControlledBranch => (
                vec![branch_relative(8), li(3, value ^ 1), li(3, value)],
                BTreeSet::from([CaseFeature::MappedMemory, CaseFeature::ControlledFlow]),
                u64::from(value ^ 0x55),
                false,
                None,
            ),
            PpuSequenceFamily::Quickening => (
                vec![ori(3, 3, 0), addi(3, 3, 1)],
                BTreeSet::from(mapped),
                u64::from(value ^ 0x55),
                false,
                None,
            ),
            PpuSequenceFamily::FusionInvalidation => (
                vec![li(3, value), stw(3, 4, 0)],
                BTreeSet::from(mapped),
                u64::from(value ^ 0x55),
                false,
                Some(PpuCodeMutation {
                    word_index: 0,
                    replacement: li(3, value ^ 1),
                }),
            ),
        };
        assert_eq!(generated.words, words, "{family:?}");
        assert_eq!(generated.assessment.features, features, "{family:?}");
        assert_eq!(gpr[5], gpr5, "{family:?}");
        assert_eq!(
            generated.initial_state.reservation(),
            reserved.then(|| ReservedLine::containing(DATA_BASE)),
            "{family:?}"
        );
        assert_eq!(generated.code_mutation, mutation, "{family:?}");
    }
}

#[test]
fn generated_operand_value_never_reaches_zero() {
    assert_eq!(gprs(&generate_dependency_sequence(0, 0))[3], 1);
    assert_eq!(gprs(&generate_dependency_sequence(0x1_0000, 0))[3], 1);
    assert_eq!(gprs(&generate_dependency_sequence(0x1_0005, 0))[3], 5);
    assert_eq!(gprs(&generate_dependency_sequence(2, 0))[3], 2);
    let branch = generate_dependency_sequence(0, 4);
    assert_eq!(branch.words, [branch_relative(8), li(3, 0), li(3, 1)]);
    assert_eq!(gprs(&branch)[3], 1);
}

#[test]
fn seed_and_case_index_bits_change_the_generated_operands() {
    let first = generate_dependency_sequence(1, 4);
    let second = generate_dependency_sequence(2, 4);
    assert_eq!(first.family, second.family);
    assert_ne!(first.words, second.words);
    assert_ne!(gprs(&first)[3], gprs(&second)[3]);
    let alias_a = generate_dependency_sequence(1, 0);
    let alias_b = generate_dependency_sequence(2, 0);
    assert_eq!(alias_a.words, alias_b.words);
    assert_ne!(gprs(&alias_a)[3], gprs(&alias_b)[3]);
    assert_eq!(gprs(&generate_dependency_sequence(0, 1 << 62))[3], 0x8000);
    assert_eq!(
        gprs(&generate_dependency_sequence(0x1137, 7 << 47))[3],
        0x1130
    );
}

#[test]
fn replay_reproduces_the_same_runs_and_stop_class_twice() {
    for case_index in 0..PpuSequenceFamily::ALL.len() as u64 {
        let first = replay_dependency_sequence(generate_dependency_sequence(0x5eed, case_index))
            .expect("first replay");
        let second = replay_dependency_sequence(generate_dependency_sequence(0x5eed, case_index))
            .expect("second replay");
        assert_eq!(first.stop, second.stop, "{case_index}");
        assert_eq!(first.runs, second.runs, "{case_index}");
        assert_eq!(first.intended_opcodes, second.intended_opcodes);
        assert_eq!(first.executed_opcodes, second.executed_opcodes);
        assert_eq!(first.generated.words, second.generated.words);
        assert_eq!(first.runs.len(), 4);
    }
}

#[test]
fn replay_classifies_the_expected_stop_for_every_family() {
    for (case_index, family) in PpuSequenceFamily::ALL.into_iter().enumerate() {
        let replay = replay_dependency_sequence(generate_dependency_sequence(9, case_index as u64))
            .expect("family replays");
        let expected = if family == PpuSequenceFamily::ControlledBranch {
            PpuSequenceStopClass::ControlTransfer
        } else {
            PpuSequenceStopClass::Complete
        };
        assert_eq!(replay.stop, expected, "{family:?}");
        assert_eq!(replay.generated.family, family);
    }
}

#[test]
fn replay_names_fault_and_syscall_stops() {
    let mut faulting = generate_dependency_sequence(1, 0);
    faulting.words = vec![addi(3, 3, 1), lwz(6, 7, 0)];
    faulting.initial_state.set_gpr(7, 0x2000_0000);
    let fault = replay_dependency_sequence(faulting).expect("fault replays");
    assert_eq!(fault.stop, PpuSequenceStopClass::Fault);
    assert_eq!(fault.runs[0].stop.reason, YieldReason::Fault);

    let mut calling = generate_dependency_sequence(1, 0);
    calling.words = vec![sc(0), addi(3, 3, 1)];
    let syscall = replay_dependency_sequence(calling).expect("syscall replays");
    assert_eq!(syscall.stop, PpuSequenceStopClass::Syscall);
    assert_eq!(syscall.runs[0].stop.reason, YieldReason::Syscall);
    assert_eq!(syscall.executed_opcodes.get(&kind_of(addi(3, 3, 1))), None);
    assert_eq!(syscall.executed_opcodes.get(&kind_of(sc(0))), Some(&1));
}

#[test]
fn replay_names_other_when_an_uncontrolled_sequence_skips_words() {
    let mut skipping = generate_dependency_sequence(1, 0);
    assert!(!skipping
        .assessment
        .features
        .contains(&CaseFeature::ControlledFlow));
    skipping.words = vec![branch_relative(8), li(3, 1), li(3, 2)];
    let replay = replay_dependency_sequence(skipping).expect("skip replays");
    assert_eq!(replay.stop, PpuSequenceStopClass::Other);
    assert_eq!(replay.runs[0].executed_pcs, [0, 8, 12]);
    assert_eq!(replay.intended_opcodes.get(&kind_of(li(3, 1))), Some(&2));
    assert_eq!(replay.executed_opcodes.get(&kind_of(li(3, 1))), Some(&1));
    assert_eq!(
        replay.executed_opcodes.get(&kind_of(branch_relative(8))),
        Some(&1)
    );
    assert_eq!(replay.executed_opcodes.values().sum::<u64>(), 2);
}

#[test]
fn replay_names_complete_when_a_controlled_sequence_runs_in_order() {
    let mut sequential = generate_dependency_sequence(1, 4);
    assert!(sequential
        .assessment
        .features
        .contains(&CaseFeature::ControlledFlow));
    sequential.words = vec![li(3, 1), li(3, 2)];
    let replay = replay_dependency_sequence(sequential).expect("sequential replays");
    assert_eq!(replay.stop, PpuSequenceStopClass::Complete);
    assert_eq!(replay.runs[0].executed_pcs, [0, 4]);
}

#[test]
fn replay_counts_executed_opcodes_from_the_mutated_code() {
    let mut generated = generate_dependency_sequence(1, 6);
    assert_ne!(kind_of(ori(3, 3, 0)), kind_of(li(3, 1)));
    generated.code_mutation = Some(PpuCodeMutation {
        word_index: 0,
        replacement: ori(3, 3, 0),
    });
    let replay = replay_dependency_sequence(generated).expect("mutated replays");
    assert_eq!(replay.stop, PpuSequenceStopClass::Complete);
    assert_eq!(
        replay.intended_opcodes,
        BTreeMap::from([(kind_of(li(3, 1)), 1), (kind_of(stw(3, 4, 0)), 1)])
    );
    assert_eq!(
        replay.executed_opcodes,
        BTreeMap::from([(kind_of(ori(3, 3, 0)), 1), (kind_of(stw(3, 4, 0)), 1)])
    );
    assert_eq!(replay.runs[0].observation.state.gpr[3], 1);
}

#[test]
fn replay_refuses_an_undecodable_generated_word() {
    let undecodable = 0;
    let decode_error = cellgov_ppu::decode::decode(undecodable).expect_err("word is invalid");
    let mut generated = generate_dependency_sequence(1, 0);
    generated.words = vec![addi(3, 3, 1), undecodable];
    match replay_dependency_sequence(generated) {
        Err(PpuSequenceCampaignError::Decode(found)) => assert_eq!(found, decode_error),
        Err(other) => panic!("wrong refusal: {other}"),
        Ok(replay) => panic!("undecodable word replayed as {:?}", replay.stop),
    }
}

#[test]
fn replay_refuses_empty_generated_inputs_through_the_path_error() {
    let mut no_words = generate_dependency_sequence(1, 1);
    no_words.words.clear();
    assert!(matches!(
        replay_dependency_sequence(no_words),
        Err(PpuSequenceCampaignError::Path(PpuPathError::EmptySequence))
    ));
    let mut no_data = generate_dependency_sequence(1, 1);
    no_data.data.clear();
    assert!(matches!(
        replay_dependency_sequence(no_data),
        Err(PpuSequenceCampaignError::Path(PpuPathError::EmptyData))
    ));
}

#[test]
fn campaign_error_display_names_every_failure() {
    assert_eq!(
        PpuSequenceCampaignError::Path(PpuPathError::EmptySequence).to_string(),
        "PPU sequence campaign replay failed: \
         PPU path sequence must contain at least one instruction"
    );
    let decode_error = cellgov_ppu::decode::decode(0).expect_err("word is invalid");
    assert_eq!(
        PpuSequenceCampaignError::Decode(decode_error).to_string(),
        format!("PPU sequence opcode decode failed: {decode_error}")
    );
    assert_eq!(
        PpuSequenceCampaignError::TracePc { pc: 6, words: 2 }.to_string(),
        "PPU sequence trace PC 0x0000000000000006 is outside 2 words"
    );
    assert_eq!(
        PpuSequenceCampaignError::CounterOverflow {
            counter: "stop causes"
        }
        .to_string(),
        "PPU sequence campaign counter overflowed for stop causes"
    );
}

#[test]
fn counters_refuse_overflow_instead_of_wrapping() {
    let mut counts = BTreeMap::from([("a", u64::MAX - 1)]);
    increment(&mut counts, "a", "first").expect("one below the limit");
    assert_eq!(counts.get("a"), Some(&u64::MAX));
    assert!(matches!(
        increment(&mut counts, "a", "second"),
        Err(PpuSequenceCampaignError::CounterOverflow { counter: "second" })
    ));
    increment(&mut counts, "b", "third").expect("new key");
    assert_eq!(counts.get("b"), Some(&1));

    let mut target = BTreeMap::from([("a", 1u64), ("c", u64::MAX)]);
    merge_counts(&mut target, &BTreeMap::from([("a", 2), ("b", 3)]), "merge").expect("sums fit");
    assert_eq!(
        target,
        BTreeMap::from([("a", 3), ("b", 3), ("c", u64::MAX)])
    );
    assert!(matches!(
        merge_counts(&mut target, &BTreeMap::from([("c", 1)]), "overflow"),
        Err(PpuSequenceCampaignError::CounterOverflow {
            counter: "overflow"
        })
    ));
    merge_counts(&mut target, &BTreeMap::from([("c", 0)]), "zero").expect("zero fits");
}

#[test]
fn counts_groups_words_by_exact_kind() {
    assert_eq!(kind_of(addi(3, 3, 1)), kind_of(li(3, 7)));
    let counted = counts([addi(3, 3, 1), li(3, 7), stw(3, 4, 0)]).expect("words decode");
    assert_eq!(
        counted,
        BTreeMap::from([(kind_of(addi(3, 3, 1)), 2), (kind_of(stw(3, 4, 0)), 1)])
    );
    assert!(counts([]).expect("nothing to count").is_empty());
    assert!(matches!(
        counts([addi(3, 3, 1), 0]),
        Err(PpuSequenceCampaignError::Decode(_))
    ));
}

#[test]
fn campaign_aggregates_each_case_in_index_order() {
    let empty = run_dependency_campaign(0x1137, 0).expect("empty campaign");
    assert!(empty.cases.is_empty());
    assert!(empty.intended_families.is_empty());
    assert!(empty.stop_causes.is_empty());
    assert!(empty.intended_opcodes.is_empty());
    assert!(empty.executed_opcodes.is_empty());

    let report = run_dependency_campaign(0x1137, 8).expect("eight cases");
    assert_eq!(report.cases.len(), 8);
    assert_eq!(
        report
            .cases
            .iter()
            .map(|case| case.generated.case_index)
            .collect::<Vec<_>>(),
        (0..8).collect::<Vec<_>>()
    );
    assert_eq!(
        report
            .intended_families
            .get(&PpuSequenceFamily::RegisterAlias),
        Some(&2)
    );
    assert!(PpuSequenceFamily::ALL[1..]
        .iter()
        .all(|family| report.intended_families.get(family) == Some(&1)));
    assert_eq!(
        report.stop_causes,
        BTreeMap::from([
            (PpuSequenceStopClass::Complete, 7),
            (PpuSequenceStopClass::ControlTransfer, 1),
        ])
    );
    let mut intended = BTreeMap::new();
    let mut executed = BTreeMap::new();
    for case in &report.cases {
        merge_counts(&mut intended, &case.intended_opcodes, "intended").expect("fits");
        merge_counts(&mut executed, &case.executed_opcodes, "executed").expect("fits");
    }
    assert_eq!(report.intended_opcodes, intended);
    assert_eq!(report.executed_opcodes, executed);
    assert_eq!(
        report.cases[7].generated.words,
        report.cases[0].generated.words
    );
}
