use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

fn held(opens: &[(u64, u64)]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &(device, r5) in opens {
        state.apply(DeviceBusChange::StorageDeviceOpened(StorageDeviceOpen {
            device,
            r5,
        }));
    }
    state
}

/// The list holds every nonzero lane of a state with storage devices 3
/// and 4 open.
#[test]
fn storage_device_open_sync_term_lane_vector_golden() {
    let lane = |object, field, value| contribution(LaneIndex::new(181, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 3),
        lane(0, 2, 0x8000_0000_0000_0000),
        lane(1, 0, 1),
        lane(1, 1, 4),
        lane(1, 2, 1),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(held(&[(4, 1), (3, 0x8000_0000_0000_0000)]).sync_term(), sum);
}

#[test]
fn the_device_and_the_r5_each_move_the_term() {
    let base = held(&[(1, 0)]).sync_term();
    assert_ne!(base, DeviceBusState::default().sync_term());
    assert_ne!(held(&[(2, 0)]).sync_term(), base);
    assert_ne!(held(&[(1, 1)]).sync_term(), base);
}

#[test]
fn the_storage_device_open_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(STORAGE_DEVICE_OPEN_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(surfaces[0].stamp, Stamp::Version(34));
    assert_eq!(STORAGE_DEVICE_OPEN_SOURCE, 181);
}
