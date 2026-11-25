//! The kernel's MMIO map of device 1 of bus 1 through the hypervisor,
//! held to the mapping that the console capture
//! `tests/micro/lv1_device_mmio_map` reads.

use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{ALLOCATE_MEMORY, CLOSE_DEVICE, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::device_bus::MMIO_MAP_CAPTURE;
use cellgov_time::GuestTicks;

/// The value of the row `field` of `mmio_map.tsv`, which the capture read
/// twice with agreeing values.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(MMIO_MAP_CAPTURE)
        .join("peek/cech20-cex-493/mmio_map.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split('\t').collect())
        .filter(|cells: &Vec<&str>| cells.get(1) == Some(&field))
        .collect();
    assert_eq!(rows.len(), 1, "{field}: {}", path.display());
    let row = &rows[0];
    assert_eq!(row[3..5], ["2", "yes"], "{field}");
    let digits = row[5].strip_prefix("0x").expect("0x hex");
    u64::from_str_radix(digits, 16).expect("a doubleword")
}

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
        tick: GuestTicks::new(36_735),
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
fn the_hypervisor_answers_the_census_call_with_the_captured_lpar_and_the_close_clears_it() {
    assert!(answered_numbers().contains(&MAP_DEVICE_MMIO_REGION));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(
        r3_r4(&hv.dispatch(&kernel_request(OPEN_DEVICE, &[1, 1, 0]))),
        Some((0, 0))
    );
    // Five 64 KB regions take the region counter values 2 to 6, as the
    // kernel's regions before the call do in CellGov's run.
    for _ in 0..5 {
        let allocate = kernel_request(ALLOCATE_MEMORY, &[0x1_0000, 16]);
        assert!(matches!(r3_r4(&hv.dispatch(&allocate)), Some((0, _))));
    }
    assert_eq!(hv.groups().memory().regions().next_counter(), Some(7));
    let opened = hv.sync_term();

    // The game OS mapping is the one of partition 2, in the region of
    // device slot 1; the region's address and the mapping's length are
    // the call's bus address and size.
    assert_eq!(captured("mapping_1_partition"), 2);
    assert_eq!(captured("mapping_1_region"), captured("device_slot_1"));
    let bus_address = captured("region_2_address");
    let size = captured("mapping_1_length");
    // The kernel's wrapper sends the page shift 12
    // [OFW-4.93 lv2_kernel a:0x800000000007D8B0].
    let call = kernel_request(MAP_DEVICE_MMIO_REGION, &[1, 1, bus_address, size, 12]);
    let lpar = captured("mapping_1_lpar");
    assert_eq!(r3_r4(&hv.dispatch(&call)), Some((0, lpar)));
    let mappings: Vec<_> = hv.groups().device_bus().mmio_mappings().collect();
    assert_eq!(
        mappings,
        [MmioMapping {
            bus: 1,
            device: 1,
            bus_address,
            size,
            lpar_address: lpar,
            page_shift: 12,
        }]
    );
    assert_ne!(hv.sync_term(), opened);

    assert_eq!(
        r3_r4(&hv.dispatch(&kernel_request(CLOSE_DEVICE, &[1, 1]))),
        Some((0, 0))
    );
    assert_eq!(hv.groups().device_bus().mmio_mappings().count(), 0);
}
