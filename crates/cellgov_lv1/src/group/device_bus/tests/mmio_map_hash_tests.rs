use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn mapping(bus_address: u64, lpar_address: u64) -> MmioMapping {
    MmioMapping {
        bus: 1,
        device: 1,
        bus_address,
        size: 0x1_0000,
        lpar_address,
        page_shift: 0,
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
/// out of bus address order.
#[test]
fn mmio_mapping_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(183, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 1),
        lane(0, 2, 1),
        lane(0, 3, 0x301_0000),
        lane(0, 4, 0x1_0000),
        lane(0, 5, 0x4000_0007_0000),
        lane(1, 0, 1),
        lane(1, 1, 1),
        lane(1, 2, 1),
        lane(1, 3, 0x381_0000),
        lane(1, 4, 0x1_0000),
        lane(1, 5, 0x4000_0008_0000),
        contribution(LaneIndex::new(184, 0, 0, 0), 1),
        contribution(LaneIndex::new(184, 1, 0, 0), 1),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let state = mapped(&[
        mapping(0x381_0000, 0x4000_0008_0000),
        mapping(0x301_0000, 0x4000_0007_0000),
    ]);
    assert_eq!(state.sync_term(), sum);
}

#[test]
fn each_mapping_field_moves_the_term_and_a_close_of_its_device_removes_it() {
    let base = mapped(&[mapping(0x301_0000, 0x4000_0007_0000)]);
    let term = base.sync_term();
    assert_ne!(term, DeviceBusState::default().sync_term());
    assert_ne!(
        mapped(&[mapping(0x381_0000, 0x4000_0007_0000)]).sync_term(),
        term
    );
    assert_ne!(
        mapped(&[mapping(0x301_0000, 0x4000_0008_0000)]).sync_term(),
        term
    );
    let mut other = mapping(0x301_0000, 0x4000_0007_0000);
    other.size = 0x8000;
    assert_ne!(mapped(&[other]).sync_term(), term);
    let mut other_device = mapping(0x301_0000, 0x4000_0007_0000);
    other_device.device = 2;
    assert_ne!(mapped(&[other_device]).sync_term(), term);
    let mut other_bus = mapping(0x301_0000, 0x4000_0007_0000);
    other_bus.bus = 2;
    assert_ne!(mapped(&[other_bus]).sync_term(), term);

    let mut closed = base.clone();
    closed.apply(DeviceBusChange::Closed(OpenDevice { bus: 1, device: 2 }));
    assert_eq!(closed.sync_term(), term);
    closed.apply(DeviceBusChange::Closed(OpenDevice { bus: 1, device: 1 }));
    assert_eq!(closed.sync_term(), DeviceBusState::default().sync_term());
}

#[test]
fn the_mmio_mapping_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(MMIO_MAPPING_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].stamp, Stamp::Version(34));
    assert_eq!(MMIO_MAPPING_SOURCE, 183);
}
