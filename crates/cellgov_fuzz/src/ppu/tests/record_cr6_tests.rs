use super::*;
use cellgov_ppu::instruction::fuzz::PpuFuzzKind;
use cellgov_ppu::instruction::ops::VxOp;

use crate::campaign::{CampaignSchedule, CaseRange};

#[test]
fn a_recording_vcmpequw_changes_only_cr6() {
    let run = run_instructions(FuzzConfig {
        seed: 11,
        strategy: GenerationStrategy::Structured,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 2_596,
                count: 1,
            },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    });
    assert_eq!(
        run.report.instruction_kinds,
        BTreeSet::from([InstructionIdentity::Ppu(PpuFuzzKind::Vx(VxOp::Vcmpequw))])
    );
    assert_eq!(
        run.report
            .metamorphic_executions
            .get(&CheckIdentity::PpuRecordCr6),
        Some(&1)
    );
    assert!(
        run.report.finding_counts.is_empty(),
        "{:?}",
        run.report.finding_counts
    );
}
