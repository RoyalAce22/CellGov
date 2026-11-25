//! The ports of the 4.93 kernel's storage device interrupts, after the
//! port of its storage bus interrupt, held to the console capture of the
//! port that lv1 connects to each storage device.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::CONSTRUCT_EVENT_RECEIVE_PORT;
use cellgov_ps3_abi::lv1::event_port::{
    game_os_port_id, STORAGE_DEVICE_PORT_CAPTURE, STORAGE_PORT_CAPTURE,
};
use cellgov_time::GuestTicks;
use std::path::Path;

/// The `sc` of the kernel's call that makes the network device's
/// interrupt port [OFW-4.93 lv2_kernel a:0x80000000000a28e0].
const NETWORK_SC: u64 = 0x8000_0000_000a_28e0;

/// The `sc` of the kernel's port setup of an interrupt object of type 2
/// [OFW-4.93 lv2_kernel a:0x8000000000096494].
const STORAGE_SC: u64 = 0x8000_0000_0009_6494;

/// The global that holds the storage bus object, from the TOC slot at
/// r2 - 26656 [OFW-4.93 lv1 a:0x353818-0x35381C].
const STORAGE_BUS_GLOBAL: u64 = 0x36_8388;

/// The offset in the storage bus object of the port that slot 1's handle
/// holds [OFW-4.93 lv1 a:0x25554C-0x255570] [OFW-4.93 lv1 a:0x2BEDD4].
const BUS_PORT_OFFSET: u64 = 2568 + 64 + 16;

/// The offset in the storage bus object of the device object of device
/// 0; device `id` is 8 x `id` higher [OFW-4.93 lv1 a:0x2369AC-0x2369B4].
const DEVICES_OFFSET: u64 = 3816;

/// The offset in a device object of slot 0's partition word; slot 1's is
/// 64 bytes higher [OFW-4.93 lv1 a:0x2554F4] [OFW-4.93 lv1 a:0x255508].
const SLOT_PARTITION_OFFSET: u64 = 2544;

/// The offset in a device object of the port that slot 0's handle holds;
/// slot 1's is 64 bytes higher [OFW-4.93 lv1 a:0x25554C-0x255554]
/// [OFW-4.93 lv1 a:0x255520-0x255524].
const SLOT_PORT_OFFSET: u64 = 2568 + 16;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The partition word of the game OS partition in a device's slot; see
/// the capture of the storage device opens.
const GAME_OS_PARTITION: u64 = 2;

/// The value of the doubleword at `address` in the file `name` of the
/// peek capture `capture`. Panics unless both reads of it agree.
fn captured(capture: &str, name: &str, address: u64) -> u64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join("peek/cech20-cex-493")
        .join(name);
    let file = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
    assert_eq!(
        text.lines().next(),
        Some("address\treads\tagree\tdoubleword"),
        "{file}: header"
    );
    let hex = |cell: &str| {
        let digits = cell
            .strip_prefix("0x")
            .unwrap_or_else(|| panic!("{file}: {cell} is not 0x hex"));
        u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{file}: {cell}: {e}"))
    };
    let row: Vec<&str> = text
        .lines()
        .skip(1)
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.first().map(|cell| hex(cell)) == Some(address))
        .unwrap_or_else(|| panic!("{file}: no row for {address:#x}"));
    assert_eq!(row.len(), 4, "{file}: {address:#x} columns");
    assert_eq!(row[1..3], ["2", "yes"], "{file}: {address:#x}");
    hex(row[3])
}

/// The id of the port that slot 1's handle of the storage bus object
/// holds on the reference console.
fn bus_port_id() -> u64 {
    let bus = captured(STORAGE_PORT_CAPTURE, "bus.tsv", STORAGE_BUS_GLOBAL);
    let port = captured(STORAGE_PORT_CAPTURE, "bus_port.tsv", bus + BUS_PORT_OFFSET);
    captured(STORAGE_PORT_CAPTURE, "port.tsv", port + ID_OFFSET)
}

/// The ids of the ports that lv1 connects to the storage devices for the
/// game OS partition on the reference console, in ascending order: the
/// order in which lv1 gives the ids.
fn device_port_ids() -> Vec<u64> {
    let read = |name: &str, address: u64| captured(STORAGE_DEVICE_PORT_CAPTURE, name, address);
    let bus = read("bus.tsv", STORAGE_BUS_GLOBAL);
    let mut ids: Vec<u64> = (0..5)
        .map(|device| {
            let object = read("devices.tsv", bus + DEVICES_OFFSET + 8 * device);
            let slot = (0..2)
                .find(|slot| {
                    read("slots.tsv", object + SLOT_PARTITION_OFFSET + 64 * slot)
                        == GAME_OS_PARTITION
                })
                .unwrap_or_else(|| panic!("device {device}: no game OS slot"));
            let port = read("device_port.tsv", object + SLOT_PORT_OFFSET + 64 * slot);
            read("port.tsv", port + ID_OFFSET)
        })
        .collect();
    ids.sort_unstable();
    ids
}

fn construct(thread: u8, pc: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: CONSTRUCT_EVENT_RECEIVE_PORT,
        inputs: [0xA5A5_A5A5_A5A5_A5A5; 8],
        pc,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::new(35_493),
    }
}

fn constructed(id: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [id, 0, 0, 0, 0, 0, 0]);
    answer.push_change(InterruptChange::PortConstructed(EventReceivePort::new(id)));
    HypercallDisposition::Answered(answer)
}

/// A 4.93 hypervisor after the network device's call and the storage
/// bus's call.
fn after_storage_bus_call() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let network = game_os_port_id("4.93", 0, NETWORK_SC).expect("the network call's port id");
    assert_eq!(hv.dispatch(&construct(0, NETWORK_SC)), constructed(network));
    assert_eq!(
        hv.dispatch(&construct(0, STORAGE_SC)),
        constructed(bus_port_id())
    );
    hv
}

#[test]
fn the_five_storage_device_calls_get_the_captured_ids_in_ascending_order_and_are_kept() {
    let mut hv = after_storage_bus_call();
    let ids = device_port_ids();
    assert_eq!(ids.len(), 5);
    for &id in &ids {
        let before = hv.sync_term();
        assert_eq!(hv.dispatch(&construct(0, STORAGE_SC)), constructed(id));
        assert_eq!(
            hv.event_receive_ports().port(id),
            Some(&EventReceivePort::new(id))
        );
        assert_ne!(hv.sync_term(), before, "port {id}");
    }
    let held: Vec<u64> = hv.event_receive_ports().ports().keys().copied().collect();
    assert_eq!(held[2..], ids[..]);
    assert_eq!(held.len(), 7);
    assert_eq!(hv.refusal(), None);
}

#[test]
fn a_call_at_the_storage_sc_after_the_six_storage_ports_is_refused_and_keeps_them() {
    let mut hv = after_storage_bus_call();
    let ids = device_port_ids();
    assert_eq!(ids.len(), 5);
    for &id in &ids {
        assert_eq!(hv.dispatch(&construct(0, STORAGE_SC)), constructed(id));
    }
    let held = hv.event_receive_ports().clone();
    let term = hv.sync_term();
    let r = construct(0, STORAGE_SC);
    assert_eq!(
        hv.dispatch(&r),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: CONSTRUCT_EVENT_RECEIVE_PORT,
            unit: r.unit,
            pc: r.pc,
            what: "an event receive port call whose port id the hypervisor already holds",
        })
    );
    assert_eq!(hv.event_receive_ports(), &held);
    assert_eq!(hv.sync_term(), term);
}
