//! Tests one PPU case across sequence, internal-path, and independent-reference tiers.

use cellgov_fuzz::ppu_paths::first_path_divergence;
use cellgov_fuzz::ppu_reference::{
    compare_reference, parse_reference_json, replay_reference, PpuReferenceComponent,
    ReferenceField,
};
use cellgov_fuzz::ppu_sequences::{
    generate_dependency_sequence, replay_dependency_sequence, PpuSequenceFamily,
};

#[test]
fn generated_sequence_replays_identically_but_external_authority_detects_common_mode_error() {
    let generated = generate_dependency_sequence(7, 0);
    assert_eq!(generated.family, PpuSequenceFamily::RegisterAlias);
    let replay = replay_dependency_sequence(generated.clone()).expect("generated case must run");
    assert!(first_path_divergence(&replay.runs).is_none());

    let fixture = include_str!("fixtures/ppu_reference/li_r3_7_v1.json");
    let mut reference = parse_reference_json(fixture).expect("committed fixture must parse");
    reference.case_id = "ppc-book1-addi-r3-alias-twice".to_string();
    reference.words = generated.words;
    reference.initial_memory = generated.data;
    reference.initial_state.gpr.insert(3, 7);
    reference.initial_state.gpr.insert(4, 0x1000_0000);
    reference.initial_state.gpr.insert(5, 7 ^ 0x55);
    // [PPC-Book1 p:51 s:3.3.8] Each addi adds one to r3, so 7 becomes 9.
    let mut expected_gpr = vec![0; 32];
    expected_gpr[3] = 9;
    expected_gpr[4] = 0x1000_0000;
    expected_gpr[5] = 7 ^ 0x55;
    reference.expected.state.gpr = ReferenceField::Value {
        value: expected_gpr,
    };
    reference.expected.state.pc = ReferenceField::Value { value: 8 };
    reference.expected.memory = ReferenceField::Value { value: vec![0; 64] };
    reference.expected.stop.pc = ReferenceField::Value { value: Some(4) };
    reference.expected.retired = ReferenceField::Value { value: 2 };

    let external = replay_reference(&reference).expect("same generated case must replay offline");
    assert!(external.internal_divergence.is_none());
    assert!(external
        .comparisons
        .iter()
        .all(|comparison| comparison.is_match()));

    let mut common_mode = external.runs;
    for run in &mut common_mode {
        run.observation.state.gpr[3] = 10;
    }
    assert!(first_path_divergence(&common_mode).is_none());
    assert!(common_mode.iter().all(|run| {
        compare_reference(&reference.expected, run)
            .differences
            .iter()
            .any(|difference| difference.field == PpuReferenceComponent::StateGpr)
    }));
}
