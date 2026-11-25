use super::super::mmio_map::map_mmio;
use super::super::open::open_device;
use super::*;
use crate::group::device_bus::MmioMapping;
use crate::group::memory::allocator::HeldRegions;
use crate::group::memory::MemoryChange;
use crate::{page_table_storage, AddressSpace, AddressSpaceChange, GroupStates};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::{MAP_DEVICE_MMIO_REGION, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::{mmio_real_address, MmioRealAddress, GAME_OS_MMIO_MAPPINGS};
use cellgov_time::GuestTicks;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };
const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

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

/// Applies the changes of the answer `disposition` to `groups` and
/// returns its r3.
fn apply(groups: &mut GroupStates, disposition: HypercallDisposition) -> u64 {
    let HypercallDisposition::Answered(answer) = disposition else {
        panic!("not answered: {disposition:?}");
    };
    for change in &answer.changes {
        groups.apply(change.clone());
    }
    answer.r3
}

fn open(groups: &mut GroupStates, device: OpenDevice) {
    let request = call(OPEN_DEVICE, &[device.bus, device.device, 0]);
    let disposition = open_device(&request, Some("4.93"), groups.device_bus());
    assert_eq!(apply(groups, disposition), 0);
}

/// The group states after the open of device 1 and the kernel's map of
/// its MMIO, with the next region counter value [`KERNEL_COUNTER`].
fn opened_and_mapped() -> GroupStates {
    let mut groups = GroupStates::default();
    groups.apply(MemoryChange::CounterTaken(KERNEL_COUNTER - 1).into());
    open(&mut groups, DEVICE_1);
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
        &HeldRegions::of(&groups),
    );
    assert_eq!(apply(&mut groups, disposition), 0);
    assert_eq!(groups.device_bus().mmio_mappings().count(), 1);
    groups
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

/// The page table storage, with `entry` at index 0x89 of the table of
/// [`VAS_ID`] in `spaces`.
fn storage_with(spaces: &AddressSpaceState, entry: [u64; 2]) -> GuestMemory {
    let mut storage = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let table = spaces.table_real(VAS_ID).expect("the table's storage");
    let mut bytes = entry[0].to_be_bytes().to_vec();
    bytes.extend_from_slice(&entry[1].to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(table + 0x89 * 16), 16).expect("range");
    storage.apply_commit(range, &bytes).expect("write");
    storage
}

fn close_with(groups: &GroupStates, device: OpenDevice, entry: [u64; 2]) -> HypercallDisposition {
    let spaces = spaces();
    let storage = storage_with(&spaces, entry);
    let request = call(CLOSE_DEVICE, &[device.bus, device.device]);
    close_device(
        &request,
        Some("4.93"),
        groups.device_bus(),
        &spaces,
        &storage,
    )
}

#[test]
fn a_valid_entry_over_the_mmio_of_a_mapping_of_the_device_makes_the_close_a_refusal() {
    let groups = opened_and_mapped();
    let last_page = kernel_real() + kernel_mapping().size - 0x1000;
    let disposition = close_with(&groups, DEVICE_1, [VALID_D0, last_page | 0x2C]);
    let request = call(CLOSE_DEVICE, &[DEVICE_1.bus, DEVICE_1.device]);
    assert_eq!(disposition, MAPPED_PAGE.refusal(&request));
}

#[test]
fn an_invalid_entry_or_a_page_past_the_mmio_leaves_the_close_answered() {
    let groups = opened_and_mapped();
    let real = kernel_real();
    let entries = [
        [VALID_D0 & !1, real | 0x2C],
        [VALID_D0, (real + kernel_mapping().size) | 0x2C],
        [VALID_D0, (real - 0x1000) | 0x2C],
    ];
    assert_eq!(entries.len(), 3);
    for entry in entries {
        let HypercallDisposition::Answered(answer) = close_with(&groups, DEVICE_1, entry) else {
            panic!("not answered for {entry:x?}");
        };
        let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
        expected.push_change(DeviceBusChange::Closed(DEVICE_1));
        assert_eq!(answer, expected, "{entry:x?}");
    }
}

#[test]
fn the_registered_handler_reads_the_page_table_of_the_group_states_and_the_storage() {
    let mut groups = opened_and_mapped();
    groups.apply(
        AddressSpaceChange::Constructed(AddressSpace {
            vas_id: VAS_ID,
            htab_size_exp: 18,
            page_size_count: 2,
            page_shifts: 0x1014_0000_0000_0000,
            mapping: None,
        })
        .into(),
    );
    let storage = storage_with(groups.memory().spaces(), [VALID_D0, kernel_real() | 0x2C]);
    let request = call(CLOSE_DEVICE, &[DEVICE_1.bus, DEVICE_1.device]);
    let disposition = crate::handlers::answer(
        &request,
        crate::CallerTranslation::Loads,
        Some("4.93"),
        &groups,
        &storage,
    );
    assert_eq!(disposition, MAPPED_PAGE.refusal(&request));
}

#[test]
fn an_entry_over_another_devices_mmio_leaves_the_close_answered() {
    let mut groups = opened_and_mapped();
    open(&mut groups, DEVICE_2);
    let disposition = close_with(&groups, DEVICE_2, [VALID_D0, kernel_real() | 0x2C]);
    let HypercallDisposition::Answered(answer) = disposition else {
        panic!("not answered: {disposition:?}");
    };
    let mut expected = HypercallAnswer::lv1_status(0, [0; 7]);
    expected.push_change(DeviceBusChange::Closed(DEVICE_2));
    assert_eq!(answer, expected);
}
