//! `allocate_device_dma_region` after a close and a new open of the
//! device, through the hypervisor's own handlers.

use super::*;
use crate::answer::HypercallRefusal;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CLOSE_DEVICE, OPEN_DEVICE};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's wrapper
/// [OFW-4.93 lv2_kernel a:0x800000000009585C].
const KERNEL_SC: u64 = 0x8000_0000_0009_585C;

const DEVICE_1: OpenDevice = OpenDevice { bus: 1, device: 1 };

const DEVICE_2: OpenDevice = OpenDevice { bus: 1, device: 2 };

/// The flags of the kernel's device 1 and device 2 calls.
const SLOT_FLAGS: u64 = 1;

/// The call `number` with `registers` as r3 onward.
fn call(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::new(38_419),
    }
}

/// The device's captured regions at 4.93, in the order of the calls.
fn captured(device: OpenDevice) -> Vec<DmaRegion> {
    game_os_dma_regions("4.93", device.bus, device.device)
        .map(|row| DmaRegion {
            bus: device.bus,
            device: device.device,
            address: row.address,
            size: row.size,
            page_shift: row.page_shift,
        })
        .collect()
}

fn allocate(device: OpenDevice, region: DmaRegion) -> HypercallRequest {
    let flags = if device == NETWORK_DEVICE {
        0
    } else {
        SLOT_FLAGS
    };
    call(
        ALLOCATE_DEVICE_DMA_REGION,
        &[
            device.bus,
            device.device,
            region.size,
            region.page_shift,
            flags,
        ],
    )
}

/// The r3 of an answered `disposition`.
fn r3_of(disposition: &HypercallDisposition) -> u64 {
    match disposition {
        HypercallDisposition::Answered(answer) => answer.r3,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// Sends `request` and asserts that the answer's r3 is 0.
fn dispatch_ok(hv: &mut Hypervisor, request: &HypercallRequest) {
    assert_eq!(r3_of(&hv.dispatch(request)), 0, "{request:?}");
}

/// The bus address that an answered `disposition` gives in r4.
fn address_of(disposition: &HypercallDisposition) -> Option<u64> {
    match disposition {
        HypercallDisposition::Answered(answer) => answer.outputs().map(|out| out[0]),
        HypercallDisposition::Refused(_) => None,
    }
}

fn freed_pool_refusal(request: &HypercallRequest) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: ALLOCATE_DEVICE_DMA_REGION,
        unit: request.unit,
        pc: request.pc,
        what: FREED_POOL.what,
    })
}

fn open(hv: &mut Hypervisor, device: OpenDevice) {
    dispatch_ok(hv, &call(OPEN_DEVICE, &[device.bus, device.device, 0]));
}

fn close(hv: &mut Hypervisor, device: OpenDevice) {
    dispatch_ok(hv, &call(CLOSE_DEVICE, &[device.bus, device.device]));
}

fn hypervisor() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    hv
}

/// Opens `device`, allocates its first `count` captured regions, closes
/// it, opens it again and returns the hypervisor.
fn reopened_after_allocating(device: OpenDevice, count: usize) -> Hypervisor {
    let mut hv = hypervisor();
    open(&mut hv, device);
    let rows = captured(device);
    assert!(rows.len() >= count, "{device:?} has {} rows", rows.len());
    for &row in &rows[..count] {
        assert_eq!(
            address_of(&hv.dispatch(&allocate(device, row))),
            Some(row.address)
        );
    }
    close(&mut hv, device);
    let not_open = allocate(device, rows[0]);
    assert_eq!(r3_of(&hv.dispatch(&not_open)), DMA_REGION_NOT_OPEN_R3);
    open(&mut hv, device);
    hv
}

#[test]
fn a_reopened_device_1_gets_the_freed_pool_refusal_for_each_captured_shape() {
    let mut hv = reopened_after_allocating(DEVICE_1, 2);
    assert_eq!(hv.groups().device_bus().dma_regions().count(), 0);
    assert!(hv.groups().device_bus().pool_freed(DEVICE_1));
    let rows = captured(DEVICE_1);
    assert_eq!(rows.len(), 5);
    for row in rows {
        let request = allocate(DEVICE_1, row);
        assert_eq!(hv.dispatch(&request), freed_pool_refusal(&request));
    }
    assert_eq!(hv.groups().device_bus().dma_regions().count(), 0);
}

#[test]
fn a_reopened_network_device_gets_the_freed_pool_refusal() {
    let mut hv = reopened_after_allocating(NETWORK_DEVICE, 1);
    let rows = captured(NETWORK_DEVICE);
    assert_eq!(rows.len(), 1);
    let request = allocate(NETWORK_DEVICE, rows[0]);
    assert_eq!(hv.dispatch(&request), freed_pool_refusal(&request));
}

/// The pool function takes each device's pool by the device's index
/// [OFW-4.93 lv1 a:0x22D3B0-0x22D3F0]. The free and the search use the
/// allocator that the pool's word at +160 selects. lv1's init gives that
/// word 8 for device 1 and 9 for device 2
/// [OFW-4.93 lv1 a:0x3A9A4C-0x3A9A5C] [OFW-4.93 lv1 a:0x3A9AA0-0x3A9AB0].
/// So a close that freed device 1's regions leaves device 2's answers.
#[test]
fn a_close_that_freed_device_1_regions_leaves_device_2_its_first_captured_region() {
    let mut hv = reopened_after_allocating(DEVICE_1, 1);
    open(&mut hv, DEVICE_2);
    let first = captured(DEVICE_2)[0];
    assert_eq!(
        address_of(&hv.dispatch(&allocate(DEVICE_2, first))),
        Some(first.address)
    );
    assert_eq!(
        hv.groups().device_bus().freed_pools().collect::<Vec<_>>(),
        [DEVICE_1]
    );
}

/// The close calls the free method only for each region of the caller
/// [OFW-4.93 lv1 a:0x2295E8-0x22960C], so a close with no region held
/// frees nothing.
#[test]
fn a_close_with_no_region_held_leaves_the_first_captured_region() {
    let mut hv = reopened_after_allocating(DEVICE_1, 0);
    assert!(!hv.groups().device_bus().pool_freed(DEVICE_1));
    let first = captured(DEVICE_1)[0];
    assert_eq!(
        address_of(&hv.dispatch(&allocate(DEVICE_1, first))),
        Some(first.address)
    );
}
