use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn torn_down() -> NetworkState {
    let mut state = NetworkState::default();
    state.apply(NetworkChange::TornDown);
    state
}

/// The list holds every nonzero lane of a torn-down state: the mask 0
/// that the teardown stored [OFW-4.93 lv1 a:0x23E048-0x23E060] and the
/// state word 3 [OFW-4.93 lv1 a:0x23E0F8-0x23E108].
#[test]
fn torn_down_sync_term_lane_vector_golden() {
    let mask = |field, value| contribution(LaneIndex::new(234, 0, field, 0), value);
    let word = |field, value| contribution(LaneIndex::new(235, 0, field, 0), value);
    let sum = [mask(0, 1), word(0, 1), word(1, 3)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(torn_down().sync_term(), sum);
}

#[test]
fn the_state_word_adds_its_own_term_and_a_set_up_removes_it() {
    let mut mask_0 = NetworkState::default();
    mask_0.apply(NetworkChange::InterruptMaskSet(0));
    let word = lanes::value_term(TORN_DOWN_SOURCE, 0, &TORN_DOWN_STATE_WORD);
    assert_ne!(word, 0);
    assert_eq!(
        torn_down().sync_term(),
        mask_0.sync_term().wrapping_add(word)
    );
    let mut set_up = torn_down();
    set_up.apply(NetworkChange::SetUp);
    assert_eq!(set_up.sync_term(), mask_0.sync_term());
    let mut first_open = NetworkState::default();
    first_open.apply(NetworkChange::SetUp);
    assert_eq!(first_open.sync_term(), 0);
}

/// The stamp is pending until the scheme's tag moves, and then a version
/// after the one that first covered the mask.
#[test]
fn the_torn_down_surface_is_on_its_own_source_and_newer_than_the_mask_surface() {
    let stamp_of = |source: u8| {
        let surfaces: Vec<_> = GROUP
            .hashed_surfaces
            .iter()
            .filter(|surface| surface.source == Some(source))
            .collect();
        assert_eq!(surfaces.len(), 1, "source {source}");
        surfaces[0].stamp
    };
    let Stamp::Version(mask) = stamp_of(INTERRUPT_MASK_SOURCE) else {
        panic!("the mask surface has no version");
    };
    let stamp = stamp_of(TORN_DOWN_SOURCE);
    assert!(
        matches!(stamp, Stamp::Pending) || matches!(stamp, Stamp::Version(v) if v > mask),
        "{stamp:?} after the mask's version {mask}"
    );
    assert_eq!(TORN_DOWN_SOURCE, 235);
    assert_eq!(TORN_DOWN_STATE_WORD, 3);
    assert!(GROUP.lanes.iter().any(|lanes| lanes.contains(&235)));
}
