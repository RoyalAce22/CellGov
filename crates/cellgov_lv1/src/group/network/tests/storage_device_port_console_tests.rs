//! The 4.93 hypervisor's answers to the kernel's
//! `connect_interrupt_event_receive_port` for the storage devices, held to
//! the `lv1_storage_device_port_ids` capture of the port that lv1
//! connected to each device's game OS slot on the reference console.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONNECT_INTERRUPT_EVENT_RECEIVE_PORT, CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE,
};
use cellgov_ps3_abi::lv1::event_port::STORAGE_DEVICE_PORT_CAPTURE;
use cellgov_time::GuestTicks;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of the device object of device
/// 0; device `id` is 8 x `id` higher [OFW-4.93 lv1 a:0x2369AC-0x2369B4].
const DEVICES_OFFSET: u64 = 3816;

/// The offset in a device object of slot 0's partition word; slot 1's is
/// 64 bytes higher [OFW-4.93 lv1 a:0x2554F4] [OFW-4.93 lv1 a:0x255508].
const SLOT_PARTITION_OFFSET: u64 = 2544;

/// The offset in a device object of the port at +16 of slot 0's handle;
/// slot 1's is 64 bytes higher [OFW-4.93 lv1 a:0x25554C-0x255554]
/// [OFW-4.93 lv1 a:0x255520-0x255524] [OFW-4.93 lv1 a:0x2BEDD4].
const SLOT_PORT_OFFSET: u64 = 2568 + 16;

/// The offset in a partition of the word that the storage connect
/// compares with a slot's partition word [OFW-4.93 lv1 a:0x2554F8-0x255500].
const PARTITION_ID_OFFSET: u64 = 80;

/// The offset in a port of its partition [OFW-4.93 lv1 a:0x2DF670].
const PORT_PARTITION_OFFSET: u64 = 120;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The `sc` of the kernel's `open_device` of the storage bus
/// [OFW-4.93 lv2_kernel a:0x80000000000AE110].
const KERNEL_BUS_OPEN_SC: u64 = 0x8000_0000_000A_E110;

/// The `sc` of the kernel device object's `open_device`
/// [OFW-4.93 lv2_kernel a:0x80000000000AC120].
const KERNEL_DEVICE_OPEN_SC: u64 = 0x8000_0000_000A_C120;

/// The r5 of the kernel device object's `open_device`
/// [OFW-4.93 lv2_kernel a:0x80000000000AC0FC-0x80000000000AC118].
const KERNEL_DEVICE_OPEN_R5: u64 = 0x8000_0000_0000_0000;

/// The `sc` of the port setup's `construct_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const KERNEL_PORT_SC: u64 = 0x8000_0000_0009_6494;

/// The `sc` of the port setup's `connect_interrupt_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x80000000000964CC].
const KERNEL_SC: u64 = 0x8000_0000_0009_64CC;

/// The value of the doubleword at `address` in the capture's file
/// `name`. Panics unless the file holds a row for it.
fn captured(name: &str, address: u64) -> u64 {
    captured_row(name, address).unwrap_or_else(|| panic!("{name}: no row for {address:#x}"))
}

/// The value of the doubleword at `address` in the capture's file
/// `name`, or `None` when the file holds no row for it. Panics unless
/// both reads of a doubleword agree.
fn captured_row(name: &str, address: u64) -> Option<u64> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(STORAGE_DEVICE_PORT_CAPTURE)
        .join("peek/cech20-cex-493")
        .join(name);
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\treads\tagree\tdoubleword"),
        "{file}: header"
    );
    let hex = |cell: &str| {
        let digits = cell
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("{file}: {cell} is not 0x hex"));
        u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {cell}: {e}"))
    };
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.first().map(|cell| hex(cell)) == Some(address))?;
    assert_eq!(row.len(), 4, "{file}: {address:#x} columns");
    assert_eq!(row[1..3], ["2", "yes"], "{file}: {address:#x}");
    Some(hex(row[3]))
}

/// The id of the port that the console's lv1 holds in the game OS slot of
/// storage device `device`.
///
/// The game OS slot holds a port of the game OS partition, and its
/// partition word is the word at +80 of that partition, as the storage
/// connect compares them. The capture reads only the game OS ports.
fn console_device_port_id(device: u64) -> u64 {
    let bus = captured("bus.tsv", STORAGE_BUS_GLOBAL);
    let object = captured("devices.tsv", bus + DEVICES_OFFSET + 8 * device);
    let slots: Vec<u64> = (0..2)
        .filter(|slot| {
            let port = captured("device_port.tsv", object + SLOT_PORT_OFFSET + 64 * slot);
            captured_row("port.tsv", port + PORT_PARTITION_OFFSET).is_some_and(|partition| {
                captured("slots.tsv", object + SLOT_PARTITION_OFFSET + 64 * slot)
                    == captured("partition.tsv", partition + PARTITION_ID_OFFSET)
            })
        })
        .collect();
    assert_eq!(slots.len(), 1, "device {device}: game OS slots {slots:?}");
    let port = captured("device_port.tsv", object + SLOT_PORT_OFFSET + 64 * slots[0]);
    captured("port.tsv", port + ID_OFFSET)
}

/// The kernel's call `number` from the `sc` at `pc` with r3 to r6.
fn kernel_call(number: u64, pc: u64, r3_to_r6: [u64; 4]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..4].copy_from_slice(&r3_to_r6);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

/// The answer of `request`. Panics when the hypervisor refuses it.
fn answer(hv: &mut Hypervisor, request: &HypercallRequest) -> HypercallAnswer {
    match hv.dispatch(request) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal:?}"),
    }
}

/// The kernel makes a port with hypercall 18 and gives its id to
/// hypercall 197 for device `device` of bus 4
/// [OFW-4.93 lv2_kernel a:0x8000000000096490-0x80000000000964CC].
fn connect_new_port(hv: &mut Hypervisor, device: u64) -> HypercallAnswer {
    let made = answer(
        hv,
        &kernel_call(CONSTRUCT_EVENT_RECEIVE_PORT, KERNEL_PORT_SC, [0; 4]),
    );
    assert_eq!(made.r3, 0, "construct_event_receive_port");
    let port = made.outputs().expect("a table return")[0];
    answer(
        hv,
        &kernel_call(
            CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
            KERNEL_SC,
            [4, device, port, 0],
        ),
    )
}

#[test]
fn each_storage_device_call_connects_the_port_that_the_console_game_os_slot_holds() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = kernel_call(OPEN_DEVICE, KERNEL_BUS_OPEN_SC, [4, u64::MAX, 0, 0]);
    assert_eq!(
        answer(&mut hv, &open).r3,
        0,
        "open_device of the storage bus"
    );
    assert_eq!(connect_new_port(&mut hv, u64::MAX).r3, 0, "the storage bus");
    let devices = [0, 1, 2, 3, 4];
    for device in devices {
        let open = kernel_call(
            OPEN_DEVICE,
            KERNEL_DEVICE_OPEN_SC,
            [4, device, KERNEL_DEVICE_OPEN_R5, 0],
        );
        assert_eq!(answer(&mut hv, &open).r3, 0, "open_device {device}");
        let connected = connect_new_port(&mut hv, device);
        assert_eq!(connected.r3, 0, "device {device}");
        assert_eq!(
            connected.outputs(),
            Some([0; 7]),
            "device {device}: r4 to r10"
        );
    }
    let held: Vec<(u64, u64)> = hv
        .groups()
        .device_bus()
        .storage_device_ports()
        .map(|port| (port.device, port.port))
        .collect();
    let console: Vec<(u64, u64)> = devices
        .iter()
        .map(|&device| (device, console_device_port_id(device)))
        .collect();
    assert_eq!(held.len(), 5);
    assert_eq!(held, console);
}
