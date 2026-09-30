use cellgov_spu::fuzz::SpuSequenceRelationId as Id;
use cellgov_spu::observation::SpuObservationComponent;

use super::*;

fn result(name: &str, verdict: FusedResultVerdict) -> FusedResult {
    FusedResult {
        name: name.to_owned(),
        relation: Id::CeqNotEqualFused,
        verdict,
    }
}

#[test]
fn a_diverging_result_state_is_reported_with_its_registers_and_gives_status_4() {
    let (report, code) = outcome(&[
        result("same", FusedResultVerdict::Match),
        result(
            "dropped",
            FusedResultVerdict::Diverged {
                first_component: SpuObservationComponent::Registers,
                registers: vec![3, 10],
            },
        ),
        result("aliased", FusedResultVerdict::Inapplicable),
    ]);
    assert_eq!(
        report,
        "relations-check: same CeqNotEqualFused: match\n\
         relations-check: dropped CeqNotEqualFused: diverges in Registers (r3, r10)\n\
         relations-check: aliased CeqNotEqualFused: inapplicable, the start state is outside \
         the precondition\n\
         relations-check: 3 result state(s): 1 match, 1 diverge, 1 inapplicable\n"
    );
    assert_eq!(code, CommandExitCode::new(exit_codes::DIVERGED));
}

#[test]
fn result_states_that_all_match_or_fall_outside_give_status_0() {
    let (report, code) = outcome(&[
        result("same", FusedResultVerdict::Match),
        result("aliased", FusedResultVerdict::Inapplicable),
    ]);
    assert!(report.ends_with("2 result state(s): 1 match, 0 diverge, 1 inapplicable\n"));
    assert_eq!(code, CommandExitCode::SUCCESS);
}

#[test]
fn a_divergence_outside_the_registers_names_only_its_component() {
    let (report, _) = outcome(&[result(
        "fell",
        FusedResultVerdict::Diverged {
            first_component: SpuObservationComponent::ProgramCounter,
            registers: Vec::new(),
        },
    )]);
    assert!(
        report.starts_with("relations-check: fell CeqNotEqualFused: diverges in ProgramCounter\n")
    );
}
