use super::*;
use cellgov_mem::lanes::{contribution, LaneIndex};

const MAPPING: DmaMapping = DmaMapping {
    bus: 1,
    device: 0,
    bus_address: 0xC000_0000,
    lpar_address: 0x80_0000,
    length: 0x10_0000,
};

const REGION: DmaRegion = DmaRegion {
    bus: 1,
    device: 0,
    address: 0xC000_0000,
    size: 0x10_0000,
    page_shift: 20,
};

fn state(mappings: &[DmaMapping]) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for &mapping in mappings {
        state.apply(DeviceBusChange::DmaRegionMapped(mapping));
    }
    state
}

/// The list holds every nonzero lane of a state with one mapping.
#[test]
fn dma_mapping_sync_term_lane_vector_golden() {
    let lane = |field, value| contribution(LaneIndex::new(178, 0, field, 0), value);
    let sum = [
        lane(0, 1),
        lane(1, 1),
        lane(3, 0xC000_0000),
        lane(4, 0x80_0000),
        lane(5, 0x10_0000),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(state(&[MAPPING]).sync_term(), sum);
}

#[test]
fn dma_mapping_sync_term_keys_each_mapping_by_its_bus_device_and_address_order() {
    let later = DmaMapping {
        bus_address: 0xD000_0000,
        ..MAPPING
    };
    let lane = |object, field, value| contribution(LaneIndex::new(178, object, field, 0), value);
    let sum = [
        lane(0, 0, 1),
        lane(0, 1, 1),
        lane(0, 3, 0xC000_0000),
        lane(0, 4, 0x80_0000),
        lane(0, 5, 0x10_0000),
        lane(1, 0, 1),
        lane(1, 1, 1),
        lane(1, 3, 0xD000_0000),
        lane(1, 4, 0x80_0000),
        lane(1, 5, 0x10_0000),
    ]
    .into_iter()
    .fold(0u128, u128::wrapping_add);
    assert_eq!(state(&[later, MAPPING]).sync_term(), sum);
    let device_1 = DmaMapping {
        device: 1,
        bus_address: 0xB000_0000,
        ..MAPPING
    };
    let bus_2 = DmaMapping {
        bus: 2,
        bus_address: 0xA000_0000,
        ..MAPPING
    };
    let order: Vec<_> = state(&[bus_2, later, device_1, MAPPING])
        .dma_mappings()
        .collect();
    assert_eq!(order, [MAPPING, later, device_1, bus_2]);
}

#[test]
fn each_mapping_field_moves_the_term_and_adds_to_the_region_term() {
    let one = state(&[MAPPING]).sync_term();
    assert_ne!(one, 0);
    let changed = [
        DmaMapping { bus: 2, ..MAPPING },
        DmaMapping {
            device: 1,
            ..MAPPING
        },
        DmaMapping {
            bus_address: 0xC004_0000,
            ..MAPPING
        },
        DmaMapping {
            lpar_address: 0x90_0000,
            ..MAPPING
        },
        DmaMapping {
            length: 0x4_0000,
            ..MAPPING
        },
    ];
    assert_eq!(changed.len(), 5);
    for mapping in changed {
        assert_ne!(state(&[mapping]).sync_term(), one, "{mapping:?}");
    }
    let mut region = DeviceBusState::default();
    region.apply(DeviceBusChange::DmaRegionAllocated(REGION));
    let region_term = lanes::value_term(DMA_REGION_SOURCE, 0, &REGION);
    assert_eq!(region.sync_term(), region_term);
    let mut both = state(&[MAPPING]);
    both.apply(DeviceBusChange::DmaRegionAllocated(REGION));
    assert_eq!(both.sync_term(), one.wrapping_add(region_term));
}

#[test]
fn mappings_of_gives_only_the_mappings_of_one_device() {
    let other = DmaMapping {
        device: 1,
        ..MAPPING
    };
    let held = state(&[MAPPING, other]);
    let of_0: Vec<_> = held.mappings_of(OpenDevice { bus: 1, device: 0 }).collect();
    assert_eq!(of_0, [MAPPING]);
    assert_eq!(
        held.mappings_of(OpenDevice { bus: 2, device: 0 }).count(),
        0
    );
}

#[test]
fn the_dma_mapping_surface_is_on_its_own_source() {
    let surfaces: Vec<_> = GROUP
        .hashed_surfaces
        .iter()
        .filter(|surface| surface.source == Some(DMA_MAPPING_SOURCE))
        .collect();
    assert_eq!(surfaces.len(), 1);
    assert_eq!(DMA_MAPPING_SOURCE, 178);
    assert!(GROUP.lanes.iter().any(|lanes| lanes.contains(&178)));
}
