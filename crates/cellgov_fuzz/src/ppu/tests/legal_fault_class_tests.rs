//! The legal-outcome check reads the class of a fault. A fault whose
//! class the descriptor leaves out is a finding, even when the
//! descriptor admits a fault outcome.

use super::*;
use cellgov_ppu::instruction::fuzz::PpuFuzzDescriptor;
use cellgov_ppu::PpuFaultClass;

use crate::case::EligibilityReason;

/// `tw 31,r0,r0`: a trap whose condition always holds.
const TW_ALWAYS: u32 = (31 << 26) | (31 << 21) | (4 << 1);

/// Checks the always-taken `tw` against `descriptor`, and returns the
/// asymmetry and the report the check wrote.
fn check(descriptor: PpuFuzzDescriptor) -> (CrossReferenceAsymmetry, FuzzReport) {
    let instruction = cellgov_ppu::decode::decode(TW_ALWAYS).unwrap();
    let initial = PpuState::new();
    let memory = vec![0; DATA_LEN];
    let baseline = run_once(&instruction, &initial, &memory).unwrap();
    assert_eq!(
        baseline.verdict,
        ExecuteVerdict::Fault(cellgov_ppu::exec::PpuFault::trap(31))
    );
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        [],
    );
    let config = FuzzConfig::default();
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        config.seed,
        config.strategy,
        config.retention,
        config.max_findings as usize,
        config.sequence_words,
    );
    let case = InstructionCase {
        run: MetamorphicRun {
            instruction: &instruction,
            descriptor,
            initial: &initial,
            memory: &memory,
            raw: TW_ALWAYS,
            identity: InstructionIdentity::Ppu(descriptor.kind),
            iteration: 0,
            baseline: &baseline,
        },
        assessment: &assessment,
    };
    let asymmetry = case.check_legal_outcome(&mut report).unwrap();
    (asymmetry, report)
}

#[test]
fn a_trap_its_descriptor_names_is_no_finding() {
    let descriptor = cellgov_ppu::decode::decode(TW_ALWAYS)
        .unwrap()
        .fuzz_descriptor(TW_ALWAYS);
    assert_eq!(descriptor.faults, [PpuFaultClass::ProgramTrap]);
    let (asymmetry, report) = check(descriptor);
    assert_eq!(asymmetry, CrossReferenceAsymmetry::None);
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}

#[test]
fn a_trap_under_a_descriptor_that_names_another_fault_class_is_an_illegal_outcome() {
    let mut descriptor = cellgov_ppu::decode::decode(TW_ALWAYS)
        .unwrap()
        .fuzz_descriptor(TW_ALWAYS);
    assert!(descriptor.outcomes.contains(&PpuOutcomeClass::Fault));
    descriptor.faults = &[PpuFaultClass::ProgramIllegal];
    let (asymmetry, report) = check(descriptor);
    assert_eq!(asymmetry, CrossReferenceAsymmetry::Fault);
    assert_eq!(
        report.finding_counts.get(&FindingKind::IllegalOutcome),
        Some(&1),
        "{:?}",
        report.finding_counts
    );
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    assert_eq!(
        report.findings[0].fingerprint.check,
        CheckIdentity::LegalOutcome
    );
}
