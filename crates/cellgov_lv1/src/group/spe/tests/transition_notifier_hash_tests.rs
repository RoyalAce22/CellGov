use super::*;
use crate::group::spe::{LogicalSpe, SpeState};
use cellgov_mem::lanes::{contribution, LaneIndex};

fn notifier(spe: u64) -> TransitionNotifier {
    TransitionNotifier {
        spe,
        port: 16,
        mask: 1,
    }
}

fn notifiers(list: &[TransitionNotifier]) -> TransitionNotifierState {
    let mut state = TransitionNotifierState::default();
    for &notifier in list {
        state.set(notifier);
    }
    state
}

#[test]
fn a_state_with_no_notifier_adds_no_term() {
    assert_eq!(TransitionNotifierState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with `notifier(11)`.
#[test]
fn transition_notifier_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(193, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 11), lane(2, 16), lane(3, 1)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(notifiers(&[notifier(11)]).sync_term(), sum);
}

#[test]
fn each_notifier_and_each_field_moves_the_term() {
    let one = notifiers(&[notifier(11)]).sync_term();
    assert_ne!(one, 0);
    assert_ne!(notifiers(&[notifier(11), notifier(12)]).sync_term(), one);
    let changed = [
        TransitionNotifier {
            spe: 12,
            ..notifier(11)
        },
        TransitionNotifier {
            port: 17,
            ..notifier(11)
        },
        TransitionNotifier {
            mask: 2,
            ..notifier(11)
        },
    ];
    assert_eq!(changed.len(), 3);
    for other in changed {
        assert_ne!(notifiers(&[other]).sync_term(), one, "{other:?}");
    }
}

#[test]
fn a_new_notifier_replaces_the_notifier_of_its_spe() {
    let replaced = notifiers(&[
        notifier(11),
        TransitionNotifier {
            mask: 4,
            ..notifier(11)
        },
    ]);
    assert_eq!(replaced.notifiers().len(), 1);
    assert_eq!(replaced.notifiers()[&11].mask, 4);
}

#[test]
fn a_removed_notifier_leaves_the_state_and_term_of_no_notifier() {
    let mut state = notifiers(&[notifier(11), notifier(12)]);
    state.clear(11);
    assert_eq!(state, notifiers(&[notifier(12)]));
    state.clear(12);
    assert_eq!(state, TransitionNotifierState::default());
    assert_eq!(state.sync_term(), 0);
}

#[test]
fn the_group_state_term_adds_the_notifiers_to_the_spes() {
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
    state.apply(SpeChange::NotifierSet(notifier(11)));
    assert_eq!(state.notifiers(), &notifiers(&[notifier(11)]));
    assert_eq!(
        state.sync_term(),
        spes_only.wrapping_add(notifiers(&[notifier(11)]).sync_term())
    );
    state.apply(SpeChange::NotifierCleared(11));
    assert_eq!(state.notifiers(), &TransitionNotifierState::default());
    assert_eq!(state.sync_term(), spes_only);
}
