//! The hypervisor leaves device 0 of bus 1 open after the 4.93 kernel's
//! open, as lv1 does on the reference console, read from the committed
//! capture `tests/micro/lv1_device_bus_open`.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::GET_LOGICAL_PARTITION_ID;
use cellgov_time::GuestTicks;

const CAPTURE: &str = "tests/micro/lv1_device_bus_open/peek/cech20-cex-493";

/// The slot of device 0 in the system bus object
/// [OFW-4.93 lv1 a:0x22B838-0x22B844].
const SLOT_0: u64 = 32;

/// The open flag byte and the partition doubleword of the device
/// [OFW-4.93 lv1 a:0x2280B0-0x2280BC].
pub(super) const OPEN_FLAG: u64 = 108;
pub(super) const PARTITION: u64 = 112;

/// The method table that the device's constructor stores at +0
/// [OFW-4.93 lv1 a:0x22DDC0-0x22DDD4] [OFW-4.93 lv1 a:0x353480-0x353484].
pub(super) const DEVICE_METHOD_TABLE: u64 = 0x35_C878;

/// The address and value of each row of the capture's file `name`, whose
/// value column is `column`. Each row must hold two reads that agree.
pub(super) fn captured(name: &str, column: &str) -> Vec<(u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CAPTURE)
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some(format!("address\treads\tagree\t{column}").as_str()),
        "{}",
        path.display()
    );
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells[1..3], ["2", "yes"], "{name}: {line}");
            (hex(cells[0]), hex(cells[3]))
        })
        .collect()
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

pub(super) fn one(rows: Vec<(u64, u64)>) -> (u64, u64) {
    assert_eq!(rows.len(), 1, "{rows:?}");
    rows[0]
}

pub(super) fn kernel_request(number: u64, inputs: [u64; 8]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0x8000_0000_0009_5718,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The captured open flag and partition, after a check that the capture
/// read them from the device in slot 0.
fn device_capture() -> (u64, u64) {
    let walk = captured("walk.tsv", "doubleword");
    let [(_, bus_object), (slot, device), (device_at, method_table)] = walk[..] else {
        panic!("walk.tsv holds 3 rows: {walk:?}");
    };
    assert_eq!(slot, bus_object + SLOT_0, "slot 0");
    assert_eq!(device_at, device, "the device's +0");
    assert_eq!(
        method_table, DEVICE_METHOD_TABLE,
        "the device's method table"
    );
    let (flag_at, flag) = one(captured("open_flag.tsv", "byte"));
    let (partition_at, partition) = one(captured("partition.tsv", "doubleword"));
    assert_eq!(flag_at, device + OPEN_FLAG, "the open flag's address");
    assert_eq!(partition_at, device + PARTITION, "the partition's address");
    (flag, partition)
}

/// The kernel opens device 0 of bus 1 with r5 = 0
/// [OFW-4.93 lv2_kernel a:0x8000000000095704-0x8000000000095718].
#[test]
fn the_kernel_open_leaves_bus_1_device_0_open_for_its_partition_as_lv1_does() {
    let (flag, partition) = device_capture();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let HypercallDisposition::Answered(answer) =
        hv.dispatch(&kernel_request(OPEN_DEVICE, [1, 0, 0, 0, 0, 0, 0, 0]))
    else {
        panic!("the kernel's open_device is refused");
    };
    assert_eq!(answer.r3, 0, "r3");
    let open = hv
        .groups()
        .device_bus()
        .is_open(OpenDevice { bus: 1, device: 0 });
    assert_eq!(u64::from(open), flag, "the open flag");
    let HypercallDisposition::Answered(id) =
        hv.dispatch(&kernel_request(GET_LOGICAL_PARTITION_ID, [0; 8]))
    else {
        panic!("get_logical_partition_id is refused");
    };
    assert_eq!(
        id.outputs().map(|outputs| outputs[0]),
        Some(partition),
        "the partition that keeps the device open"
    );
}
