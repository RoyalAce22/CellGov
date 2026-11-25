use super::*;
use crate::lv1::partition::LPAR_ID_PME;

const HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";

/// The data rows of the bus capture's `unread_node.tsv`.
fn unread_rows() -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BUS_CAPTURE)
        .join("unread_node.tsv");
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(HEADER), "{shown}: header");
    lines
        .map(|line| {
            let row: Vec<String> = line.split('\t').map(str::to_string).collect();
            assert_eq!(row.len(), 11, "{shown}: {line:?}");
            row
        })
        .collect()
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The key `bus.<bus>.<word>` of lpar 1.
fn bus_field(bus: u64, word: &[u8]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", bus),
        n2: field(word, 0),
        n3: 0,
        n4: 0,
    }
}

/// The five nodes of the storage bus search that the 4.93 boot reads and
/// that only `unread_node.tsv` holds.
fn storage_search_keys() -> [RepositoryKey; 5] {
    [
        bus_field(0, b"id"),
        bus_field(2, b"id"),
        bus_field(2, b"type"),
        bus_field(3, b"id"),
        bus_field(3, b"type"),
    ]
}

/// The kernel's accessor builds `bus` with the bus index and then `id`
/// before its call of the read wrapper
/// [OFW-4.93 lv2_kernel a:0x8000000000096214-0x80000000000962b8].
#[test]
fn the_served_bus_0_id_key_is_the_words_the_4_93_kernel_builds() {
    let served: Vec<RepositoryKey> = NODES
        .iter()
        .map(|node| node.key)
        .filter(|key| key.path() == "bus.id")
        .collect();
    assert_eq!(served.len(), 1);
    let key = served[0];
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (1, 0x0000_0000_6275_7300, 0x6964_0000_0000_0000, 0, 0)
    );
}

#[test]
fn each_storage_search_node_answers_its_unread_node_row_and_cites_it() {
    let rows = unread_rows();
    let keys = storage_search_keys();
    assert_eq!(keys.len(), 5);
    for key in keys {
        let matching: Vec<&Vec<String>> = rows
            .iter()
            .filter(|row| {
                [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])]
                    == [key.n1, key.n2, key.n3, key.n4]
            })
            .collect();
        assert_eq!(matching.len(), 1, "one unread_node.tsv row for {key}");
        let row = matching[0];
        assert_eq!(
            (
                row[0].as_str(),
                row[1].as_str(),
                row[7].as_str(),
                row[8].as_str()
            ),
            ("4.93", "1", "2", "yes"),
            "{key}"
        );
        let node = lookup(&key, "4.93").unwrap_or_else(|| panic!("no 4.93 row for {key}"));
        assert_eq!(node.v1, hex(&row[9]), "v1 of {key}");
        assert_eq!(node.v2, SecondValue::Given(hex(&row[10])), "v2 of {key}");
        assert_eq!(
            node.source,
            NodeSource::Lv1NodeCapture {
                capture: BUS_CAPTURE,
                address: hex(&row[6]),
            },
            "{key}"
        );
    }
}

/// The storage bus search stops at the first bus of type 5
/// [OFW-4.93 lv2_kernel a:0x8000000000287218-0x8000000000287220]. No bus
/// of 0 to 3 has that type, so the search goes on to bus 4.
#[test]
fn no_bus_of_0_to_3_has_type_5_and_each_answers_its_id() {
    for bus in 0..4 {
        let type_node = lookup(&bus_field(bus, b"type"), "4.93")
            .unwrap_or_else(|| panic!("no 4.93 type row for bus {bus}"));
        assert_ne!(type_node.v1, 5, "bus {bus}");
        let id_node = lookup(&bus_field(bus, b"id"), "4.93")
            .unwrap_or_else(|| panic!("no 4.93 id row for bus {bus}"));
        assert_eq!(id_node.v1, bus, "the id of bus {bus}");
    }
}

/// The capture gives no value of bus 4, and the 4.93 boot sends no read
/// of the other bus 0, 2 and 3 nodes that `unread_node.tsv` holds.
#[test]
fn the_bus_reads_that_no_served_row_gives_stay_refused() {
    let unserved = [
        bus_field(0, b"num_dev"),
        bus_field(2, b"num_dev"),
        bus_field(3, b"num_dev"),
        bus_field(4, b"id"),
        bus_field(4, b"type"),
        RepositoryKey {
            n2: field(b"dev", 0),
            n3: field(b"id", 0),
            ..bus_field(0, b"dev")
        },
    ];
    assert_eq!(unserved.len(), 6);
    for key in unserved {
        assert!(lookup(&key, "4.93").is_none(), "{key}");
        assert!(lookup_absent(&key, "4.93").is_none(), "{key}");
    }
}

#[test]
fn a_storage_search_read_at_a_firmware_other_than_4_93_finds_no_node() {
    for fw in ["1.50", "1.94", "3.70"] {
        for key in storage_search_keys() {
            assert!(lookup(&key, fw).is_none(), "{key} at {fw}");
        }
    }
}
