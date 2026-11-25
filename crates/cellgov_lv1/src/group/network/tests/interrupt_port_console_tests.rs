//! The 4.93 hypervisor's answer to the kernel's
//! `connect_interrupt_event_receive_port`, held to the
//! `lv1_network_port_id` capture of the port that lv1 connected to the
//! network device's interrupt on the reference console after boot.

use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_EVENT_RECEIVE_PORT, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::event_port::NETWORK_PORT_CAPTURE;
use cellgov_time::GuestTicks;

/// The lv1 global that holds the connection object of the network
/// device's interrupt, from the TOC slot r2 - 26624
/// [OFW-4.93 lv1 a:0x353838].
const CONNECTION_GLOBAL: u64 = 0x3682e8;

/// The offset in the connection object of the connected port
/// [OFW-4.93 lv1 a:0x246140-0x246150] [OFW-4.93 lv1 a:0x2BEDD4].
const CONNECTION_PORT_OFFSET: u64 = 32;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The `sc` of the kernel's `construct_event_receive_port` that makes the
/// port of its call [OFW-4.93 lv2_kernel a:0x80000000000A28E0].
const KERNEL_PORT_SC: u64 = 0x8000_0000_000A_28E0;

/// The `sc` of the kernel's `connect_interrupt_event_receive_port`
/// [OFW-4.93 lv2_kernel a:0x80000000000A2930].
const KERNEL_SC: u64 = 0x8000_0000_000A_2930;

/// The address and the value of the field `field` in the capture. Panics
/// unless the capture's two reads of the doubleword agree.
fn captured(field: &str) -> (u64, u64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(NETWORK_PORT_CAPTURE)
        .join("peek/cech20-cex-493/network_port.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!(row.len(), 6, "{name}: {field} columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
    };
    (hex(row[0]), hex(row[5]))
}

/// The id of the port that the console's connection object holds.
fn console_connected_port_id() -> u64 {
    let (address, connection) = captured("connection");
    assert_eq!(address, CONNECTION_GLOBAL, "the connection global");
    let (address, port) = captured("port");
    assert_eq!(address, connection + CONNECTION_PORT_OFFSET, "the port row");
    let (address, id) = captured("port_id");
    assert_eq!(address, port + ID_OFFSET, "the id row");
    id
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

/// The kernel opens the network device, makes its port with hypercall
/// 18 and gives that id to hypercall 197
/// [OFW-4.93 lv2_kernel a:0x80000000000A291C-0x80000000000A292C].
#[test]
fn the_kernel_call_connects_the_port_that_the_console_connection_holds() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = kernel_call(OPEN_DEVICE, KERNEL_SC, [1, 0, 0, 0]);
    assert_eq!(answer(&mut hv, &open).r3, 0, "open_device");
    let made = answer(
        &mut hv,
        &kernel_call(CONSTRUCT_EVENT_RECEIVE_PORT, KERNEL_PORT_SC, [0; 4]),
    );
    assert_eq!(made.r3, 0, "construct_event_receive_port");
    let port = made.outputs().expect("a table return")[0];
    let connect = kernel_call(
        CONNECT_INTERRUPT_EVENT_RECEIVE_PORT,
        KERNEL_SC,
        [1, 0, port, 0],
    );
    let connected = answer(&mut hv, &connect);
    assert_eq!(connected.r3, 0, "connect_interrupt_event_receive_port");
    assert_eq!(connected.outputs(), Some([0; 7]), "r4 to r10");
    assert_eq!(
        hv.groups().network().interrupt_port(),
        Some(console_connected_port_id())
    );
}
