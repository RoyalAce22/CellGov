use super::*;
use crate::group::spe::{LogicalSpe, SpeState};
use cellgov_mem::lanes::{contribution, LaneIndex};

fn mask(spe: u64, class: u64) -> InterruptMask {
    InterruptMask {
        spe,
        class,
        mask: 7,
    }
}

fn masks(list: &[InterruptMask]) -> InterruptMaskState {
    let mut state = InterruptMaskState::default();
    for &mask in list {
        state.set(mask);
    }
    state
}

#[test]
fn a_state_with_no_mask_adds_no_term() {
    assert_eq!(InterruptMaskState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with `mask(11, 2)`.
#[test]
fn interrupt_mask_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(195, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 11), lane(2, 2), lane(3, 7)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(masks(&[mask(11, 2)]).sync_term(), sum);
}

/// The state sets the mask of spe 12 first.
#[test]
fn interrupt_mask_sync_term_numbers_the_masks_in_spe_id_and_class_order() {
    let lane = |object, field, value| contribution(LaneIndex::new(195, object, field, 0), value);
    let lanes = [
        lane(0, 0, 1),
        lane(0, 1, 11),
        lane(0, 2, 2),
        lane(0, 3, 7),
        lane(1, 0, 1),
        lane(1, 1, 12),
        lane(1, 2, 1),
        lane(1, 3, 3),
    ];
    let sum = lanes.into_iter().fold(0u128, u128::wrapping_add);
    let later = InterruptMask {
        spe: 12,
        class: 1,
        mask: 3,
    };
    assert_eq!(masks(&[later, mask(11, 2)]).sync_term(), sum);
}

#[test]
fn each_mask_and_each_field_moves_the_term() {
    let one = masks(&[mask(11, 0)]).sync_term();
    assert_ne!(one, 0);
    assert_ne!(masks(&[mask(11, 0), mask(11, 1)]).sync_term(), one);
    let changed = [
        InterruptMask {
            spe: 12,
            ..mask(11, 0)
        },
        InterruptMask {
            class: 1,
            ..mask(11, 0)
        },
        InterruptMask {
            mask: 3,
            ..mask(11, 0)
        },
    ];
    assert_eq!(changed.len(), 3);
    for other in changed {
        assert_ne!(masks(&[other]).sync_term(), one, "{other:?}");
    }
}

#[test]
fn a_new_mask_replaces_only_the_mask_of_its_spe_and_class() {
    let replaced = masks(&[
        mask(11, 0),
        mask(11, 1),
        InterruptMask {
            mask: 1,
            ..mask(11, 0)
        },
    ]);
    assert_eq!(replaced.masks().len(), 2);
    assert_eq!(replaced.masks()[&(11, 0)].mask, 1);
    assert_eq!(replaced.masks()[&(11, 1)].mask, 7);
}

/// The base constructor of each interrupt object stores 0 at +112
/// [OFW-4.93 lv1 a:0x2FC3B8].
#[test]
fn a_mask_of_0_leaves_the_state_and_term_of_no_mask() {
    let mut state = masks(&[mask(11, 0), mask(11, 2)]);
    state.set(InterruptMask {
        mask: 0,
        ..mask(11, 0)
    });
    assert_eq!(state, masks(&[mask(11, 2)]));
    state.set(InterruptMask {
        mask: 0,
        ..mask(11, 2)
    });
    assert_eq!(state, InterruptMaskState::default());
    assert_eq!(state.sync_term(), 0);
}

#[test]
fn the_group_state_term_adds_the_masks_to_the_spes() {
    let spe = LogicalSpe {
        id: 11,
        spe_type: 1,
        vas_id: 2,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
        shadow_real: 0x1100_0000,
    };
    let mut state = SpeState::default();
    state.apply(SpeChange::Constructed(spe));
    let spes_only = state.sync_term();
    state.apply(SpeChange::InterruptMaskSet(mask(11, 1)));
    assert_eq!(state.masks(), &masks(&[mask(11, 1)]));
    assert_eq!(
        state.sync_term(),
        spes_only.wrapping_add(masks(&[mask(11, 1)]).sync_term())
    );
    state.apply(SpeChange::InterruptMaskSet(InterruptMask {
        mask: 0,
        ..mask(11, 1)
    }));
    assert_eq!(state.masks(), &InterruptMaskState::default());
    assert_eq!(state.sync_term(), spes_only);
}
