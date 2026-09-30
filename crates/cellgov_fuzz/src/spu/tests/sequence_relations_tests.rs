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
        // A float row's precondition excludes whole lane classes, so fewer
        // of its draws compare.
        let floor = if relation.precondition.is_some() {
            DRAWS as u32 / 16
        } else {
            DRAWS as u32 / 2
        };
        assert!(
            matched > floor,
            "{:?}: only {matched} of {DRAWS} draws compared",
            relation.id
        );
        // An inexact row pins constants and needs every register its own.
        let needs_distinct = matches!(relation.float_class, SpuFloatClass::Inexact { .. });
        assert!(
            aliased > 0 || needs_distinct,
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
        let (mut named, mut compared) = (0, 0);
        for index in 0..DRAWS {
            let instance = draw(relation, index);
            let verdict = compare_relation(relation, &instance, index).expect("the row encodes");
            if verdict == RelationVerdict::Inapplicable {
                continue;
            }
            compared += 1;
            // Without the tail the same draw matches: the read is what exposes it.
            assert_eq!(
                verdict,
                RelationVerdict::Match,
                "{:?} draw {index}",
                relation.id
            );
            let excluded = relation.excluded_registers(&instance.assignment, relation.dead);
            let reads = reader_tail_reads(relation, &instance).expect("the tail encodes");
            assert!(
                reads.iter().all(|read| excluded.contains(read)),
                "{:?} draw {index}: read {reads:?}, excluded {excluded:?}",
                relation.id
            );
            named += u32::from(!reads.is_empty());
        }
        assert!(
            named > compared / 2,
            "{:?}: the tail named a register on only {named} of {compared} compared draws",
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
        // A measured row compares its value only by distance, so its fused
        // partner raises an FPSCR flag the sequence does not.
        _ if relation.float_class == SpuFloatClass::Inexact { ulp: None } => {
            Box::new(|partner, _| partner.state.fpscr |= 1)
        }
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

#[test]
fn every_precondition_is_drawn_on_both_sides_and_needed() {
    assert_eq!(check_preconditions(1449, DRAWS * 4).expect("draws run"), []);
}

/// The word `lane` of symbolic register `symbolic` at the start.
fn start_word(instance: &RelationInstance, symbolic: usize, lane: usize) -> u32 {
    lanes(&instance.start.regs[usize::from(instance.assignment[symbolic])])[lane]
}

#[test]
fn the_host_maximum_diverges_only_on_an_exponent_255_or_zero_pair_lane() {
    let relation = row(SpuSequenceRelationId::FloatMax);
    let (mut maximal, mut zero_pair) = (0, 0);
    for index in 0..DRAWS * 4 {
        let instance = draw(&relation, index);
        // Distinct registers, so a divergence comes from the lanes.
        let registers = &instance.assignment[0..3];
        if registers[0] == registers[1]
            || registers[0] == registers[2]
            || registers[1] == registers[2]
        {
            continue;
        }
        let verdict = compare_sides(&relation, &instance, index, relation.dead, &[], &|_, _| {})
            .expect("the row encodes");
        let RelationVerdict::Diverged(divergence) = verdict else {
            continue;
        };
        let rt = instance.assignment[3];
        for &(register, lane) in &divergence.differing_words {
            if register != rt {
                continue;
            }
            let lane = usize::from(lane);
            let (a, b) = (
                start_word(&instance, 1, lane),
                start_word(&instance, 2, lane),
            );
            let exponent = |word: u32| (word >> 23) & 0xFF;
            let is_maximal = exponent(a) == 255 || exponent(b) == 255;
            let is_zero_pair = exponent(a) == 0 && exponent(b) == 0;
            assert!(
                is_maximal || is_zero_pair,
                "draw {index} lane {lane}: a={a:#010x} b={b:#010x}"
            );
            maximal += u32::from(is_maximal);
            zero_pair += u32::from(is_zero_pair);
        }
    }
    assert!(
        maximal > 0 && zero_pair > 0,
        "{maximal} exponent-255 lanes, {zero_pair} zero pairs"
    );
}

#[test]
fn the_pick_precondition_rejects_a_zero_pair_lane_the_host_maximum_breaks() {
    let relation = row(SpuSequenceRelationId::FloatMax);
    let precondition = relation.precondition.expect("the pick rows have one");
    let mut instance = (0..DRAWS)
        .map(|index| draw(&relation, index))
        .find(|instance| precondition(&instance.start, &instance.assignment))
        .expect("a draw lies inside the precondition");
    // A positive denorm against negative zero: the host orders the denorm
    // above, the SPU compares the two equal.
    for (symbolic, word) in [(1, 0x0000_0001u32), (2, 0x8000_0000)] {
        let register = usize::from(instance.assignment[symbolic]);
        let mut value = instance.start.regs[register];
        value[0..4].copy_from_slice(&word.to_be_bytes());
        instance.start.set_reg(register, value);
    }
    assert!(!precondition(&instance.start, &instance.assignment));
    let verdict = compare_sides(&relation, &instance, 0, relation.dead, &[], &|_, _| {})
        .expect("the row encodes");
    assert!(
        matches!(verdict, RelationVerdict::Diverged(_)),
        "{verdict:?}"
    );
}

#[test]
fn a_word_that_does_not_decode_errors_in_the_program_and_ends_a_run_elsewhere() {
    let garbage: u32 = 0xafff_ffff;
    assert!(
        cellgov_spu::decode::decode(garbage).is_err(),
        "the test needs a word that does not decode"
    );
    const OUTSIDE: u32 = 0x1_0000;
    let bra = 0x060 << 23 | (OUTSIDE >> 2) << 7;
    assert_eq!(
        cellgov_spu::decode::decode(bra),
        Ok(cellgov_spu::instruction::SpuInstruction::Bra {
            address: (OUTSIDE >> 2) as i32
        })
    );
    let mut start = SpuState::new();
    start.ls[OUTSIDE as usize..OUTSIDE as usize + 4].copy_from_slice(&garbage.to_be_bytes());
    let relation = SpuSequenceRelationId::FloatMax;
    let observation =
        run_side(relation, &[bra], &start, false).expect("a word outside the program ends the run");
    assert_eq!(
        observation.state.pc,
        OUTSIDE.wrapping_sub(SEQUENCE_PROGRAM_BASE + 4)
    );
    assert!(matches!(
        run_side(relation, &[garbage], &start, false),
        Err(FuzzError::Invariant(
            InvariantError::UnencodableSequenceRelation { .. }
        ))
    ));
}

#[test]
fn a_relation_check_is_named_by_its_row_and_no_other_check_shares_the_name() {
    use CheckIdentity as C;
    let others = [
        C::PpuDecoder,
        C::SpuDecoder,
        C::PpuExecutor,
        C::SpuExecutor,
        C::DeterministicReplay,
        C::PpuRecordCr0,
        C::PpuRecordCr1,
        C::PpuRecordCr6,
        C::PpuOverflowEnable,
        C::SpuIgnoredField,
        C::SpuCountMasking,
        C::SpuShufbControlClass,
        C::SpuImmediateRegister,
        C::SpuCommutative,
        C::SpuSlotPermutation,
        C::SpuCompareBranch,
        C::LegalOutcome,
        C::LegalEffect,
        C::AllowedFootprint,
        C::ProgramCounter,
        C::ExternalReference,
    ];
    // A new check fails this match until the list above names it.
    for check in others {
        match check {
            C::SpuSequenceRelation(_) => panic!("the list holds only the other checks"),
            C::PpuDecoder
            | C::SpuDecoder
            | C::PpuExecutor
            | C::SpuExecutor
            | C::DeterministicReplay
            | C::PpuRecordCr0
            | C::PpuRecordCr1
            | C::PpuRecordCr6
            | C::PpuOverflowEnable
            | C::SpuIgnoredField
            | C::SpuCountMasking
            | C::SpuShufbControlClass
            | C::SpuImmediateRegister
            | C::SpuCommutative
            | C::SpuSlotPermutation
            | C::SpuCompareBranch
            | C::LegalOutcome
            | C::LegalEffect
            | C::AllowedFootprint
            | C::ProgramCounter
            | C::ExternalReference => {}
        }
    }
    let names: std::collections::BTreeSet<String> = others.iter().map(C::name).collect();
    assert_eq!(names.len(), others.len());
    for relation in sequence_relations() {
        let name = C::SpuSequenceRelation(relation.id).name();
        assert_eq!(name, format!("{:?}", relation.id));
        assert!(!names.contains(&name), "{name} names another check");
    }
}

/// The symbolic register a sequence-partner defect corrupts: the first
/// write of a fused partner for the write defect, the last write of any
/// partner for the lane defect; none when the defect does not apply.
fn corrupted(relation: &SpuSequenceRelation, defect: crate::seeded::SeededDefect) -> Option<u8> {
    use crate::seeded::SeededDefect;
    match (defect, relation.partner) {
        (SeededDefect::SequencePartnerWrite, SpuSequencePartner::Fused(fused)) => {
            fused.writes.first().copied()
        }
        (SeededDefect::SequencePartnerLane, SpuSequencePartner::Fused(fused)) => {
            fused.writes.last().copied()
        }
        (SeededDefect::SequencePartnerLane, SpuSequencePartner::Guest(words)) => {
            words.last().map(|word| word.rt)
        }
        _ => None,
    }
}

#[test]
fn each_sequence_partner_defect_is_caught_by_every_row_it_reaches() {
    use crate::seeded::{seed, SeededDefect};
    for defect in [
        SeededDefect::SequencePartnerWrite,
        SeededDefect::SequencePartnerLane,
    ] {
        let _guard = seed(defect);
        let mut missed = Vec::new();
        for relation in sequence_relations() {
            // A row that only measures a register cannot see any change in
            // it: that gap is the row's claim, and the rule names it here.
            let reached = corrupted(relation, defect).is_some_and(|register| {
                !(relation.float_class == SpuFloatClass::Inexact { ulp: None }
                    && relation.approximate.contains(&register))
            });
            let caught = (0..DRAWS).any(|index| {
                matches!(
                    compare_relation(relation, &draw(relation, index), index)
                        .expect("the row encodes"),
                    RelationVerdict::Diverged(_)
                )
            });
            if reached != caught {
                missed.push((relation.id, reached, caught));
            }
        }
        assert_eq!(missed, [], "{defect:?}: (row, reached, caught)");
    }
}

#[test]
fn every_inexact_row_reports_a_ulp_bound_within_its_claim() {
    for relation in sequence_relations() {
        let SpuFloatClass::Inexact { ulp } = relation.float_class else {
            continue;
        };
        let measured = measured_ulp(relation, 1449, DRAWS)
            .expect("draws run")
            .unwrap_or_else(|| panic!("{:?}: no draw compared", relation.id));
        if let Some(bound) = ulp {
            assert!(measured <= bound, "{:?}: {measured} ulp", relation.id);
        }
    }
}

#[test]
fn the_square_root_chain_leaves_a_host_square_root_exactly_on_a_negative_lane() {
    let relation = row(SpuSequenceRelationId::SquareRoot);
    let mut diverged = 0;
    for index in 0..DRAWS * 4 {
        let instance = draw(&relation, index);
        let mut registers = instance.assignment.clone();
        registers.sort_unstable();
        registers.dedup();
        let in_range = (0..4).all(|lane| {
            let exponent = (start_word(&instance, 1, lane) >> 23) & 0xFF;
            (1..=254).contains(&exponent)
        });
        if registers.len() < instance.assignment.len() || !in_range {
            continue;
        }
        let negative = (0..4).any(|lane| start_word(&instance, 1, lane) >> 31 == 1);
        let verdict = compare_sides(&relation, &instance, index, relation.dead, &[], &|_, _| {})
            .expect("the row encodes");
        let is_diverged = matches!(verdict, RelationVerdict::Diverged(_));
        assert_eq!(is_diverged, negative, "draw {index}");
        diverged += u32::from(is_diverged);
    }
    assert!(diverged > 0);
}
