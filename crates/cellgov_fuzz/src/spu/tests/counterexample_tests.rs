use super::*;

use cellgov_spu::fuzz::SpuSequenceRelationId;

use crate::rng::Rng;
use crate::seeded::{seed, SeededDefect};
use crate::spu::sequence_relations::instantiate;
use crate::CAMPAIGN_VERSION;

/// The seeded counterexample the committed store holds.
const SEEDED: &str = "seeded-partner-write";

fn row(id: SpuSequenceRelationId) -> &'static SpuSequenceRelation {
    sequence_relations()
        .iter()
        .find(|row| row.id == id)
        .expect("every id has a row")
}

/// The first draw of `relation` that diverges under the active seeding.
fn diverging(relation: &SpuSequenceRelation) -> (RelationInstance, SpuObservationComponent) {
    (0..256)
        .find_map(|index| {
            let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1450, index);
            let instance = instantiate(relation, &mut rng).expect("instantiation draws");
            match compare_relation(relation, &instance, index).expect("the row encodes") {
                RelationVerdict::Diverged(divergence) => {
                    Some((instance, divergence.first_component))
                }
                RelationVerdict::Match | RelationVerdict::Inapplicable => None,
            }
        })
        .expect("a draw diverges under the seeded defect")
}

/// Whether a replay of `counterexample` diverges in its stored first
/// component.
fn reproduces(counterexample: &RelationCounterexample) -> bool {
    counterexample
        .replay()
        .expect("the row runs")
        .is_some_and(|divergence| divergence.first_component == counterexample.first_component)
}

fn nonzero_words(instance: &RelationInstance) -> usize {
    (0..SPU_REG_COUNT)
        .flat_map(|register| {
            instance.start.regs[register]
                .chunks_exact(4)
                .collect::<Vec<_>>()
        })
        .filter(|word| word.iter().any(|&byte| byte != 0))
        .count()
}

#[test]
fn a_counterexample_round_trips_through_json_and_rebuilds_its_start_state() {
    let relation = row(SpuSequenceRelationId::SplitAddressLoad);
    let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1450, 0);
    let instance = instantiate(relation, &mut rng).expect("instantiation draws");
    assert!(instance.start.ls.iter().any(|&byte| byte != 0));
    let counterexample = RelationCounterexample::from_instance(
        "round-trip".to_owned(),
        relation,
        &instance,
        SpuObservationComponent::LocalStore,
    );
    let parsed = RelationCounterexample::parse_json(&counterexample.to_json()).expect("parses");
    assert_eq!(parsed, counterexample);
    assert_eq!(parsed.instance(), instance);
}

#[test]
fn every_observation_component_round_trips_by_name() {
    for (component, name) in COMPONENTS {
        assert_eq!(component_name(component), name);
    }
    let mut names: Vec<_> = COMPONENTS.iter().map(|(_, name)| *name).collect();
    names.dedup();
    assert_eq!(names.len(), COMPONENTS.len());
}

/// Whether a parse refusal is the one a case expects.
type Refusal = fn(&CounterexampleError) -> bool;

#[test]
fn a_fixture_out_of_form_is_refused_by_its_field() {
    let relation = row(SpuSequenceRelationId::CeqNotEqualFused);
    let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1450, 1);
    let instance = instantiate(relation, &mut rng).expect("instantiation draws");
    let good = RelationCounterexample::from_instance(
        "form".to_owned(),
        relation,
        &instance,
        SpuObservationComponent::Registers,
    )
    .to_json();
    let register = format!("\"{}\": \"", instance.assignment[0]);
    let cases: [(&str, String, Refusal); 7] = [
        (
            "schema",
            good.replace("\"schema_version\": 1", "\"schema_version\": 2"),
            |error| matches!(error, CounterexampleError::Schema { found: 2 }),
        ),
        (
            "row",
            good.replace("\"CeqNotEqualFused\"", "\"NoSuchRow\""),
            |error| matches!(error, CounterexampleError::UnknownRow { .. }),
        ),
        (
            "component",
            good.replace("\"Registers\"", "\"Memory\""),
            |error| matches!(error, CounterexampleError::UnknownComponent { .. }),
        ),
        (
            "register past the file",
            good.replace(&register, "\"128\": \""),
            |error| matches!(error, CounterexampleError::Register { .. }),
        ),
        (
            "short hex",
            good.replace(&register, &format!("{register}0")),
            |error| matches!(error, CounterexampleError::Register { .. }),
        ),
        (
            "misaligned line",
            good.replace(
                "\"local_store\": {}",
                "\"local_store\": {\"0x00008\": \"00000000000000000000000000000001\"}",
            ),
            |error| matches!(error, CounterexampleError::Line { .. }),
        ),
        (
            "assignment",
            good.replacen("\"assignment\": [", "\"assignment\": [\n    1,", 1),
            |error| matches!(error, CounterexampleError::Assignment { .. }),
        ),
    ];
    for (label, text, expected) in cases {
        assert_ne!(text, good, "{label}: the edit applies");
        let error = RelationCounterexample::parse_json(&text).expect_err(label);
        assert!(expected(&error), "{label}: {error}");
    }
}

#[test]
fn reduction_zeroes_what_the_seeded_divergence_does_not_need() {
    let _guard = seed(SeededDefect::SequencePartnerWrite);
    let relation = row(SpuSequenceRelationId::CeqNotEqualFused);
    let (instance, component) = diverging(relation);
    let reduced = reduce_instance(relation, &instance, component).expect("the row runs");
    assert!(matches!(
        compare_relation(relation, &reduced, 0).expect("the row runs"),
        RelationVerdict::Diverged(divergence) if divergence.first_component == component
    ));
    for register in 0..SPU_REG_COUNT {
        if !reduced.assignment.contains(&(register as u8)) {
            assert_eq!(reduced.start.regs[register], [0; 16], "r{register}");
        }
    }
    assert!(nonzero_words(&reduced) < nonzero_words(&instance));
}

#[test]
fn reduction_clears_local_store_a_divergence_does_not_read() {
    let _guard = seed(SeededDefect::SequencePartnerLane);
    let relation = row(SpuSequenceRelationId::SplitAddressLoad);
    let (instance, component) = diverging(relation);
    assert!(instance.start.ls.iter().any(|&byte| byte != 0));
    let reduced = reduce_instance(relation, &instance, component).expect("the row runs");
    assert!(reduced.start.ls.iter().all(|&byte| byte == 0));
    let counterexample =
        RelationCounterexample::from_instance("lane".to_owned(), relation, &reduced, component);
    assert!(counterexample.local_store.is_empty());
    assert!(reproduces(&counterexample));
}

#[test]
fn the_stored_seeded_counterexample_replays_to_its_finding_only_under_its_defect() {
    let stored = stored_counterexamples().expect("the committed store parses");
    let seeded = stored
        .iter()
        .find(|counterexample| counterexample.name == SEEDED)
        .expect("the store holds the seeded counterexample");
    assert_eq!(seeded.replay().expect("the row runs"), None);
    let _guard = seed(SeededDefect::SequencePartnerWrite);
    assert!(reproduces(seeded));
}

#[test]
fn every_stored_counterexample_replays_clean_without_a_seeded_defect() {
    for counterexample in stored_counterexamples().expect("the committed store parses") {
        assert_eq!(
            counterexample.replay().expect("the row runs"),
            None,
            "{} still separates its row",
            counterexample.name
        );
    }
}

#[test]
fn a_store_with_a_duplicate_name_is_refused() {
    let relation = row(SpuSequenceRelationId::CeqNotEqualFused);
    let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1450, 2);
    let instance = instantiate(relation, &mut rng).expect("instantiation draws");
    let fixture = RelationCounterexample::from_instance(
        "twice".to_owned(),
        relation,
        &instance,
        SpuObservationComponent::Registers,
    )
    .to_json();
    let store = format!("{{\"counterexamples\": [{fixture}, {fixture}]}}");
    assert!(matches!(
        parse_store(&store),
        Err(CounterexampleError::DuplicateName { .. })
    ));
}

#[test]
fn the_stored_seeded_counterexample_is_the_reduction_of_its_first_diverging_draw() {
    let stored = stored_counterexamples().expect("the committed store parses");
    let seeded = stored
        .iter()
        .find(|counterexample| counterexample.name == SEEDED)
        .expect("the store holds the seeded counterexample");
    let _guard = seed(SeededDefect::SequencePartnerWrite);
    let relation = row(SpuSequenceRelationId::CeqNotEqualFused);
    let (instance, component) = diverging(relation);
    let reduced = reduce_instance(relation, &instance, component).expect("the row runs");
    let regenerated =
        RelationCounterexample::from_instance(SEEDED.to_owned(), relation, &reduced, component);
    assert_eq!(
        &regenerated,
        seeded,
        "the regenerated fixture, to copy into the store:\n{}",
        regenerated.to_json()
    );
}

#[test]
fn reduction_keeps_the_words_a_divergence_needs() {
    // mpy32's partner drops t1 = mpyh(a, b): the rows differ only while
    // that product differs from t1's start value, so an all-zero state
    // matches and the reducer must refuse it.
    let _guard = seed(SeededDefect::SequencePartnerWrite);
    let relation = row(SpuSequenceRelationId::Mpy32);
    let (instance, component) = diverging(relation);
    let mut zeroed = instance.clone();
    for register in 0..SPU_REG_COUNT {
        zeroed.start.set_reg(register, [0; 16]);
    }
    assert_eq!(
        compare_relation(relation, &zeroed, 0).expect("the row runs"),
        RelationVerdict::Match
    );
    let reduced = reduce_instance(relation, &instance, component).expect("the row runs");
    assert!(matches!(
        compare_relation(relation, &reduced, 0).expect("the row runs"),
        RelationVerdict::Diverged(divergence) if divergence.first_component == component
    ));
    assert!(nonzero_words(&reduced) > 0);
    assert!(nonzero_words(&reduced) < nonzero_words(&instance));
}

#[test]
fn a_stored_counterexample_that_still_diverges_counts_as_a_finding_it_does_not_retain() {
    use crate::report::{FindingKind, FuzzReport};
    use crate::spu::sequence_relations::replay_counterexamples;
    let stored = stored_counterexamples().expect("the committed store parses");
    let report_of = || {
        let mut report = FuzzReport::new(
            crate::FuzzTarget::SpuSequence,
            1,
            crate::GenerationStrategy::Structured,
            crate::RetentionConfig::default(),
            4,
            32,
        );
        replay_counterexamples(&mut report, &stored).expect("the rows run");
        report
    };
    let clean = report_of();
    assert!(clean.is_clean());
    assert_eq!(clean.stored_replays.len(), stored.len());
    let _guard = seed(SeededDefect::SequencePartnerWrite);
    let seeded = report_of();
    assert_eq!(
        seeded
            .finding_counts
            .get(&FindingKind::MetamorphicViolation),
        Some(&1)
    );
    assert!(seeded.findings.is_empty());
}

#[test]
fn a_stored_fixture_stands_and_a_different_one_at_its_path_is_refused() {
    let relation = row(SpuSequenceRelationId::CeqNotEqualFused);
    let mut rng = Rng::for_case(CAMPAIGN_VERSION, 1450, 3);
    let instance = instantiate(relation, &mut rng).expect("instantiation draws");
    let fixture = RelationCounterexample::from_instance(
        "stands".to_owned(),
        relation,
        &instance,
        SpuObservationComponent::Registers,
    );
    let scratch = cellgov_testkit::scratch::scratch_labeled("counterexample_store");
    let path = counterexample_path(&scratch.to_string_lossy(), &fixture.name);
    fixture.store(&path).expect("the first write stores");
    fixture.store(&path).expect("the same fixture stands");
    let different = RelationCounterexample {
        first_component: SpuObservationComponent::LocalStore,
        ..fixture.clone()
    };
    assert!(matches!(
        different.store(&path),
        Err(CounterexampleStoreError::Collision { .. })
    ));
    let text = std::fs::read_to_string(&path).expect("the fixture reads");
    assert_eq!(
        RelationCounterexample::parse_json(&text).expect("parses"),
        fixture
    );
}
