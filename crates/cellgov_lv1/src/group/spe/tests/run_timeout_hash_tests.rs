use super::*;
use crate::group::spe::{LogicalSpe, SpeState};
use cellgov_mem::lanes::{contribution, LaneIndex};

const LIMIT: u64 = 9_576_000_000;

fn timeout(spe: u64) -> RunTimeout {
    RunTimeout { spe, limit: LIMIT }
}

fn timeouts(list: &[RunTimeout]) -> RunTimeoutState {
    let mut state = RunTimeoutState::default();
    for &timeout in list {
        state.set(timeout);
    }
    state
}

#[test]
fn a_state_with_no_limit_adds_no_term() {
    assert_eq!(RunTimeoutState::default().sync_term(), 0);
}

/// The list holds every nonzero lane of a state with `timeout(11)`.
#[test]
fn run_timeout_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(194, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 11), lane(2, LIMIT)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(timeouts(&[timeout(11)]).sync_term(), sum);
}

#[test]
fn each_limit_and_each_field_moves_the_term() {
    let one = timeouts(&[timeout(11)]).sync_term();
    assert_ne!(one, 0);
    assert_ne!(timeouts(&[timeout(11), timeout(12)]).sync_term(), one);
    let changed = [
        RunTimeout {
            spe: 12,
            ..timeout(11)
        },
        RunTimeout {
            limit: LIMIT + 1,
            ..timeout(11)
        },
    ];
    assert_eq!(changed.len(), 2);
    for other in changed {
        assert_ne!(timeouts(&[other]).sync_term(), one, "{other:?}");
    }
}

/// Calls that set spe 12 and then spe 11 give spe 11 the object 0 and
/// spe 12 the object 1.
#[test]
fn run_timeout_sync_term_keys_each_limit_by_its_spe_id_order() {
    let lane = |object, field, value| contribution(LaneIndex::new(194, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 11),
        lane(0, 2, LIMIT),
        lane(1, 0, 1),
        lane(1, 1, 12),
        lane(1, 2, LIMIT + 1),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let later = RunTimeout {
        spe: 12,
        limit: LIMIT + 1,
    };
    assert_eq!(timeouts(&[later, timeout(11)]).sync_term(), sum);
}

#[test]
fn a_new_limit_replaces_only_the_limit_of_its_spe() {
    let replaced = timeouts(&[timeout(11), timeout(12), RunTimeout { spe: 11, limit: 5 }]);
    assert_eq!(replaced.timeouts().len(), 2);
    assert_eq!(replaced.timeouts()[&11].limit, 5);
    assert_eq!(replaced.timeouts()[&12], timeout(12));
}

/// The type 1 worker of `construct_logical_spe` sets +584 to 0
/// [OFW-4.93 lv1 a:0x315958].
#[test]
fn a_limit_of_0_leaves_the_state_and_term_of_no_limit() {
    let mut state = timeouts(&[timeout(11), timeout(12)]);
    state.set(RunTimeout { spe: 11, limit: 0 });
    assert_eq!(state, timeouts(&[timeout(12)]));
    state.set(RunTimeout { spe: 12, limit: 0 });
    assert_eq!(state, RunTimeoutState::default());
    assert_eq!(state.sync_term(), 0);
}

#[test]
fn the_group_state_term_adds_the_limits_to_the_spes() {
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
    state.apply(SpeChange::RunTimeoutSet(timeout(11)));
    assert_eq!(state.timeouts(), &timeouts(&[timeout(11)]));
    assert_eq!(
        state.sync_term(),
        spes_only.wrapping_add(timeouts(&[timeout(11)]).sync_term())
    );
    state.apply(SpeChange::RunTimeoutSet(RunTimeout { spe: 11, limit: 0 }));
    assert_eq!(state.timeouts(), &RunTimeoutState::default());
    assert_eq!(state.sync_term(), spes_only);
}
