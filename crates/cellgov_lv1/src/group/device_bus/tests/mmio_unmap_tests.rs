use super::super::mmio_map::map_mmio;
use super::super::open::open_device;
use super::*;
use crate::group::memory::allocator::HeldRegions;
use crate::group::memory::MemoryChange;
use crate::{page_table_storage, AddressSpace, AddressSpaceChange, GroupStates, StateChange};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{MAP_DEVICE_MMIO_REGION, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::GAME_OS_MMIO_MAPPINGS;
use cellgov_time::GuestTicks;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// The region counter value that the kernel's first map takes in
/// CellGov's run of the 4.93 kernel.
const KERNEL_COUNTER: u64 = 7;

/// The vas id of the 4.93 kernel's address space.
const VAS_ID: u64 = 2;

/// Doubleword 0 of a valid entry: the kernel's entry for the first page
/// of its MMIO mapping [OFW-4.93 lv2_kernel a:0x800000000005e4f0].
const VALID_D0: u64 = 0x1_0001;

/// The call `number` with `registers` as r3 onward, from the `sc` of the
/// 4.93 kernel's wrapper [OFW-4.93 lv2_kernel a:0x800000000007D8CC].
fn call(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: 0x8000_0000_0007_D8CC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(36_735),
    }
}

fn unmap_call(bus: u64, device: u64, address: u64) -> HypercallRequest {
    call(UNMAP_DEVICE_MMIO_REGION, &[bus, device, address])
}

/// The mapping of the console's record of the kernel's map.
fn kernel_mapping() -> MmioMapping {
    let row = GAME_OS_MMIO_MAPPINGS[0];
    MmioMapping {
        bus: row.bus,
        device: row.device,
        bus_address: row.bus_address,
        size: row.size,
        lpar_address: row.lpar_address,
        page_shift: row.page_shift,
    }
}

/// Applies the changes of `disposition` to `groups`, then returns its r3,
/// its outputs and its changes.
fn apply(
    groups: &mut GroupStates,
    disposition: HypercallDisposition,
) -> (u64, Option<[u64; 7]>, Vec<StateChange>) {
    let HypercallDisposition::Answered(answer) = disposition else {
        panic!("not answered: {disposition:?}");
    };
    for change in &answer.changes {
        groups.apply(change.clone());
    }
    (answer.r3, answer.outputs(), answer.changes)
}

fn map(groups: &mut GroupStates) -> (u64, Option<[u64; 7]>, Vec<StateChange>) {
    let row = GAME_OS_MMIO_MAPPINGS[0];
    let request = call(
        MAP_DEVICE_MMIO_REGION,
        &[
            row.bus,
            row.device,
            row.bus_address,
            row.size,
            row.page_shift,
        ],
    );
    let disposition = map_mmio(
        &request,
        Some("4.93"),
        groups.device_bus(),
        groups.memory().regions(),
        &HeldRegions::of(groups),
    );
    apply(groups, disposition)
}

/// The group states after the open of device 1 and the kernel's map,
/// with the next region counter value [`KERNEL_COUNTER`] before the map.
fn opened_and_mapped() -> GroupStates {
    let mut groups = GroupStates::default();
    groups.apply(MemoryChange::CounterTaken(KERNEL_COUNTER - 1).into());
    let open = call(OPEN_DEVICE, &[DEVICE_1.bus, DEVICE_1.device, 0]);
    let disposition = open_device(&open, Some("4.93"), groups.device_bus());
    assert_eq!(apply(&mut groups, disposition).0, 0);
    let (r3, outputs, _) = map(&mut groups);
    assert_eq!(r3, 0);
    assert_eq!(
        outputs.map(|out| out[0]),
        Some(kernel_mapping().lpar_address)
    );
    groups
}

/// The kernel's address space with a 256 KB page table in storage.
fn spaces() -> AddressSpaceState {
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0x1014_0000_0000_0000,
        mapping: None,
    }));
    spaces
}

/// The page table storage, with `entry` at index `index` of the table of
/// [`VAS_ID`] in `spaces`.
fn storage_with(spaces: &AddressSpaceState, index: u64, entry: [u64; 2]) -> GuestMemory {
    let mut storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let table = spaces.table_real(VAS_ID).expect("the table's storage");
    let mut bytes = entry[0].to_be_bytes().to_vec();
    bytes.extend_from_slice(&entry[1].to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(table + index * 16), 16).expect("range");
    storage.apply_commit(range, &bytes).expect("write");
    storage
}

fn empty_storage() -> GuestMemory {
    GuestMemory::from_regions(vec![page_table_storage()]).expect("storage")
}

fn unmap(groups: &GroupStates, request: &HypercallRequest) -> HypercallDisposition {
    unmap_mmio(
        request,
        Some("4.93"),
        groups.device_bus(),
        &spaces(),
        &empty_storage(),
    )
}

fn unmap_applied(
    groups: &mut GroupStates,
    request: &HypercallRequest,
) -> (u64, Option<[u64; 7]>, Vec<StateChange>) {
    let disposition = unmap(groups, request);
    apply(groups, disposition)
}

/// The real address of the kernel mapping's MMIO.
fn kernel_real() -> u64 {
    let mapping = kernel_mapping();
    let MmioRealAddress::Real(real) =
        mmio_real_address("4.93", mapping.bus, mapping.device, mapping.bus_address)
    else {
        panic!("the region's real address");
    };
    real
}

#[test]
fn an_unmap_at_the_kernel_mappings_start_answers_0_and_removes_the_mapping() {
    let mut groups = opened_and_mapped();
    let mapping = kernel_mapping();
    let request = unmap_call(mapping.bus, mapping.device, mapping.lpar_address);
    let (r3, outputs, changes) = unmap_applied(&mut groups, &request);
    assert_eq!(r3, 0);
    assert_eq!(outputs, Some([0; 7]));
    assert_eq!(
        changes,
        [DeviceBusChange::MmioRegionUnmapped(mapping).into()]
    );
    assert_eq!(groups.device_bus().mmio_mappings().count(), 0);
}

#[test]
fn a_map_after_an_unmap_takes_the_next_counter_value() {
    let mut groups = opened_and_mapped();
    let mapping = kernel_mapping();
    let request = unmap_call(mapping.bus, mapping.device, mapping.lpar_address);
    assert_eq!(unmap_applied(&mut groups, &request).0, 0);
    assert_eq!(
        groups.memory().regions().next_counter(),
        Some(KERNEL_COUNTER + 1)
    );
    let (r3, _, changes) = map(&mut groups);
    assert_eq!(r3, 0);
    assert!(changes.contains(&MemoryChange::CounterTaken(KERNEL_COUNTER + 1).into()));
}

#[test]
fn a_second_unmap_of_the_same_address_finds_no_mapping() {
    let mut groups = opened_and_mapped();
    let mapping = kernel_mapping();
    let request = unmap_call(mapping.bus, mapping.device, mapping.lpar_address);
    assert_eq!(unmap_applied(&mut groups, &request).0, 0);
    assert_eq!(
        unmap(&groups, &request),
        lv1_answer(MMIO_UNMAP_NO_MAPPING_R3)
    );
}

#[test]
fn each_failed_check_gets_lv1s_r3_and_changes_nothing() {
    let groups = opened_and_mapped();
    let address = kernel_mapping().lpar_address;
    let past_end = address + kernel_mapping().size;
    let cases = [
        (unmap_call(5, 1, address), MMIO_UNMAP_BUS_RANGE_R3),
        (unmap_call(0, 1, address), MMIO_UNMAP_NO_BUS_R3),
        (unmap_call(1, 2, address), MMIO_UNMAP_NOT_OPEN_R3),
        (unmap_call(1, 1, past_end), MMIO_UNMAP_NO_MAPPING_R3),
        (unmap_call(1, 1, 0), MMIO_UNMAP_NO_MAPPING_R3),
    ];
    assert_eq!(cases.len(), 5);
    for (request, r3) in cases {
        assert_eq!(unmap(&groups, &request), lv1_answer(r3), "{request:?}");
    }
}

#[test]
fn an_unmap_for_open_device_0_gets_the_device_0_r3() {
    let mut groups = GroupStates::default();
    groups.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 0 }).into());
    assert_eq!(
        unmap(&groups, &unmap_call(1, 0, 0x4000_0007_0000)),
        lv1_answer(MMIO_UNMAP_DEVICE_0_R3)
    );
}

/// lv1's access check runs before its device 0 check
/// [OFW-4.93 lv1 a:0x2371A8-0x2371B8].
#[test]
fn an_unmap_for_device_0_that_the_partition_did_not_open_gets_the_not_open_r3() {
    let groups = GroupStates::default();
    assert_eq!(
        unmap(&groups, &unmap_call(1, 0, 0x4000_0007_0000)),
        lv1_answer(MMIO_UNMAP_NOT_OPEN_R3)
    );
}

#[test]
fn an_unmap_on_device_2_of_device_1s_address_finds_no_mapping() {
    let mut groups = opened_and_mapped();
    groups.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 2 }).into());
    let request = unmap_call(1, 2, kernel_mapping().lpar_address);
    assert_eq!(
        unmap(&groups, &request),
        lv1_answer(MMIO_UNMAP_NO_MAPPING_R3)
    );
}

#[test]
fn an_unmap_inside_the_mapping_past_its_start_is_a_refusal() {
    let groups = opened_and_mapped();
    let request = unmap_call(1, 1, kernel_mapping().lpar_address + 0x1000);
    assert_eq!(unmap(&groups, &request), INSIDE_MAPPING.refusal(&request));
}

#[test]
fn unread_buses_devices_and_firmware_are_refusals() {
    let groups = opened_and_mapped();
    let address = kernel_mapping().lpar_address;
    for bus in [2, 3, 4] {
        let other_bus = unmap_call(bus, 0, address);
        assert_eq!(
            unmap(&groups, &other_bus),
            OTHER_BUS.refusal(&other_bus),
            "bus {bus}"
        );
    }
    let other_device = unmap_call(1, 3, address);
    assert_eq!(
        unmap(&groups, &other_device),
        OTHER_DEVICE.refusal(&other_device)
    );
    let request = unmap_call(1, 1, address);
    let at_4_92 = unmap_mmio(
        &request,
        Some("4.92"),
        groups.device_bus(),
        &spaces(),
        &empty_storage(),
    );
    assert_eq!(at_4_92, UNREAD_FIRMWARE.refusal(&request));
}

#[test]
fn a_valid_entry_that_maps_the_mappings_mmio_makes_the_unmap_a_refusal() {
    let groups = opened_and_mapped();
    let spaces = spaces();
    let last_page = kernel_real() + kernel_mapping().size - 0x1000;
    let storage = storage_with(&spaces, 0x89, [VALID_D0, last_page | 0x2C]);
    let request = unmap_call(1, 1, kernel_mapping().lpar_address);
    let disposition = unmap_mmio(
        &request,
        Some("4.93"),
        groups.device_bus(),
        &spaces,
        &storage,
    );
    assert_eq!(disposition, MAPPED_PAGE.refusal(&request));
}

#[test]
fn an_invalid_entry_or_a_page_past_the_mmio_leaves_the_unmap_answered() {
    let groups = opened_and_mapped();
    let spaces = spaces();
    let real = kernel_real();
    let entries = [
        [VALID_D0 & !1, real | 0x2C],
        [VALID_D0, (real + kernel_mapping().size) | 0x2C],
        [VALID_D0, (real - 0x1000) | 0x2C],
    ];
    assert_eq!(entries.len(), 3);
    let request = unmap_call(1, 1, kernel_mapping().lpar_address);
    for entry in entries {
        let storage = storage_with(&spaces, 0x89, entry);
        let disposition = unmap_mmio(
            &request,
            Some("4.93"),
            groups.device_bus(),
            &spaces,
            &storage,
        );
        let HypercallDisposition::Answered(answer) = disposition else {
            panic!("not answered for {entry:x?}: {disposition:?}");
        };
        assert_eq!(answer.r3, 0, "{entry:x?}");
    }
}
