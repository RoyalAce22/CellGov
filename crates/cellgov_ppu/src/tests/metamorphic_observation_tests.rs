use super::*;

fn observation() -> PpuObservation {
    let state = PpuState::new();
    finish_observation(PpuObservationInput {
        initial_state: &state,
        final_state: &state,
        memory_base: 0,
        initial_memory: &[0; 16],
        outcome: PpuObservedOutcome::Execution(ExecuteVerdict::Continue),
        effects: Vec::new(),
        stores: StoreBuffer::new(),
        unit: UnitId::new(0),
    })
    .unwrap()
}

#[test]
fn condition_relations_allow_only_their_named_cr_field() {
    for (delta, shift) in [
        (PpuPermittedDelta::Cr0, 28),
        (PpuPermittedDelta::Cr1, 24),
        (PpuPermittedDelta::Cr6, 4),
    ] {
        let first = observation();
        let mut permitted = first.clone();
        permitted.state.cr ^= 0x0f << shift;
        assert!(first
            .compare_metamorphic(&permitted, delta)
            .disallowed_differences
            .is_empty());

        let mut leak = permitted;
        leak.state.cr ^= 0x0f << ((shift + 4) % 32);
        assert_eq!(
            first
                .compare_metamorphic(&leak, delta)
                .disallowed_differences,
            [PpuObservationComponent::State].into_iter().collect()
        );
    }
}

#[test]
fn overflow_relation_allows_xer_overflow_bits_but_rejects_a_seeded_register_leak() {
    let first = observation();
    let mut permitted = first.clone();
    permitted.state.xer ^= (1 << 30) | (1 << 31);
    assert!(first
        .compare_metamorphic(&permitted, PpuPermittedDelta::XerOverflow)
        .disallowed_differences
        .is_empty());

    permitted.state.gpr[0] = 1;
    assert_eq!(
        first
            .compare_metamorphic(&permitted, PpuPermittedDelta::XerOverflow)
            .disallowed_differences,
        [PpuObservationComponent::State].into_iter().collect()
    );
}
