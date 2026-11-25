use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };
const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

fn region(device: OpenDevice) -> DmaRegion {
    DmaRegion {
        bus: device.bus,
        device: device.device,
        address: 0xC000_0000,
        size: 0x4_0000,
        page_shift: 16,
    }
}

/// The state after the partition opened each of `devices`, allocated one
/// region of each and closed each.
fn freed(devices: &[OpenDevice]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &device in devices {
        state.apply(DeviceBusChange::Opened(device));
        state.apply(DeviceBusChange::DmaRegionAllocated(region(device)));
        state.apply(DeviceBusChange::Closed(device));
    }
    state
}

/// The list holds every nonzero lane of a state whose closes freed the
/// regions of devices 2 and 1, in that order.
#[test]
fn freed_pool_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(185, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 1),
        lane(0, 2, 1),
        lane(1, 0, 1),
        lane(1, 1, 1),
        lane(1, 2, 2),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    let state = freed(&[DEVICE_2, DEVICE_1]);
    assert_eq!(state.open_devices().count(), 0);
    assert_eq!(state.dma_regions().count(), 0);
    assert_eq!(
        state.freed_pools().collect::<Vec<_>>(),
        [DEVICE_1, DEVICE_2]
    );
    assert_eq!(state.sync_term(), sum);
}

#[test]
fn a_new_open_and_a_second_close_keep_the_record() {
    let mut state = freed(&[DEVICE_1]);
    let term = state.sync_term();
    state.apply(DeviceBusChange::Opened(DEVICE_1));
    assert!(state.pool_freed(DEVICE_1));
    state.apply(DeviceBusChange::Closed(DEVICE_1));
    assert_eq!(state.open_devices().count(), 0);
    assert_eq!(state.freed_pools().collect::<Vec<_>>(), [DEVICE_1]);
    assert_eq!(state, freed(&[DEVICE_1]));
    assert_eq!(state.sync_term(), term);
}

#[test]
fn the_freed_pool_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(FREED_POOL_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].stamp, Stamp::Version(36));
    assert_eq!(FREED_POOL_SOURCE, 185);
    assert!(GROUP
        .lanes
        .iter()
        .any(|range| range.contains(&FREED_POOL_SOURCE)));
}
