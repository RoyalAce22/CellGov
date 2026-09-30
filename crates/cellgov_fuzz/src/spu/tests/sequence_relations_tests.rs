use super::*;

use cellgov_spu::fuzz::{SpuFloatClass, SpuFusedFlow, SpuFusedReference, SpuSequenceRelationId};

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

/// Whether the row's first word is an equality compare of symbolic
/// registers 1 and 2.
fn compares_for_equality(relation: &SpuSequenceRelation) -> bool {
    use cellgov_spu::instruction::SpuInstructionKind as K;
    let first = relation.sequence[0];
    matches!(first.kind, K::Ceq | K::Ceqh | K::Ceqb | K::Fceq | K::Fcmeq)
        && (first.ra, first.rb) == (1, 2)
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
        let mut matched = 0;
        for index in 0..DRAWS {
            let instance = draw(relation, index);
            let mut distinct = instance.assignment.clone();
            distinct.sort_unstable();
            distinct.dedup();
            let verdict = compare_relation(relation, &instance, index).expect("the row encodes");
            if verdict == RelationVerdict::Inapplicable {
                assert!(
                    relation.precondition.is_some(),
                    "{:?}: inapplicable with no precondition",
                    relation.id
                );
                continue;
            }
            assert_eq!(
                verdict,
                RelationVerdict::Match,
                "{:?} draw {index}: {:?}",
                relation.id,
                instance.assignment
            );
            matched += 1;
            aliased += u32::from(distinct.len() < instance.assignment.len());
            equal += u32::from(compares_for_equality(relation) && preferred_words_equal(&instance));
        }
        assert!(
            matched > DRAWS as u32 / 2,
            "{:?}: only {matched} of {DRAWS} draws compared",
            relation.id
        );
        assert!(
            aliased > 0,
            "{:?}: no aliased assignment drawn",
            relation.id
        );
        assert!(
            equal > 0 || !compares_for_equality(relation),
            "{:?}: the compare never held",
            relation.id
        );
    }
}

/// The fused compare with RT's word 1 left as it was.
fn wrong_lane(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
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
    SpuFusedFlow::FallThrough
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
fn wrong_when_odd(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
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
    SpuFusedFlow::FallThrough
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

/// The preferred word of symbolic register `symbolic` at the start.
fn preferred(instance: &RelationInstance, symbolic: usize) -> u32 {
    let value = instance.start.regs[usize::from(instance.assignment[symbolic])];
    u32::from_be_bytes([value[0], value[1], value[2], value[3]])
}

/// Whether the split rows' access, `y + 0x30 + 2*16`, crosses the local
/// store limit.
fn crosses_the_limit(instance: &RelationInstance) -> bool {
    let limit = instance.start.lslr();
    (preferred(instance, 1) & limit) + 0x50 > limit
}

#[test]
fn the_branch_rows_match_in_both_directions_and_the_split_rows_across_the_limit() {
    use SpuSequenceRelationId as Id;
    for id in [
        Id::BranchOrxBrz,
        Id::BranchOrxBrnz,
        Id::BranchOrxBiz,
        Id::BranchOrxBinz,
    ] {
        let relation = row(id);
        let (mut zero, mut nonzero) = (0, 0);
        for index in 0..DRAWS {
            let instance = draw(&relation, index);
            if compare_relation(&relation, &instance, index).expect("the row encodes")
                != RelationVerdict::Match
            {
                continue;
            }
            let v = instance.start.regs[usize::from(instance.assignment[1])];
            if v.iter().all(|byte| *byte == 0) {
                zero += 1;
            } else {
                nonzero += 1;
            }
        }
        assert!(
            zero > 0 && nonzero > 0,
            "{id:?}: {zero} zero, {nonzero} nonzero"
        );
    }
    for id in [Id::SplitAddressLoad, Id::SplitAddressStore] {
        let relation = row(id);
        let crossed = (0..DRAWS)
            .map(|index| (index, draw(&relation, index)))
            .filter(|(index, instance)| {
                crosses_the_limit(instance)
                    && compare_relation(&relation, instance, *index).expect("the row encodes")
                        == RelationVerdict::Match
            })
            .count();
        assert!(crossed > 0, "{id:?}: no matched draw crossed the limit");
    }
}

/// A seeded partner defect for `relation`: the change it makes, as a hook
/// on the partner's observation.
/// A partner defect a test seeds.
type Defect = Box<dyn Fn(&mut SpuObservation, &RelationInstance)>;

fn defect(relation: &SpuSequenceRelation) -> Defect {
    use SpuSequenceRelationId as Id;
    let real = |instance: &RelationInstance, symbolic: u8| {
        usize::from(instance.assignment[usize::from(symbolic)])
    };
    match (relation.id, relation.partner) {
        // A fused partner that forgets to write its first intermediate.
        (_, SpuSequencePartner::Fused(fused)) => {
            let first = fused.writes[0];
            Box::new(move |partner, instance| {
                let register = real(instance, first);
                partner.state.regs[register] = instance.start.regs[register];
            })
        }
        // A split load without the wrap: past the limit it reads nothing.
        (Id::SplitAddressLoad, _) => Box::new(move |partner, instance| {
            if crosses_the_limit(instance) {
                partner.state.regs[real(instance, 2)] = [0; 16];
            }
        }),
        // A split store without the wrap: past the limit it stores nothing.
        (Id::SplitAddressStore, _) => Box::new(move |partner, instance| {
            if crosses_the_limit(instance) {
                let at = ((preferred(instance, 1) & instance.start.lslr()) + 0x50)
                    & instance.start.lslr()
                    & !0xF;
                let at = at as usize;
                partner.state.ls[at..at + 16].copy_from_slice(&instance.start.ls[at..at + 16]);
            }
        }),
        // A guest partner whose result has one wrong lane.
        (Id::CeqNotEqualNor, _) => Box::new(move |partner, instance| {
            partner.state.regs[real(instance, 3)][4] ^= 0x5A;
        }),
        _ => Box::new(move |partner, instance| {
            partner.state.regs[real(instance, 2)][4] ^= 0x5A;
        }),
    }
}

#[test]
fn every_row_catches_its_seeded_partner_defect() {
    for relation in sequence_relations() {
        let hook = defect(relation);
        let diverged = (0..DRAWS)
            .filter(|&index| {
                let instance = draw(relation, index);
                matches!(
                    compare_under(relation, &instance, index, relation.dead, &[], &*hook)
                        .expect("the row encodes"),
                    RelationVerdict::Diverged(_)
                )
            })
            .count();
        assert!(
            diverged > 0,
            "{:?}: the seeded defect went unseen",
            relation.id
        );
    }
}

/// `fsm c,x; selb rt,a,b,c`: the mask comes from four bits of `x`'s
/// preferred word, not from a lane compare.
const FSM_SELECT: &[SpuSymbolicWord] = &[
    SpuSymbolicWord {
        kind: SpuInstructionKind::Fsm,
        rt: 0,
        ra: 1,
        rb: 0,
        rc: 0,
        imm: 0,
    },
    SpuSymbolicWord {
        kind: SpuInstructionKind::Selb,
        rt: 2,
        ra: 3,
        rb: 4,
        rc: 0,
        imm: 0,
    },
];

/// The compare-select fusion applied to the `fsm` mask as if `c` were a
/// word compare of `x` against zero.
fn compare_select_on_x(state: &mut SpuState, assignment: &[u8]) -> SpuFusedFlow {
    let at = |symbolic: usize| usize::from(assignment[symbolic]);
    let x = state.regs[at(1)];
    let mask: [u8; 16] = std::array::from_fn(|byte| {
        let word = byte / 4 * 4;
        if x[word..word + 4].iter().any(|b| *b != 0) {
            0xFF
        } else {
            0
        }
    });
    let (a, b) = (state.regs[at(3)], state.regs[at(4)]);
    state.set_reg(at(0), mask);
    state.set_reg(
        at(2),
        std::array::from_fn(|byte| if mask[byte] != 0 { b[byte] } else { a[byte] }),
    );
    SpuFusedFlow::FallThrough
}

#[test]
fn the_compare_select_fusion_does_not_apply_to_an_fsm_mask() {
    let relation = SpuSequenceRelation {
        sequence: FSM_SELECT,
        partner: SpuSequencePartner::Fused(SpuFusedReference {
            writes: &[0, 2],
            apply: compare_select_on_x,
        }),
        ..row(SpuSequenceRelationId::CeqNotEqualFused)
    };
    let diverged = (0..DRAWS)
        .filter(|&index| {
            let instance = draw(&relation, index);
            matches!(
                compare_relation(&relation, &instance, index).expect("the row encodes"),
                RelationVerdict::Diverged(_)
            )
        })
        .count();
    assert!(
        diverged > DRAWS as usize / 2,
        "the fsm select diverged on only {diverged} of {DRAWS} draws"
    );
}

#[test]
fn a_turn_redraws_past_the_precondition_and_counts_each_draw_outside_it() {
    let rows = sequence_relations();
    let position = rows
        .iter()
        .position(|row| row.id == SpuSequenceRelationId::Mpy32)
        .expect("the multiply row is in the catalog") as u64;
    let mut report = FuzzReport::new(
        FuzzTarget::SpuSequence,
        1448,
        crate::GenerationStrategy::Structured,
        crate::RetentionConfig::default(),
        16,
        4,
    );
    const TURNS: u64 = 64;
    for turn in 0..TURNS {
        let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1448, turn);
        let case = position + turn * rows.len() as u64;
        run_relation_check(&mut report, &mut rng, case).expect("the turn runs");
    }
    let check = CheckIdentity::SpuSequenceRelation(SpuSequenceRelationId::Mpy32);
    let executed = report
        .metamorphic_executions
        .get(&check)
        .copied()
        .unwrap_or(0);
    let outside = report
        .metamorphic_inapplicable
        .get(&check)
        .copied()
        .unwrap_or(0);
    assert!(outside > 0, "no draw fell outside the precondition");
    assert!(
        executed >= TURNS - 2,
        "only {executed} of {TURNS} turns compared; {outside} draws outside"
    );
    assert!(report.findings.is_empty(), "{:?}", report.findings);
}
