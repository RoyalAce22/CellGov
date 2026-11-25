use super::*;
use crate::SpeState;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn spe(id: u64) -> LogicalSpe {
    LogicalSpe {
        id,
        spe_type: 1,
        vas_id: 2,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
    }
}

fn spes(list: &[LogicalSpe]) -> LogicalSpeState {
    let mut state = LogicalSpeState::default();
    for &spe in list {
        state.construct(spe);
    }
    state
}

#[test]
fn a_state_with_no_spe_adds_no_term() {
    assert_eq!(LogicalSpeState::default().sync_term(), 0);
    assert_eq!(SpeState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with the SPE `spe(11)`.
#[test]
fn logical_spe_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(LOGICAL_SPE_SOURCE, 0, field, 0), value);
    let sum = [
        lane(0, 1),
        lane(1, 11),
        lane(2, 1),
        lane(3, 2),
        lane(4, 0x0000_4c00_0028_0000),
        lane(5, 0x0000_3000_0000_6000),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(spes(&[spe(11)]).sync_term(), sum);
}

#[test]
fn each_spe_and_each_field_moves_the_term() {
    let one = spes(&[spe(11)]).sync_term();
    assert_ne!(one, 0);
    assert_ne!(spes(&[spe(11), spe(12)]).sync_term(), one);
    let changed = [
        LogicalSpe { id: 12, ..spe(11) },
        LogicalSpe {
            spe_type: 2,
            ..spe(11)
        },
        LogicalSpe {
            vas_id: 3,
            ..spe(11)
        },
        LogicalSpe {
            area: 0x0000_4c00_0030_0000,
            ..spe(11)
        },
        LogicalSpe {
            shadow: 0x0000_3000_0000_7000,
            ..spe(11)
        },
    ];
    assert_eq!(changed.len(), 5);
    for other in changed {
        assert_ne!(spes(&[other]).sync_term(), one, "{other:?}");
    }
}

#[test]
fn the_term_does_not_depend_on_the_construction_order() {
    let forward = spes(&[spe(11), spe(12)]);
    let backward = spes(&[spe(12), spe(11)]);
    assert_eq!(forward, backward);
    assert_eq!(forward.sync_term(), backward.sync_term());
}

#[test]
fn the_group_state_term_is_the_term_of_its_spes() {
    let mut state = SpeState::default();
    state.apply(SpeChange::Constructed(spe(11)));
    assert_eq!(state.spes(), &spes(&[spe(11)]));
    assert_eq!(state.sync_term(), spes(&[spe(11)]).sync_term());
}
