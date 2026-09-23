use super::*;
use crate::ppu::execute::run_once;
use crate::ppu::generate::DATA_LEN;

#[test]
fn architecturally_undefined_ppu_state_is_classified_before_replay() {
    let mut state = PpuState::new();
    state.set_gpr(4, 7);
    state.set_gpr(5, 0);
    let instruction = PpuInstruction::Divd {
        rt: 3,
        ra: 4,
        rb: 5,
        oe: false,
        rc: false,
    };
    let descriptor = instruction.fuzz_descriptor(0);
    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        &instruction,
        &state,
        descriptor,
        &ExecuteVerdict::Continue,
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Undefined);
    assert!(assessment
        .reasons
        .contains(&EligibilityReason::ArchitecturallyUndefined));
    assert!(PpuInstruction::Divw {
        rt: 3,
        ra: 4,
        rb: 4,
        oe: false,
        rc: false,
    }
    .fuzz_case_is_architecturally_undefined(&state));
    assert!(PpuInstruction::Mfocrf { rt: 3, crm: 3 }.fuzz_case_is_architecturally_undefined(&state));
    state.set_gpr(5, 2);
    assert!(!instruction.fuzz_case_is_architecturally_undefined(&state));
}

#[test]
fn an_unexpected_fault_remains_eligible_for_contract_checks() {
    let instruction = PpuInstruction::Addi {
        rt: 3,
        ra: 4,
        imm: 1,
    };
    let descriptor = instruction.fuzz_descriptor(14 << 26);
    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        &instruction,
        &PpuState::new(),
        descriptor,
        &ExecuteVerdict::Fault(cellgov_ppu::exec::PpuFault::UnimplementedInstruction(14)),
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Eligible);
}

#[test]
fn raw_words_do_not_claim_an_intentional_fault_boundary() {
    let instruction = PpuInstruction::Popcntb { ra: 3, rs: 4 };
    let descriptor = instruction.fuzz_descriptor(31 << 26);
    let assessment = assess_instruction_case(
        GenerationStrategy::RawWords,
        &instruction,
        &PpuState::new(),
        descriptor,
        &ExecuteVerdict::Fault(cellgov_ppu::exec::PpuFault::UnimplementedInstruction(122)),
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Eligible);
    assert!(!assessment
        .features
        .contains(&CaseFeature::NamedFaultBoundary));
    assert!(assessment
        .reasons
        .contains(&EligibilityReason::DecoderRobustness));
}

#[test]
fn a_memory_fault_retracts_unmet_state_features() {
    let instruction = PpuInstruction::Lwz {
        rt: 3,
        ra: 0,
        imm: 0,
    };
    let state = PpuState::new();
    let observed = run_once(&instruction, &state, &[0; DATA_LEN]).unwrap();
    assert!(matches!(observed.verdict, ExecuteVerdict::MemFault(_)));
    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        &instruction,
        &state,
        instruction.fuzz_descriptor(32 << 26),
        &observed.verdict,
        [CaseFeature::MappedMemory, CaseFeature::Reservation]
            .into_iter()
            .collect(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Unsupported);
    assert!(!assessment.features.contains(&CaseFeature::MappedMemory));
    assert!(!assessment.features.contains(&CaseFeature::Reservation));
}

#[test]
fn a_structured_sequence_that_executed_nothing_is_an_unmet_precondition() {
    let structured = assess_sequence_case(GenerationStrategy::Structured, 0, BTreeSet::new());
    assert_eq!(structured.eligibility, CaseEligibility::Unsupported);
    assert!(structured
        .reasons
        .contains(&EligibilityReason::UnmetStatePrecondition));

    let raw = assess_sequence_case(GenerationStrategy::RawWords, 0, BTreeSet::new());
    assert_eq!(raw.eligibility, CaseEligibility::Eligible);
    assert!(raw.reasons.contains(&EligibilityReason::DecoderRobustness));
}
