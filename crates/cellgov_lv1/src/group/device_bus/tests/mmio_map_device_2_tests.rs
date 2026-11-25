//! The kernel's MMIO maps of device 2 of bus 1 through the hypervisor,
//! after its maps of device 1, held to the rows that the console
//! captures of both devices read.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_MEMORY, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::GAME_OS_MMIO_MAPPINGS;
use cellgov_time::GuestTicks;

/// The kernel's call `number` with r3 onwards from `registers`.
fn kernel_request(number: u64, registers: &[u64]) -> HypercallRequest {
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
        tick: GuestTicks::new(38_675),
    }
}

/// The r3 and r4 of an answer, or `None` for a refusal.
fn r3_r4(disposition: &HypercallDisposition) -> Option<(u64, u64)> {
    match disposition {
        HypercallDisposition::Answered(answer) => {
            Some((answer.r3, answer.outputs().map_or(u64::MAX, |out| out[0])))
        }
        HypercallDisposition::Refused(_) => None,
    }
}

#[test]
fn the_census_calls_for_device_2_take_the_counter_values_9_and_10_after_device_1() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for device in [1, 2] {
        let open = kernel_request(OPEN_DEVICE, &[1, device, 0]);
        assert_eq!(r3_r4(&hv.dispatch(&open)), Some((0, 0)), "{device}");
    }
    // Five 64 KB regions take the region counter values 2 to 6, as the
    // kernel's regions before its MMIO maps do in CellGov's run.
    for _ in 0..5 {
        let allocate = kernel_request(ALLOCATE_MEMORY, &[0x1_0000, 16]);
        assert!(matches!(r3_r4(&hv.dispatch(&allocate)), Some((0, _))));
    }
    assert_eq!(hv.groups().memory().regions().next_counter(), Some(7));

    let rows: Vec<_> = GAME_OS_MMIO_MAPPINGS
        .iter()
        .filter(|row| row.firmware == "4.93")
        .collect();
    let calls: Vec<_> = rows
        .iter()
        .map(|row| (row.device, row.bus_address))
        .collect();
    assert_eq!(
        calls,
        [
            (1, 0x301_0000),
            (1, 0x381_0000),
            (2, 0x302_0000),
            (2, 0x382_0000)
        ]
    );
    for row in &rows {
        let call = kernel_request(
            MAP_DEVICE_MMIO_REGION,
            &[
                row.bus,
                row.device,
                row.bus_address,
                row.size,
                row.page_shift,
            ],
        );
        assert_eq!(
            r3_r4(&hv.dispatch(&call)),
            Some((0, row.lpar_address)),
            "{:#x}",
            row.bus_address
        );
    }
    assert_eq!(hv.groups().memory().regions().next_counter(), Some(11));
    let device_2: Vec<(u64, u64)> = hv
        .groups()
        .device_bus()
        .mmio_mappings()
        .filter(|m| m.device == 2)
        .map(|m| (m.bus_address, m.lpar_address))
        .collect();
    assert_eq!(
        device_2,
        [
            (0x302_0000, 0x4000_0009_0000),
            (0x382_0000, 0x4000_000A_0000)
        ]
    );
}
