use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

const REGION: DmaRegion = DmaRegion {
    bus: 1,
    device: 0,
    address: 0xC000_0000,
    size: 0x10_0000,
    page_shift: 20,
};

fn state(regions: &[DmaRegion]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &region in regions {
        state.apply(DeviceBusChange::DmaRegionAllocated(region));
    }
    state
}

/// The list holds every nonzero lane of a state with one region.
#[test]
fn dma_region_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(177, 0, field, 0), value);
    let sum = [
        lane(0, 1),
        lane(1, 1),
        lane(3, 0xC000_0000),
        lane(4, 0x10_0000),
        lane(5, 20),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(state(&[REGION]).sync_term(), sum);
}

#[test]
fn dma_region_sync_term_keys_each_region_by_its_bus_device_and_address_order() {
    let later = DmaRegion {
        address: 0xD000_0000,
        ..REGION
    };
    let lane = |object, field, value| contribution(LaneIndex::new(177, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 1),
        lane(0, 3, 0xC000_0000),
        lane(0, 4, 0x10_0000),
        lane(0, 5, 20),
        lane(1, 0, 1),
        lane(1, 1, 1),
        lane(1, 3, 0xD000_0000),
        lane(1, 4, 0x10_0000),
        lane(1, 5, 20),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(state(&[later, REGION]).sync_term(), sum);
    let order: Vec<_> = state(&[later, REGION]).dma_regions().collect();
    assert_eq!(order, [REGION, later]);
    let next_device = DmaRegion {
        device: 1,
        address: 0xA000_0000,
        ..REGION
    };
    let next_bus = DmaRegion {
        bus: 2,
        address: 0x9000_0000,
        ..REGION
    };
    let order: Vec<_> = state(&[next_bus, next_device, later, REGION])
        .dma_regions()
        .collect();
    assert_eq!(order, [REGION, later, next_device, next_bus]);
}

#[test]
fn each_region_field_moves_the_term_and_adds_to_the_open_device_term() {
    let one = state(&[REGION]).sync_term();
    assert_ne!(one, 0);
    let changed = [
        DmaRegion { bus: 2, ..REGION },
        DmaRegion {
            device: 1,
            ..REGION
        },
        DmaRegion {
            address: 0xA000_0000,
            ..REGION
        },
        DmaRegion {
            size: 0x20_0000,
            ..REGION
        },
        DmaRegion {
            page_shift: 16,
            ..REGION
        },
    ];
    assert_eq!(changed.len(), 5);
    for region in changed {
        assert_ne!(state(&[region]).sync_term(), one, "{region:?}");
    }
    let device = OpenDevice { bus: 1, device: 0 };
    let mut both = state(&[REGION]);
    both.apply(DeviceBusChange::Opened(device));
    let mut open = DeviceBusState::default();
    open.apply(DeviceBusChange::Opened(device));
    let open_term = lanes::value_term(OPEN_DEVICE_SOURCE, 0, &device);
    assert_eq!(open.sync_term(), open_term);
    assert_eq!(both.sync_term(), one.wrapping_add(open_term));
}

#[test]
fn regions_of_gives_only_the_regions_of_one_device() {
    let other = DmaRegion {
        device: 1,
        ..REGION
    };
    let held = state(&[REGION, other]);
    let of_0: Vec<_> = held.regions_of(OpenDevice { bus: 1, device: 0 }).collect();
    assert_eq!(of_0, [REGION]);
    assert_eq!(held.regions_of(OpenDevice { bus: 2, device: 0 }).count(), 0);
}

#[test]
fn the_dma_region_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(DMA_REGION_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert!(GROUP.lanes.iter().any(|lanes| lanes.contains(&177)));
}
