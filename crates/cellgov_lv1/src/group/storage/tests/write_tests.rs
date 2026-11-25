use super::*;
use crate::group::memory::region::MemoryRegion;
use crate::group::memory::storage::{
    allocated_storage, ALLOCATED_STORAGE_BASE, ALLOCATED_STORAGE_BYTES,
};
use crate::group::state::StateChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_mem::{PageSize, Region};
use cellgov_ps3_abi::lv1::call::{ALLOCATE_MEMORY, OPEN_DEVICE};
use cellgov_time::GuestTicks;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the kernel's notification write
/// [OFW-4.93 lv2_kernel a:0x80000000000AE250].
const KERNEL_SC: u64 = 0x8000_0000_000A_E250;

/// The address of the request's buffer, in the real mode region.
const BUFFER_ADDRESS: u64 = 0x1_0000;

/// The bytes of the caller's real address space in these tests.
const MEMORY_BYTES: usize = 0x2_0000;

/// The kernel's r3 to r8: device -1, region 0, start sector 0, one
/// sector, flags 0 and the buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE234-0x80000000000AE24C].
const KERNEL_INPUTS: [u64; 6] = [u64::MAX, 0, 0, 1, 0, BUFFER_ADDRESS];

/// The event mask that the kernel stores at +8 of its buffer
/// [OFW-4.93 lv2_kernel a:0x80000000000AE1F4-0x80000000000AE200].
const KERNEL_MASK: u64 = 2;

/// Memory whose buffer holds `command` at +0 and `mask` at +8.
fn memory(command: u64, mask: u64) -> GuestMemory {
    let mut memory = GuestMemory::new(MEMORY_BYTES);
    let head = ByteRange::new(GuestAddr::new(BUFFER_ADDRESS), 16).expect("the buffer head");
    let mut bytes = command.to_be_bytes().to_vec();
    bytes.extend_from_slice(&mask.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

fn kernel_memory() -> GuestMemory {
    memory(NOTIFY_WRITE_COMMAND, KERNEL_MASK)
}

fn call(number: u64, pc: u64, args: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_990),
    }
}

fn write(args: &[u64]) -> HypercallRequest {
    call(STORAGE_WRITE, KERNEL_SC, args)
}

/// A 4.93 hypervisor, after the kernel's open of the storage bus when
/// `open` is set.
fn hypervisor(open: bool) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    if open {
        let opened = hv.dispatch(&call(OPEN_DEVICE, KERNEL_OPEN_SC, &[4, u64::MAX, 0]));
        assert!(
            matches!(opened, HypercallDisposition::Answered(ref answer) if answer.r3 == 0),
            "{opened:?}"
        );
    }
    hv
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

/// lv1's `status` in r3, zero-extended as the handler returns it
/// [OFW-4.93 lv1 a:0x308B58], with r4 to r10 cleared.
fn status(status: i64) -> HypercallDisposition {
    HypercallDisposition::Answered(HypercallAnswer::lv1_status(zero_extended(status), []))
}

#[test]
fn a_notify_write_with_no_port_takes_tag_1_and_keeps_the_mask_and_the_completed_request() {
    let mut hv = hypervisor(true);
    let answer = answered(hv.dispatch_with(&write(&KERNEL_INPUTS), &kernel_memory()));
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
    let request = NotifyRequest {
        tag: 1,
        completion: Some(0),
    };
    assert_eq!(
        answer.changes,
        vec![
            StateChange::from(StorageChange::NotifyEventMaskSet(KERNEL_MASK)),
            StateChange::from(StorageChange::NotifyRequested(request)),
        ]
    );
    let storage = hv.groups().storage();
    assert_eq!(storage.notify_requests().collect::<Vec<_>>(), vec![request]);
    assert_eq!(storage.notify_event_mask(), Some(KERNEL_MASK));
}

#[test]
fn each_notify_write_takes_the_lowest_free_tag_and_one_with_no_tag_free_gets_busy_with_the_mask() {
    let mut hv = hypervisor(true);
    let memory = kernel_memory();
    for tag in 1..NOTIFY_REQUEST_TAGS {
        let answer = answered(hv.dispatch_with(&write(&KERNEL_INPUTS), &memory));
        assert_eq!(answer.r3, 0, "tag {tag}");
        assert_eq!(answer.outputs().map(|o| o[0]), Some(u64::from(tag)));
    }
    assert_eq!(hv.groups().storage().held_tags(), 0xFFFE);
    let full = answered(hv.dispatch_with(&write(&KERNEL_INPUTS), &memory));
    assert_eq!(full.r3, zero_extended(NOTIFY_REQUESTS_FULL_STATUS));
    assert_eq!(full.outputs(), Some([0; 7]));
    assert_eq!(
        full.changes,
        vec![StateChange::from(StorageChange::NotifyEventMaskSet(
            KERNEL_MASK
        ))]
    );
}

#[test]
fn a_device_above_6_other_than_minus_1_gets_no_entry() {
    let mut hv = hypervisor(true);
    for device in [7, u64::MAX - 1] {
        let mut inputs = KERNEL_INPUTS;
        inputs[0] = device;
        assert_eq!(
            hv.dispatch_with(&write(&inputs), &kernel_memory()),
            status(STORAGE_WRITE_NO_DEVICE_STATUS),
            "device {device:#x}"
        );
    }
}

#[test]
fn a_device_from_0_to_6_is_a_refusal() {
    let mut hv = hypervisor(true);
    for device in [0, STORAGE_DEVICE_ID_MAX] {
        let mut inputs = KERNEL_INPUTS;
        inputs[0] = device;
        let request = write(&inputs);
        assert_eq!(
            hv.dispatch_with(&request, &kernel_memory()),
            DEVICE_OBJECT.refusal(&request)
        );
    }
}

#[test]
fn a_partition_that_did_not_open_the_bus_gets_wrong_state() {
    let mut hv = hypervisor(false);
    assert_eq!(
        hv.dispatch_with(&write(&KERNEL_INPUTS), &kernel_memory()),
        status(STORAGE_WRITE_NOT_READY_STATUS)
    );
    assert_eq!(hv.groups().storage().notify_requests().count(), 0);
}

#[test]
fn a_start_sector_count_or_flags_that_lv1_does_not_accept_gets_illegal_parameter() {
    let mut hv = hypervisor(true);
    for (index, value) in [(2, 1), (3, 0), (3, 2), (4, 8)] {
        let mut inputs = KERNEL_INPUTS;
        inputs[index] = value;
        assert_eq!(
            hv.dispatch_with(&write(&inputs), &kernel_memory()),
            status(NOTIFY_WRITE_BAD_REQUEST_STATUS),
            "r{} = {value}",
            index + 3
        );
    }
}

#[test]
fn the_region_id_does_not_change_the_answer() {
    let mut inputs = KERNEL_INPUTS;
    inputs[1] = 5;
    let mut hv = hypervisor(true);
    let answer = answered(hv.dispatch_with(&write(&inputs), &kernel_memory()));
    assert_eq!(answer.outputs(), Some([1, 0, 0, 0, 0, 0, 0]));
}

#[test]
fn a_buffer_whose_command_is_not_0_gets_illegal_parameter() {
    let mut hv = hypervisor(true);
    assert_eq!(
        hv.dispatch_with(&write(&KERNEL_INPUTS), &memory(1, KERNEL_MASK)),
        status(NOTIFY_WRITE_BAD_REQUEST_STATUS)
    );
    assert_eq!(hv.groups().storage().notify_event_mask(), None);
}

#[test]
fn a_buffer_that_the_memory_does_not_hold_is_a_refusal() {
    let mut hv = hypervisor(true);
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = MEMORY_BYTES as u64 - NOTIFY_BUFFER_BYTES + 8;
    let request = write(&inputs);
    assert_eq!(
        hv.dispatch_with(&request, &kernel_memory()),
        BUFFER.refusal(&request)
    );
}

#[test]
fn a_write_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.92");
    let request = write(&KERNEL_INPUTS);
    assert_eq!(
        hv.dispatch_with(&request, &kernel_memory()),
        UNREAD_FIRMWARE.refusal(&request)
    );
}

/// The size and the page size exponent of the kernel's allocation at
/// [OFW-4.93 lv2_kernel a:0x80000000002653c0], which passes 20.
const REGION_SIZE: u64 = 0x10_0000;
const REGION_PAGE_SIZE_EXP: u64 = 20;

/// A hypervisor after the storage bus open and one allocation, the
/// region's logical partition address, and memory whose region storage
/// holds the kernel's buffer at offset `offset` of the region.
fn allocated(offset: u64) -> (Hypervisor, u64, GuestMemory) {
    let mut hv = hypervisor(true);
    let mut memory = GuestMemory::from_regions(vec![
        Region::new(0, MEMORY_BYTES, "flat", PageSize::Page64K),
        allocated_storage(),
    ])
    .expect("the regions do not overlap");
    let allocate = call(
        ALLOCATE_MEMORY,
        KERNEL_SC,
        &[REGION_SIZE, REGION_PAGE_SIZE_EXP, 0, 0],
    );
    let lpar = answered(hv.dispatch_with(&allocate, &memory))
        .outputs()
        .expect("outputs")[0];
    let memory_state = hv.groups().memory().regions();
    let real = memory_state
        .storage_of(&memory_state.regions()[0])
        .expect("storage");
    let head = ByteRange::new(GuestAddr::new(real + offset), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    (hv, lpar, memory)
}

#[test]
fn a_buffer_in_an_allocated_region_is_read_from_the_region_storage() {
    for offset in [0, 0x100] {
        let (mut hv, lpar, memory) = allocated(offset);
        let mut inputs = KERNEL_INPUTS;
        inputs[5] = lpar + offset;
        let answer = answered(hv.dispatch_with(&write(&inputs), &memory));
        assert_eq!(answer.r3, 0, "offset {offset:#x}");
        assert_eq!(
            answer.outputs().map(|o| o[0]),
            Some(1),
            "offset {offset:#x}"
        );
        assert_eq!(
            hv.groups().storage().notify_event_mask(),
            Some(KERNEL_MASK),
            "offset {offset:#x}"
        );
    }
}

/// The first region fills the storage, so the second gets none. A buffer
/// in the second region is unheld; it never takes the storage of the
/// first.
#[test]
fn a_buffer_in_an_allocated_region_with_no_storage_is_unheld() {
    let stored = MemoryRegion {
        lpar_address: 0x0000_5000_0030_0000,
        size: ALLOCATED_STORAGE_BYTES,
        page_size_exp: 16,
        unit_id: 1,
        counter: 2,
    };
    let unstored = MemoryRegion {
        lpar_address: 0x0000_6400_0200_0000,
        size: REGION_SIZE,
        page_size_exp: 20,
        unit_id: 1,
        counter: 3,
    };
    let mut regions = MemoryState::default();
    regions.create(stored);
    regions.create(unstored);
    assert_eq!(regions.storage_of(&stored), Some(ALLOCATED_STORAGE_BASE));
    assert_eq!(regions.storage_of(&unstored), None);
    let rm_size = real_mode_size("4.93");
    assert_eq!(
        buffer_real(
            &regions,
            rm_size,
            unstored.lpar_address + 0x100,
            NOTIFY_BUFFER_BYTES
        ),
        BufferPlace::Unheld
    );
    assert_eq!(
        buffer_real(
            &regions,
            rm_size,
            stored.lpar_address + 0x100,
            NOTIFY_BUFFER_BYTES
        ),
        BufferPlace::Real(ALLOCATED_STORAGE_BASE + 0x100)
    );
}

/// The first address past the allocated region lies in no region of the
/// partition and above the real mode region. The region storage just past
/// the region's range holds a buffer, so a lookup that takes the address
/// into the region gets an answer.
#[test]
fn a_buffer_that_starts_at_its_allocated_region_end_is_a_refusal() {
    let (mut hv, lpar, memory) = allocated(REGION_SIZE);
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = lpar + REGION_SIZE;
    let request = write(&inputs);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        BUFFER.refusal(&request)
    );
    assert_eq!(hv.groups().storage().notify_event_mask(), None);
}

#[test]
fn a_buffer_that_ends_past_its_allocated_region_gets_lv1_no_privilege() {
    let offset = REGION_SIZE - NOTIFY_BUFFER_BYTES + 8;
    let (mut hv, lpar, memory) = allocated(offset);
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = lpar + offset;
    let answer = answered(hv.dispatch_with(&write(&inputs), &memory));
    assert_eq!(
        answer.r3,
        zero_extended(NOTIFY_WRITE_BUFFER_PAST_REGION_STATUS)
    );
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!(hv.groups().storage().notify_event_mask(), None);
}

#[test]
fn a_buffer_that_ends_at_its_allocated_region_end_is_read() {
    let offset = REGION_SIZE - NOTIFY_BUFFER_BYTES;
    let (mut hv, lpar, memory) = allocated(offset);
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = lpar + offset;
    let answer = answered(hv.dispatch_with(&write(&inputs), &memory));
    assert_eq!(answer.r3, 0);
    assert_eq!(hv.groups().storage().notify_event_mask(), Some(KERNEL_MASK));
}

/// CellGov's storage of the allocated regions lies above the real mode
/// region, and no allocated region gives the partition its address.
#[test]
fn a_buffer_above_the_real_mode_region_in_no_allocated_region_is_a_refusal() {
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    assert!(ALLOCATED_STORAGE_BASE >= rm_size);
    let mut hv = hypervisor(true);
    let mut memory = GuestMemory::from_regions(vec![
        Region::new(0, MEMORY_BYTES, "flat", PageSize::Page64K),
        allocated_storage(),
    ])
    .expect("the regions do not overlap");
    let address = ALLOCATED_STORAGE_BASE;
    let head = ByteRange::new(GuestAddr::new(address), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = address;
    let request = write(&inputs);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        BUFFER.refusal(&request)
    );
    assert_eq!(hv.groups().storage().notify_event_mask(), None);
}

/// A memory whose one region holds the 0x1000 bytes on each side of the
/// end of the 4.93 real mode region, with the kernel's buffer head at
/// `address`.
fn memory_at_the_real_mode_end(address: u64) -> GuestMemory {
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    let mut memory = GuestMemory::from_regions(vec![Region::new(
        rm_size - 0x1000,
        0x2000,
        "real mode end",
        PageSize::Page64K,
    )])
    .expect("one region");
    let head = ByteRange::new(GuestAddr::new(address), 16).expect("the buffer head");
    let mut bytes = NOTIFY_WRITE_COMMAND.to_be_bytes().to_vec();
    bytes.extend_from_slice(&KERNEL_MASK.to_be_bytes());
    memory.apply_commit(head, &bytes).expect("the buffer write");
    memory
}

#[test]
fn a_real_mode_buffer_that_ends_past_the_real_mode_region_is_a_refusal() {
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    let address = rm_size - NOTIFY_BUFFER_BYTES + 8;
    let memory = memory_at_the_real_mode_end(address);
    let mut hv = hypervisor(true);
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = address;
    let request = write(&inputs);
    assert_eq!(
        hv.dispatch_with(&request, &memory),
        BUFFER.refusal(&request)
    );
    assert_eq!(hv.groups().storage().notify_event_mask(), None);
}

#[test]
fn a_real_mode_buffer_that_ends_at_the_real_mode_region_end_is_read() {
    let rm_size = real_mode_size("4.93").expect("the 4.93 real mode size");
    let address = rm_size - NOTIFY_BUFFER_BYTES;
    let memory = memory_at_the_real_mode_end(address);
    let mut hv = hypervisor(true);
    let mut inputs = KERNEL_INPUTS;
    inputs[5] = address;
    let answer = answered(hv.dispatch_with(&write(&inputs), &memory));
    assert_eq!(answer.r3, 0);
    assert_eq!(hv.groups().storage().notify_event_mask(), Some(KERNEL_MASK));
}
