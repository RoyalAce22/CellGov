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

/// The page shifts add their terms on their own source, keyed by the
/// mapping's index, beside the terms of the mappings with no page shift.
#[test]
fn mmio_page_shift_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(184, object, field, 0), value);
    let state = mapped(&[mapping(0x381_0000, 16), mapping(0x301_0000, 12)]);
    let without = mapped(&[mapping(0x381_0000, 0), mapping(0x301_0000, 0)]);
    let zero_shifts = [lane(0, 0, 1), lane(1, 0, 1)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    let sum = [lane(0, 0, 1), lane(0, 1, 12), lane(1, 0, 1), lane(1, 1, 16)]
        .into_iter()
        .fold(
            without.sync_term().wrapping_sub(zero_shifts),
            u128::wrapping_add,
        );
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
