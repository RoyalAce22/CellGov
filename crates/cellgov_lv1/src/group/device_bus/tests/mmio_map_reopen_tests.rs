//! `map_device_mmio_region` after a close and a new open of the device:
//! the close leaves the region counter, so the map takes its next value.

use super::super::close::close_device;
use super::super::open::open_device;
use super::*;
use crate::{GroupStates, StateChange};
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::call::{CLOSE_DEVICE, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::GAME_OS_MMIO_MAPPINGS;
use cellgov_time::GuestTicks;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

/// The region counter value that the kernel's first map takes in
/// CellGov's run of the 4.93 kernel.
const KERNEL_COUNTER: u64 = 7;

/// The address of the 64 KB region at the counter value `counter`: the
/// size exponent 16 in bits 42 to 47 and the counter shifted left by 16.
const fn region_address(counter: u64) -> u64 {
    (16 << 42) | (counter << 16)
}

/// The address of the 64 KB region at the counter value 8.
const NEXT_LPAR_ADDRESS: u64 = region_address(KERNEL_COUNTER + 1);

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

/// The kernel's map of device 1's region at bus address 0x3010000, with
/// the inputs of the console's record.
fn kernel_map() -> HypercallRequest {
    let row = GAME_OS_MMIO_MAPPINGS[0];
    call(
        MAP_DEVICE_MMIO_REGION,
        &[
            row.bus,
            row.device,
            row.bus_address,
            row.size,
            row.page_shift,
        ],
    )
}

/// Applies the changes of `disposition` to `groups`, then returns its r3,
/// its r4 and its changes.
fn apply(
    groups: &mut GroupStates,
    disposition: HypercallDisposition,
) -> (u64, u64, Vec<StateChange>) {
    let HypercallDisposition::Answered(answer) = disposition else {
        panic!("not answered: {disposition:?}");
    };
    let r4 = answer.outputs().map_or(u64::MAX, |out| out[0]);
    for change in &answer.changes {
        groups.apply(change.clone());
    }
    (answer.r3, r4, answer.changes)
}

fn open(groups: &mut GroupStates) -> (u64, u64, Vec<StateChange>) {
    let request = call(OPEN_DEVICE, &[DEVICE_1.bus, DEVICE_1.device, 0]);
    let disposition = open_device(&request, Some("4.93"), groups.device_bus());
    apply(groups, disposition)
}

fn map(groups: &mut GroupStates) -> (u64, u64, Vec<StateChange>) {
    let disposition = map_mmio(
        &kernel_map(),
        Some("4.93"),
        groups.device_bus(),
        groups.memory().regions(),
        &HeldRegions::of(groups),
    );
    apply(groups, disposition)
}

fn close(groups: &mut GroupStates) -> (u64, u64, Vec<StateChange>) {
    let request = call(CLOSE_DEVICE, &[DEVICE_1.bus, DEVICE_1.device]);
    let disposition = close_device(
        &request,
        Some("4.93"),
        groups.device_bus(),
        groups.memory().spaces(),
        &GuestMemory::new(0),
    );
    apply(groups, disposition)
}

/// The group states after the open and the first map of device 1, with
/// the next region counter value [`KERNEL_COUNTER`] before the map.
fn opened_and_mapped() -> GroupStates {
    let mut groups = GroupStates::default();
    groups.apply(MemoryChange::CounterTaken(KERNEL_COUNTER - 1).into());
    assert_eq!(open(&mut groups).0, 0);
    let (r3, r4, changes) = map(&mut groups);
    assert_eq!((r3, r4), (0, GAME_OS_MMIO_MAPPINGS[0].lpar_address));
    assert!(changes.contains(&MemoryChange::CounterTaken(KERNEL_COUNTER).into()));
    groups
}

#[test]
fn a_map_after_a_close_and_a_reopen_takes_the_counter_value_8_and_its_address() {
    let mut groups = opened_and_mapped();
    assert_eq!(
        GAME_OS_MMIO_MAPPINGS[0].lpar_address,
        region_address(KERNEL_COUNTER)
    );
    assert_eq!(close(&mut groups).0, 0);
    assert_eq!(groups.device_bus().mmio_mappings().count(), 0);
    assert_eq!(open(&mut groups).0, 0);

    let (r3, r4, changes) = map(&mut groups);
    assert_eq!((r3, r4), (0, NEXT_LPAR_ADDRESS));
    assert!(changes.contains(&MemoryChange::CounterTaken(8).into()));
    assert_eq!(groups.memory().regions().next_counter(), Some(9));
    let mapped: Vec<u64> = groups
        .device_bus()
        .mmio_mappings()
        .map(|mapping| mapping.lpar_address)
        .collect();
    assert_eq!(mapped, [NEXT_LPAR_ADDRESS]);
}

#[test]
fn the_close_answer_carries_only_the_close_and_leaves_the_counter_at_8() {
    let mut groups = opened_and_mapped();
    let memory = groups.memory().clone();
    assert_eq!(memory.regions().next_counter(), Some(8));
    let (r3, _, changes) = close(&mut groups);
    assert_eq!(r3, 0);
    assert_eq!(changes, [DeviceBusChange::Closed(DEVICE_1).into()]);
    assert_eq!(*groups.memory(), memory);
}
