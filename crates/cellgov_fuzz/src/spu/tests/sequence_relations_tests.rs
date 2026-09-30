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
fn every_row_holds_over_aliased_and_equal_lane_instantiations() {
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

/// The result-only row with the dead set `dead` in place of its own.
fn result_only(dead: &'static [u8]) -> SpuSequenceRelation {
    SpuSequenceRelation {
        dead,
        ..row(SpuSequenceRelationId::CeqNotEqualResultOnly)
    }
}

#[test]
fn a_result_only_partner_fails_with_no_dead_set_and_holds_with_its_intermediate_dead() {
    let complete = result_only(&[]);
    let mut diverged = 0;
    for index in 0..DRAWS {
        let instance = draw(&complete, index);
        let c = instance.assignment[0];
        if let RelationVerdict::Diverged(divergence) =
            compare_relation(&complete, &instance, index).expect("the row encodes")
        {
            diverged += 1;
            assert_eq!(
                divergence.first_component,
                SpuObservationComponent::Registers
            );
            assert!(
                divergence
                    .bit_distance
                    .iter()
                    .all(|(register, _)| *register == c),
                "only c differs: {:?}",
                divergence.bit_distance
            );
        }
        // With `c` dead, the same draw matches.
        let row = row(SpuSequenceRelationId::CeqNotEqualResultOnly);
        assert_eq!(
            compare_relation(&row, &instance, index).expect("the row encodes"),
            RelationVerdict::Match,
            "draw {index}: {:?}",
            instance.assignment
        );
    }
    assert!(
        diverged > DRAWS / 2,
        "only {diverged} of {DRAWS} draws diverged"
    );
}

#[test]
fn a_dead_set_one_register_too_large_is_reported_and_the_catalog_is_minimal() {
    // A is an input nothing writes: neither side can leave it stale.
    let oversized = result_only(&[0, 1]);
    assert_eq!(
        unneeded_dead_registers(&oversized, 1447, DRAWS).expect("draws run"),
        [1]
    );
    assert_eq!(
        unneeded_dead_registers(&result_only(&[0]), 1447, DRAWS).expect("draws run"),
        [0u8; 0]
    );
    assert_eq!(check_dead_sets(1447, DRAWS).expect("draws run"), []);
}

#[test]
fn a_reader_tail_names_no_register_a_partner_writes_correctly() {
    // The full fused row and the guest row both write c; naming c dead is
    // too generous, and a read of c then sees the same value on both sides.
    for id in [
        SpuSequenceRelationId::CeqNotEqualFused,
        SpuSequenceRelationId::CeqNotEqualNor,
    ] {
        let generous = SpuSequenceRelation {
            dead: &[0],
            ..row(id)
        };
        for index in 0..DRAWS {
            let instance = draw(&generous, index);
            assert_eq!(
                reader_tail_reads(&generous, &instance).expect("the tail encodes"),
                [0u8; 0],
                "{id:?} draw {index}: {:?}",
                instance.assignment
            );
        }
    }
}

#[test]
fn a_reader_tail_makes_every_dead_set_row_diverge_on_the_register_it_read() {
    let with_dead: Vec<_> = sequence_relations()
        .iter()
        .filter(|relation| !relation.dead.is_empty())
        .collect();
    assert!(!with_dead.is_empty());
    for relation in with_dead {
        let mut named = 0;
        for index in 0..DRAWS {
            let instance = draw(relation, index);
            let excluded = relation.excluded_registers(&instance.assignment, relation.dead);
            let reads = reader_tail_reads(relation, &instance).expect("the tail encodes");
            assert!(
                reads.iter().all(|read| excluded.contains(read)),
                "{:?} draw {index}: read {reads:?}, excluded {excluded:?}",
                relation.id
            );
            named += u32::from(!reads.is_empty());
            // Without the tail the same draw matches: the read is what exposes it.
            assert_eq!(
                compare_relation(relation, &instance, index).expect("the row encodes"),
                RelationVerdict::Match
            );
        }
        assert!(
            named > DRAWS as u32 / 2,
            "{:?}: the tail named a register on only {named} of {DRAWS} draws",
            relation.id
        );
    }
}
