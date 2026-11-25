//! The 4.93 port id of the kernel's call that makes the network device's
//! interrupt port, held to the console capture of that port in lv1.

use super::*;

/// The lv1 global that holds the connection object of the network
/// device's interrupt, from the TOC slot r2 - 26624
/// [OFW-4.93 lv1 a:0x353838].
const CONNECTION_GLOBAL: u64 = 0x3682e8;

/// The offset in the connection object of the connected port: +16 of
/// the handle at +16 [OFW-4.93 lv1 a:0x2BEDD4].
const CONNECTION_PORT_OFFSET: u64 = 32;

/// The offset in an object of its id in the partition's object table
/// [OFW-4.93 lv1 a:0x2CC6DC-0x2CC6EC].
const ID_OFFSET: u64 = 72;

/// The class that the connect function of hypercall 197 checks, from
/// the TOC slot r2 - 24632 [OFW-4.93 lv1 a:0x354000].
const PORT_CLASS: u64 = 0x329310;

/// The word that `construct_event_receive_port` gives the port
/// constructor for +136, from the TOC slot r2 - 7896
/// [OFW-4.93 lv1 a:0x358160].
const PORT_WORD: u64 = 0x3298d2;

/// The offset in a partition of its object table
/// [OFW-4.93 lv1 a:0x245FFC-0x246008].
const TABLE_OFFSET: u64 = 88;

/// The address and the value of the field `field` in the capture. Both
/// reads of the doubleword must agree.
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

#[test]
fn the_network_call_gets_the_id_of_the_port_lv1_connects_to_the_network_device() {
    let (address, connection) = captured("connection");
    assert_eq!(address, CONNECTION_GLOBAL, "the connection global");
    let (address, port) = captured("port");
    assert_eq!(address, connection + CONNECTION_PORT_OFFSET, "the port row");
    assert_eq!(captured("port_class"), (port + 48, PORT_CLASS), "class");
    assert_eq!(captured("port_word"), (port + 136, PORT_WORD), "+136");
    let (address, id) = captured("port_id");
    assert_eq!(address, port + ID_OFFSET, "the id row");
    assert_eq!(game_os_port_id("4.93", 0, NETWORK_SC), Some(id));
}

#[test]
fn the_partition_table_slot_of_the_network_call_id_holds_the_connected_port() {
    let (_, port) = captured("port");
    let id = game_os_port_id("4.93", 0, NETWORK_SC).expect("the network call's port id");
    let (address, partition) = captured("port_partition_56");
    assert_eq!(address, port + 56, "+56");
    assert_eq!(captured("port_partition_120"), (port + 120, partition));
    let table = partition + TABLE_OFFSET;
    let (address, segments) = captured("table_segments");
    assert_eq!(address, table + 24, "the segment array row");
    let (address, shape) = captured("table_shape");
    assert_eq!(address, table + 32, "the shape row");
    assert!(id < 1 << (shape >> 32), "id {id} is in segment 0");
    let (address, segment) = captured("segment_0");
    assert_eq!(address, segments, "the first segment row");
    let (address, stride) = captured("segment_stride");
    assert_eq!(address, segment + 48, "the stride row");
    let (address, base) = captured("segment_base");
    assert_eq!(address, segment + 72, "the base row");
    assert_eq!(captured("slot_19"), (base + id * stride, port), "the slot");
}
