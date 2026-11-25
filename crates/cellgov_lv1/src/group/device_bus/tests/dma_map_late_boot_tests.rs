use super::*;
use crate::DmaRegion;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::GAME_OS_DMA_REGIONS;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000007D6B4].
const KERNEL_SC: u64 = 0x8000_0000_0007_D6B4;

/// The flags that the kernel's wrapper builds from the flags word 0 that
/// each of its callers gives
/// [OFW-4.93 lv2_kernel a:0x800000000007d50c-0x800000000007d5bc]
/// [OFW-4.93 lv2_kernel a:0x80000000000954d0-0x80000000000954d4]
/// [OFW-4.93 lv2_kernel a:0x80000000000aa4c8-0x80000000000aa4cc].
const KERNEL_FLAGS: u64 = 0xE000_0000_0000_0000;

/// The two mappings of the capture `lv1_device_dma_mappings` that come
/// after the kernel's 18 pages of devices 1 and 2: device 1 in its region
/// at 0xC0800000, and device 2 in its region at 0xD0000000.
const LATE_MAPPINGS: [DmaMapping; 2] = [
    DmaMapping {
        bus: 1,
        device: 1,
        bus_address: 0xC080_0000,
        lpar_address: 0x9A_0000,
        length: 0x1_0000,
    },
    DmaMapping {
        bus: 1,
        device: 2,
        bus_address: 0xD000_0000,
        lpar_address: 0x80_0000,
        length: 0x10_0000,
    },
];

fn call(m: DmaMapping, flags: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..6].copy_from_slice(&[
        m.bus,
        m.device,
        m.lpar_address,
        m.bus_address,
        m.length,
        flags,
    ]);
    HypercallRequest {
        unit: UnitId::new(1),
        number: MAP_DEVICE_DMA_REGION,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(41_700),
    }
}

/// The state with devices 1 and 2 open and their captured regions held.
fn allocated() -> DeviceBusState {
    let mut state = DeviceBusState::default();
    for device in [1, 2] {
        state.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device }));
    }
    for row in GAME_OS_DMA_REGIONS
        .iter()
        .filter(|row| row.firmware == "4.93" && row.device != 0)
    {
        state.apply(DeviceBusChange::DmaRegionAllocated(DmaRegion {
            bus: row.bus,
            device: row.device,
            address: row.address,
            size: row.size,
            page_shift: row.page_shift,
        }));
    }
    state
}

fn map(request: &HypercallRequest) -> HypercallDisposition {
    map_region(request, Some("4.93"), &allocated(), &MemoryState::default())
}

fn mapped_answer(mapping: DmaMapping) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::DmaRegionMapped(mapping));
    HypercallDisposition::Answered(answer)
}

#[test]
fn each_late_captured_map_of_devices_1_and_2_with_the_kernel_flags_maps() {
    for m in LATE_MAPPINGS {
        assert_eq!(map(&call(m, KERNEL_FLAGS)), mapped_answer(m), "{m:X?}");
    }
}

#[test]
fn each_late_captured_map_with_other_allowed_flags_is_the_uncaptured_refusal() {
    let flags = [
        0,
        0x4000_0000_0000_0000,
        0x8000_0000_0000_0000,
        0xE000_0000_0000_0800,
        DMA_MAP_FLAGS_MASK,
    ];
    let mut checked = 0;
    for m in LATE_MAPPINGS {
        for flags in flags {
            assert_eq!(flags & !DMA_MAP_FLAGS_MASK, 0, "lv1's flags check passes");
            assert_ne!(flags, KERNEL_FLAGS);
            let request = call(m, flags);
            assert_eq!(
                map(&request),
                UNCAPTURED_MAP.refusal(&request),
                "{m:X?} {flags:#x}"
            );
            checked += 1;
        }
    }
    assert_eq!(checked, 10);
}
