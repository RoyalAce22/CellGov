use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn mapping(bus_address: u64, page_shift: u64) -> MmioMapping {
    MmioMapping {
        bus: 1,
        device: 1,
        bus_address,
        size: 0x1_0000,
        lpar_address: 0x4000_0007_0000,
        page_shift,
    }
}

fn mapped(mappings: &[MmioMapping]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &m in mappings {
        state.apply(DeviceBusChange::MmioRegionMapped(m));
    }
    state
}

/// The list holds every nonzero lane of a state with two mappings, given
/// out of bus address order: the mapping lanes on source 183 and the page
/// shift lanes on source 184, both keyed by the mapping's index.
#[test]
fn mmio_page_shift_sync_term_lane_vector_golden() {
    let lane = |source, object, field, value| {
        contribution(LaneIndex::new(source, object, field, 0), value)
    };
    let sum = [
        lane(183, 0, 0, 1),
        lane(183, 0, 1, 1),
        lane(183, 0, 2, 1),
        lane(183, 0, 3, 0x301_0000),
        lane(183, 0, 4, 0x1_0000),
        lane(183, 0, 5, 0x4000_0007_0000),
        lane(183, 1, 0, 1),
        lane(183, 1, 1, 1),
        lane(183, 1, 2, 1),
        lane(183, 1, 3, 0x381_0000),
        lane(183, 1, 4, 0x1_0000),
        lane(183, 1, 5, 0x4000_0007_0000),
        lane(184, 0, 0, 1),
        lane(184, 0, 1, 12),
        lane(184, 1, 0, 1),
        lane(184, 1, 1, 16),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let state = mapped(&[mapping(0x381_0000, 16), mapping(0x301_0000, 12)]);
    assert_eq!(state.sync_term(), sum);
}

#[test]
fn the_page_shift_moves_the_term() {
    let term = mapped(&[mapping(0x301_0000, 12)]).sync_term();
    assert_ne!(mapped(&[mapping(0x301_0000, 16)]).sync_term(), term);
    assert_ne!(mapped(&[mapping(0x301_0000, 0)]).sync_term(), term);
}

#[test]
fn the_page_shift_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(MMIO_PAGE_SHIFT_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].stamp, Stamp::Version(35));
    assert_eq!(MMIO_PAGE_SHIFT_SOURCE, 184);
    assert!(GROUP
        .lanes
        .iter()
        .any(|range| range.contains(&MMIO_PAGE_SHIFT_SOURCE)));
}
