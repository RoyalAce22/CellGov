use super::*;
use crate::lv1::partition::LPAR_ID_PME;

/// The data rows of the capture file `file` under `capture`, each split
/// into its cells, after a check of the header.
fn capture_rows(capture: &str, file: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join(file);
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{shown}: header");
    let width = header.split('\t').count();
    lines
        .map(|line| {
            let row: Vec<String> = line.split('\t').map(str::to_string).collect();
            assert_eq!(row.len(), width, "{shown}: {line:?}");
            row
        })
        .collect()
}

/// The rows of the storage bus capture's `node.tsv`.
fn bus_4_rows() -> Vec<Vec<String>> {
    capture_rows(
        STORAGE_BUS_CAPTURE,
        "node.tsv",
        "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2",
    )
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

fn row_key(row: &[String]) -> [u64; 4] {
    [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])]
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

/// The two reads of the storage bus search for bus 4, in the order of
/// the search [OFW-4.93 lv2_kernel a:0x80000000002871e4-0x8000000000287228].
fn bus_4_keys() -> [RepositoryKey; 2] {
    [bus_field(4, b"id"), bus_field(4, b"type")]
}

fn lv1_address(node: &RepositoryNode) -> u64 {
    match node.source {
        NodeSource::Lv1NodeCapture { capture, address } => {
            assert_eq!(capture, STORAGE_BUS_CAPTURE, "{}", node.key);
            address
        }
        other => panic!("{} cites {other:?}", node.key),
    }
}

#[test]
fn each_bus_4_node_answers_its_capture_row_and_cites_it() {
    let rows = bus_4_rows();
    assert_eq!(rows.len(), 2);
    for key in bus_4_keys() {
        let matching: Vec<&Vec<String>> = rows
            .iter()
            .filter(|row| row_key(row) == [key.n1, key.n2, key.n3, key.n4])
            .collect();
        assert_eq!(matching.len(), 1, "one node.tsv row for {key}");
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
        assert_eq!(lv1_address(node), hex(&row[6]), "address of {key}");
    }
}

#[test]
fn the_served_nodes_that_cite_the_storage_bus_capture_are_its_two_rows() {
    let served: Vec<[u64; 4]> = NODES
        .iter()
        .filter(|node| node.source.citation() == STORAGE_BUS_CAPTURE)
        .map(|node| [node.key.n1, node.key.n2, node.key.n3, node.key.n4])
        .collect();
    let captured: Vec<[u64; 4]> = bus_4_rows().iter().map(|row| row_key(row)).collect();
    assert_eq!(served, captured);
}

/// The storage bus search stops at the first bus of type 5 and keeps v1
/// of that bus's id
/// [OFW-4.93 lv2_kernel a:0x8000000000287218-0x800000000028725c].
#[test]
fn bus_4_is_the_first_bus_of_type_5_in_the_captures_and_answers_its_captured_id() {
    const HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";
    let rows: Vec<Vec<String>> = capture_rows(BUS_CAPTURE, "node.tsv", HEADER)
        .into_iter()
        .chain(capture_rows(BUS_CAPTURE, "unread_node.tsv", HEADER))
        .chain(bus_4_rows())
        .collect();
    let captured_v1 = |key: RepositoryKey| {
        let matching: Vec<&Vec<String>> = rows
            .iter()
            .filter(|row| row_key(row) == [key.n1, key.n2, key.n3, key.n4])
            .collect();
        assert_eq!(matching.len(), 1, "one captured row for {key}");
        hex(&matching[0][9])
    };
    let captured: Vec<u64> = (0..5)
        .map(|bus| captured_v1(bus_field(bus, b"type")))
        .collect();
    let types: Vec<u64> = (0..5)
        .map(|bus| {
            lookup(&bus_field(bus, b"type"), "4.93")
                .unwrap_or_else(|| panic!("no 4.93 type row for bus {bus}"))
                .v1
        })
        .collect();
    assert_eq!(
        types, captured,
        "the served and captured types of buses 0 to 4"
    );
    assert_eq!(
        captured.iter().position(|&t| t == 5),
        Some(4),
        "{captured:?}"
    );
    let id = lookup(&bus_field(4, b"id"), "4.93").expect("a 4.93 row for bus.4.id");
    assert_eq!(id.v1, captured_v1(bus_field(4, b"id")));
}

/// lv1 keeps v1 at +104 of a node. The README of
/// `tests/micro/lv1_repository_debug_support` gives the layout.
#[test]
fn each_bus_4_v1_address_is_its_list_node_address_plus_104() {
    let list = capture_rows(
        LIST_CAPTURE,
        "list.tsv",
        "fw\tlpar\tn1\tn2\tn3\tn4\tnode_address\tsplit_order_key\treads\tagree",
    );
    for key in bus_4_keys() {
        let matching: Vec<&Vec<String>> = list
            .iter()
            .filter(|row| row_key(row) == [key.n1, key.n2, key.n3, key.n4])
            .collect();
        assert_eq!(matching.len(), 1, "one list.tsv row for {key}");
        let node = lookup(&key, "4.93").unwrap_or_else(|| panic!("no 4.93 row for {key}"));
        assert_eq!(
            lv1_address(node),
            hex(&matching[0][6]) + 104,
            "address of {key}"
        );
    }
}

#[test]
fn a_bus_4_read_at_a_firmware_other_than_4_93_finds_no_node() {
    for fw in ["1.50", "1.94", "3.70"] {
        for key in bus_4_keys() {
            assert!(lookup(&key, fw).is_none(), "{key} at {fw}");
        }
    }
}
