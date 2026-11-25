use super::*;
use crate::group::state::GroupStates;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::memory::region_address;
use cellgov_time::GuestTicks;

/// A 64 KB address holds 2^26 counter values.
const VALUES: u64 = 1 << 26;

/// The map of bus 1 device 1's region at 0x3010000 that the console
/// capture `lv1_device_mmio_map` holds: 64 KB with the page shift 12.
fn request() -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: MAP_DEVICE_MMIO_REGION,
        inputs: [1, 1, 0x301_0000, 0x1_0000, 12, 0, 0, 0],
        pc: 0x8000_0000_0007_D8CC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

fn address(counter: u64) -> u64 {
    region_address(0x1_0000, counter).expect("a 64 KB address")
}

/// lv1 skips an address that a region of the partition holds
/// [OFW-4.93 lv1 a:0x2CEDE4-0x2CEDEC].
#[test]
fn a_wrapped_address_that_a_held_mapping_takes_is_skipped_to_the_next_value() {
    let next = VALUES + 7;
    let mut groups = GroupStates::default();
    groups.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 1 }).into());
    groups.apply(MemoryChange::CounterTaken(next - 1).into());
    groups.apply(
        DeviceBusChange::MmioRegionMapped(MmioMapping {
            bus: 1,
            device: 2,
            bus_address: 0x302_0000,
            size: 0x1_0000,
            lpar_address: address(7),
            page_shift: 12,
        })
        .into(),
    );
    let call = request();
    let d = map_mmio(
        &call,
        Some("4.93"),
        groups.device_bus(),
        groups.memory().regions(),
        &HeldRegions::of(&groups),
    );
    let mut expected = HypercallAnswer::lv1_status(0, [address(8)]);
    expected.push_change(DeviceBusChange::MmioRegionMapped(MmioMapping {
        bus: 1,
        device: 1,
        bus_address: 0x301_0000,
        size: 0x1_0000,
        lpar_address: address(8),
        page_shift: 12,
    }));
    expected.push_change(MemoryChange::CounterTaken(next + 1));
    assert_eq!(d, HypercallDisposition::Answered(expected));
}
