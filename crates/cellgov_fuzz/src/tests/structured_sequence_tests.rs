use super::*;

#[test]
fn an_spu_sequence_does_not_execute_beyond_its_generated_words() {
    let mut initial = SpuState::new();
    let branch = generation_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.kind == cellgov_spu::instruction::SpuInstructionKind::Br)
        .expect("branch generation descriptor must exist");
    let mut parameters = branch.canonical_parameters();
    parameters[0] = 2;
    let branch_to_third_word = branch
        .encode(&parameters)
        .expect("branch to third word must encode");
    initial.ls[..4].copy_from_slice(&branch_to_third_word.to_be_bytes());
    initial.ls[8..12].copy_from_slice(&branch.canonical_word.to_be_bytes());

    let (_, decoded, _) = run_generated_sequence(&initial, 2, GenerationStrategy::Structured);

    assert_eq!(decoded, 1);
}
