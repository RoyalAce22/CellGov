use std::collections::BTreeSet;
use std::path::Path;

use super::placeholder_console_tests::{hex, nodes as capture_nodes, PLACEHOLDER_CAPTURE};
use super::*;

const PEEK_HEADER: &str = "address\treads\tagree\tdoubleword";

fn registration_4_93() -> &'static StorageRegistration {
    registration("4.93").expect("the 4.93 registration")
}

fn device_4_93(id: u64) -> &'static StorageDevice {
    registration_4_93().device(id).expect("a table device")
}

/// v1 and v2 of the node `key` in the console captures of lpar 1's
/// `bus.4` nodes, after a check that they hold the key once.
fn captured(key: &RepositoryKey) -> (u64, u64) {
    let found: Vec<(u64, u64)> = [
        (REGISTRATION_CAPTURE, "node.tsv"),
        (REGISTRATION_CAPTURE, "region.tsv"),
        (PLACEHOLDER_CAPTURE, "node.tsv"),
    ]
    .into_iter()
    .flat_map(|(dir, name)| capture_nodes(dir, name))
    .filter(|(at, _, _)| at == key)
    .map(|(_, v1, v2)| (v1, v2))
    .collect();
    assert_eq!(found.len(), 1, "{}", key.path());
    found[0]
}

/// The doublewords of the captured bus table, four for each device, after
/// a check that both reads of each word agree.
fn captured_table() -> Vec<u64> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(REGISTRATION_CAPTURE)
        .join("table.tsv");
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(PEEK_HEADER), "{shown}: header");
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 4, "{shown}: {line:?}");
            assert_eq!((row[1], row[2]), ("2", "yes"), "{shown}: {line:?}");
            hex(row[3])
        })
        .collect()
}

#[test]
fn only_4_93_has_a_registration() {
    assert_eq!(registration("4.93"), REGISTRATIONS.first());
    for firmware in ["1.50", "1.94", "3.41", "3.70", "4.92"] {
        assert_eq!(registration(firmware), None, "{firmware}");
    }
}

#[test]
fn the_4_93_table_holds_devices_0_to_4_and_no_other_id() {
    let table = captured_table();
    assert_eq!(table.len(), 20);
    // The device id is +8 of each 32-byte entry [OFW-4.93 lv1 a:0x259540-0x25955C].
    let captured_ids: Vec<u64> = table.iter().skip(1).step_by(4).copied().collect();
    let ids: Vec<u64> = registration_4_93().devices.iter().map(|d| d.id).collect();
    assert_eq!(ids, captured_ids);
    assert_eq!(registration_4_93().device(5), None);
    assert_eq!(registration_4_93().device(u64::MAX), None);
}

#[test]
fn the_registration_of_device_3_writes_lv1s_nodes_in_lv1s_order() {
    let nodes = registration_nodes(registration_4_93(), device_4_93(3));
    let paths: Vec<String> = nodes.iter().map(|node| node.key.path()).collect();
    // [OFW-4.93 lv1 a:0x2524A4-0x2526E4] [OFW-4.93 lv1 a:0x25B16C-0x25B34C]
    let expected = [
        "bus+4.dev+3.id",
        "bus+4.dev+3.type",
        "bus+4.dev+3.intr",
        "bus+4.dev+3.region.id",
        "bus+4.dev+3.region.size",
        "bus+4.dev+3.region.start",
        "bus+4.dev+3.region.crypto",
        "bus+4.dev+3.region+1.id",
        "bus+4.dev+3.region+1.size",
        "bus+4.dev+3.region+1.start",
        "bus+4.dev+3.region+1.crypto",
        "bus+4.dev+3.region+2.id",
        "bus+4.dev+3.region+2.size",
        "bus+4.dev+3.region+2.start",
        "bus+4.dev+3.region+2.crypto",
        "bus+4.dev+3.n_regs",
        "bus+4.dev+3.blk_size",
        "bus+4.dev+3.n_blocks",
        "bus+4.dev+3.port",
        "bus+4.dev+3.meddling",
        "bus+4.dev+3.hosttype",
        "bus+4.id",
        "bus+4.type",
        "bus+4.num_dev",
    ];
    assert_eq!(paths, expected);
    assert!(nodes.iter().all(|node| node.key.lpar == 1 && node.v2 == 0));
    assert!(nodes
        .iter()
        .all(|node| node.key.n1 == first_field(b"bus", STORAGE_BUS_INDEX)));
}

#[test]
fn the_registration_of_device_3_writes_each_of_its_nodes_once_with_the_captured_value() {
    let nodes = registration_nodes(registration_4_93(), device_4_93(3));
    let keys: BTreeSet<RepositoryKey> = nodes.iter().map(|node| node.key).collect();
    assert_eq!(nodes.len(), 24);
    assert_eq!(keys.len(), nodes.len());
    for node in &nodes {
        assert_eq!(
            (node.v1, node.v2),
            captured(&node.key),
            "{}",
            node.key.path()
        );
    }
}

#[test]
fn the_initializer_writes_41_placeholders_for_each_id_0_to_6_and_3_bus_nodes() {
    // [OFW-4.93 lv1 a:0x252130-0x25238C] [OFW-4.93 lv1 a:0x252390]
    let nodes = placeholder_nodes();
    assert_eq!(nodes.len(), 7 * 41 + 3);
    let device_nodes = &nodes[..7 * 41];
    for id in 0..PLACEHOLDER_DEVICES {
        let own: Vec<&WrittenNode> = device_nodes
            .iter()
            .filter(|n| n.key.n2 == field(b"dev", id))
            .collect();
        assert_eq!(own.len(), 41, "device {id}");
        let types: Vec<u64> = own
            .iter()
            .filter(|n| n.key == device_key(id, b"type", 0, None))
            .map(|n| n.v1)
            .collect();
        assert_eq!(types, [PLACEHOLDER_TYPE], "device {id}");
    }
    let tail: Vec<(String, u64, u64)> = nodes[7 * 41..]
        .iter()
        .map(|n| (n.key.path(), n.v1, n.v2))
        .collect();
    // [OFW-4.93 lv1 a:0x25239C-0x2523AC] [OFW-4.93 lv1 a:0x2380B8-0x2380C4]
    assert_eq!(
        tail,
        [
            ("bus+4.id".to_string(), 4, 0),
            ("bus+4.type".to_string(), 5, 0),
            ("bus+4.num_dev".to_string(), 0, 0),
        ]
    );
}

#[test]
fn a_placeholder_region_id_holds_its_index_and_the_marker() {
    // [OFW-4.93 lv1 a:0x252130-0x25238C]
    let nodes = placeholder_nodes();
    let found: Vec<(u64, u64)> = nodes
        .iter()
        .filter(|n| n.key == device_key(6, b"region", 7, Some(b"id")))
        .map(|n| (n.v1, n.v2))
        .collect();
    assert_eq!(found, [(7, PLACEHOLDER_V2)]);
}

#[test]
fn no_node_answers_before_a_table_device_is_registered() {
    let key = device_key(3, b"port", 0, None);
    let reg = registration_4_93();
    assert_eq!(reg.node([], &key), None);
    assert_eq!(reg.node([5, 6], &key), None);
}

#[test]
fn a_registration_overwrites_only_its_own_device_and_the_bus_nodes() {
    let reg = registration_4_93();
    let at = |key: RepositoryKey| reg.node([3], &key);
    let port_3 = device_key(3, b"port", 0, None);
    assert_eq!(at(port_3), Some(captured(&port_3)));
    // The placeholders of device 4 [OFW-4.93 lv1 a:0x252130-0x25238C].
    assert_eq!(
        at(device_key(4, b"port", 0, None)),
        Some((0, PLACEHOLDER_V2))
    );
    assert_eq!(
        at(device_key(4, b"type", 0, None)),
        Some((PLACEHOLDER_TYPE, 0))
    );
    assert_eq!(
        at(device_key(3, b"region", 3, Some(b"id"))),
        Some((3, PLACEHOLDER_V2))
    );
    assert_eq!(
        at(bus_key(b"num_dev")),
        Some(captured(&bus_key(b"num_dev")))
    );
    assert_eq!(at(device_key(7, b"port", 0, None)), None);
    assert_eq!(at(bus_key(b"intr")), None);
}

#[test]
fn the_order_of_two_registrations_does_not_change_a_node() {
    let reg = registration_4_93();
    for key in [
        bus_key(b"num_dev"),
        device_key(1, b"n_regs", 0, None),
        device_key(4, b"region", 6, Some(b"crypto")),
    ] {
        let expected = Some(captured(&key));
        assert_eq!(reg.node([1, 4], &key), expected, "{}", key.path());
        assert_eq!(reg.node([4, 1], &key), expected, "{}", key.path());
    }
}

#[test]
fn each_notify_entry_is_its_captured_bus_table_entry() {
    let table = captured_table();
    let devices = registration_4_93().devices;
    assert_eq!(table.len(), 4 * devices.len());
    for (device, entry) in devices.iter().zip(table.chunks(4)) {
        assert_eq!(device.notify_entry()[..], *entry, "device {}", device.id);
    }
}
