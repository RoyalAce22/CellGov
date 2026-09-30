use super::*;

use cellgov_spu::fuzz::{SpuFloatClass, SpuFusedReference, SpuSequenceRelationId};

use crate::CAMPAIGN_VERSION;

const DRAWS: u64 = 256;

fn row(id: SpuSequenceRelationId) -> SpuSequenceRelation {
    *sequence_relations()
        .iter()
        .find(|row| row.id == id)
        .expect("every id has a row")
}

fn draw(relation: &SpuSequenceRelation, index: u64) -> RelationInstance {
    let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1446, index);
    instantiate(relation, &mut rng).expect("instantiation draws")
}

/// Whether A's preferred word equals B's in `instance`: the compare holds.
fn preferred_words_equal(instance: &RelationInstance) -> bool {
    let (a, b) = (instance.assignment[1], instance.assignment[2]);
    instance.start.regs[usize::from(a)][0..4] == instance.start.regs[usize::from(b)][0..4]
}

#[test]
fn both_rows_hold_over_aliased_and_equal_lane_instantiations() {
    for relation in sequence_relations() {
        let mut aliased = 0;
        let mut equal = 0;
        for index in 0..DRAWS {
            let instance = draw(relation, index);
            let mut distinct = instance.assignment.clone();
            distinct.sort_unstable();
            distinct.dedup();
            aliased += u32::from(distinct.len() < instance.assignment.len());
            equal += u32::from(preferred_words_equal(&instance));
            assert_eq!(
                compare_relation(relation, &instance, index).expect("the row encodes"),
                RelationVerdict::Match,
                "{:?} draw {index}: {:?}",
                relation.id,
                instance.assignment
            );
        }
        assert!(
            aliased > 0,
            "{:?}: no aliased assignment drawn",
            relation.id
        );
        assert!(equal > 0, "{:?}: the compare never held", relation.id);
    }
}

/// The fused compare with RT's word 1 left as it was.
fn wrong_lane(state: &mut SpuState, assignment: &[u8]) {
    let rt = usize::from(assignment[3]);
    let kept = state.regs[rt];
    let SpuSequencePartner::Fused(fused) = row(SpuSequenceRelationId::CeqNotEqualFused).partner
    else {
        unreachable!("the fused row has a fused partner");
    };
    (fused.apply)(state, assignment);
    let mut value = state.regs[rt];
    value[4..8].copy_from_slice(&kept[4..8]);
    state.set_reg(rt, value);
}

#[test]
fn a_fused_reference_that_writes_the_wrong_lane_names_the_registers_component() {
    let relation = SpuSequenceRelation {
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[0, 3],
            apply: wrong_lane,
        }),
        ..row(SpuSequenceRelationId::CeqNotEqualFused)
    };
    let mut diverged = 0;
    for index in 0..DRAWS {
        let instance = draw(&relation, index);
        let rt = instance.assignment[3];
        match compare_relation(&relation, &instance, index).expect("the row encodes") {
            RelationVerdict::Diverged(divergence) => {
                diverged += 1;
                assert_eq!(
                    divergence.first_component,
                    SpuObservationComponent::Registers
                );
                assert_eq!(divergence.relation, relation.id);
                assert_eq!(divergence.case_index, index);
                assert_eq!(divergence.assignment, instance.assignment);
                assert_eq!(*divergence.start_registers, *instance.start.regs.as_array());
                let (register, bits) = divergence.bit_distance[0];
                assert_eq!(divergence.bit_distance.len(), 1, "only RT differs");
                assert_eq!(register, rt);
                assert!((1..=32).contains(&bits), "{bits} bits in one word");
            }
            // A kept word can already hold the value the compare gives.
            RelationVerdict::Match => {}
            RelationVerdict::Inapplicable => panic!("the row has no precondition"),
        }
    }
    assert!(
        diverged > DRAWS / 2,
        "only {diverged} of {DRAWS} draws diverged"
    );
}

/// Admits a start state whose A preferred word is even.
fn a_is_even(state: &SpuState, assignment: &[u8]) -> bool {
    state.regs[usize::from(assignment[1])][3] & 1 == 0
}

/// The fused compare, wrong in every lane when A's preferred word is odd.
fn wrong_when_odd(state: &mut SpuState, assignment: &[u8]) {
    let odd = !a_is_even(state, assignment);
    let SpuSequencePartner::Fused(fused) = row(SpuSequenceRelationId::CeqNotEqualFused).partner
    else {
        unreachable!("the fused row has a fused partner");
    };
    (fused.apply)(state, assignment);
    if odd {
        let rt = usize::from(assignment[3]);
        state.set_reg(rt, state.regs[rt].map(|byte| !byte));
    }
}

#[test]
fn the_sampler_draws_both_sides_of_a_precondition_and_compares_only_inside_it() {
    let relation = SpuSequenceRelation {
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[0, 3],
            apply: wrong_when_odd,
        }),
        precondition: Some(a_is_even),
        float_class: SpuFloatClass::BitExactUnderPrecondition,
        ..row(SpuSequenceRelationId::CeqNotEqualFused)
    };
    let (mut inside, mut outside) = (0, 0);
    for index in 0..DRAWS {
        let instance = draw(&relation, index);
        let verdict = compare_relation(&relation, &instance, index).expect("the row encodes");
        if a_is_even(&instance.start, &instance.assignment) {
            inside += 1;
            assert_eq!(verdict, RelationVerdict::Match, "draw {index}");
        } else {
            outside += 1;
            assert_eq!(verdict, RelationVerdict::Inapplicable, "draw {index}");
        }
    }
    assert!(
        inside > 0 && outside > 0,
        "{inside} inside, {outside} outside"
    );
}

#[test]
fn each_spu_sequence_smoke_campaign_executes_every_relation_row_and_finds_nothing() {
    let campaigns: Vec<_> = crate::smoke::SMOKE_CAMPAIGNS
        .iter()
        .filter(|campaign| campaign.target == FuzzTarget::SpuSequence)
        .collect();
    assert_eq!(
        campaigns.len(),
        2,
        "one structured and one raw-word campaign"
    );
    for campaign in campaigns {
        let run = campaign.run();
        assert!(
            run.report.findings.is_empty(),
            "{}: {:?}",
            campaign.name,
            run.report.findings
        );
        assert!(run.report.sequence_relation_divergences.is_empty());
        for relation in sequence_relations() {
            let check = CheckIdentity::SpuSequenceRelation(relation.id);
            assert!(
                run.report
                    .metamorphic_executions
                    .get(&check)
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "{}: {:?} never ran: {:?}",
                campaign.name,
                relation.id,
                run.report.metamorphic_executions
            );
        }
    }
}
