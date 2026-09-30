//! The observation entry points that read the local store in place give
//! what their copying counterparts give.

use cellgov_event::UnitId;
use cellgov_spu::exec::execute;
use cellgov_spu::fuzz::generation_descriptors;
use cellgov_spu::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
use cellgov_spu::state::{SpuObservableSnapshot, SpuState};

fn initial_states() -> [SpuState; 2] {
    let blank = SpuState::new();
    let mut varied = SpuState::new();
    for k in 0..128 {
        varied.set_reg(k, [(k as u8).wrapping_mul(37); 16]);
    }
    for (index, byte) in varied.ls.iter_mut().enumerate().step_by(97) {
        *byte = index as u8;
    }
    varied.channels.in_mbox = vec![3];
    [blank, varied]
}

#[test]
fn every_descriptor_observes_the_same_through_both_entry_points() {
    let mut compared = 0usize;
    let mut local_store_violations = 0usize;
    for initial in initial_states() {
        let before = SpuObservableSnapshot::capture(&initial);
        for descriptor in generation_descriptors() {
            let instruction =
                cellgov_spu::decode::decode(descriptor.canonical_word).expect("canonical decodes");
            let footprint = SpuAllowedFootprint::for_instruction(&instruction);
            let mut after = initial.clone();
            let outcome = execute(&instruction, &mut after, UnitId::new(0));
            // The executed state, and one with a local-store byte the
            // instruction did not write.
            let mut tampered = after.clone();
            tampered.ls[0x3f00] ^= 0xff;
            for state in [&after, &tampered] {
                let observed = SpuObservation::capture(state, &outcome);
                let expected = footprint.violations(&initial, &observed);
                assert_eq!(
                    footprint.violations_of(&initial, &observed.state, &outcome),
                    expected,
                    "{:?}",
                    descriptor.kind
                );
                assert_eq!(
                    footprint.violations_after(&initial, state, &outcome),
                    expected,
                    "{:?}",
                    descriptor.kind
                );
                local_store_violations +=
                    usize::from(expected.contains(&SpuObservationComponent::LocalStore));

                let unchanged = SpuObservation::from_parts(before.clone(), outcome.clone());
                assert_eq!(
                    SpuObservation::compare_parts(&observed.state, &outcome, &before, &outcome),
                    observed.compare(&unchanged),
                    "{:?}",
                    descriptor.kind
                );
                assert_eq!(
                    observed.state.matches(&initial),
                    observed.state == before,
                    "{:?}",
                    descriptor.kind
                );
                compared += 1;
            }
        }
    }
    let expected = 2 * 2 * generation_descriptors().len();
    assert_eq!(compared, expected);
    assert!(
        local_store_violations > 0,
        "no case reached the local-store component, so its equivalence went unchecked"
    );
}
