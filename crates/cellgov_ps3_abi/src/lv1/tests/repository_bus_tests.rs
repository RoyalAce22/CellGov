use super::*;
use crate::lv1::partition::LPAR_ID_PME;

/// The header of the capture's `node.tsv` and `unread_node.tsv`.
const HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";

/// The data rows of the capture's `node.tsv`, each split into its cells.
fn captured_nodes() -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BUS_CAPTURE)
        .join("node.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(HEADER), "{name}: header");
    lines
        .map(|line| {
            let row: Vec<String> = line.split('\t').map(str::to_string).collect();
            assert_eq!(row.len(), 11, "{name}: {row:?}");
            assert_eq!(
                (row[7].as_str(), row[8].as_str()),
                ("2", "yes"),
                "{name}: two reads that agree: {row:?}"
            );
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

/// The key of lpar 1's bus `bus` with the last three key words `n`.
fn bus(bus: u64, n: [u64; 3]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", bus),
        n2: n[0],
        n3: n[1],
        n4: n[2],
    }
}

fn bus_type(index: u64) -> RepositoryKey {
    bus(index, [field(b"type", 0), 0, 0])
}

/// The kernel builds these words in r3 to r7 before its `sc`
/// [OFW-4.93 lv2_kernel a:0x80000000000922d4-0x80000000000922fc]: "bus"
/// with index 0, then "type".
#[test]
fn the_served_bus_0_type_key_is_the_words_the_4_93_kernel_builds() {
    let key = bus_type(0);
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (1, 0x0000_0000_6275_7300, 0x7479_7065_0000_0000, 0, 0)
    );
    assert!(lookup(&key, "4.93").is_some());
}

#[test]
fn every_4_93_bus_node_answers_the_values_of_the_lv1_capture_and_cites_it() {
    let rows = captured_nodes();
    let served: Vec<&RepositoryNode> = NODES
        .iter()
        .filter(|node| node.source.citation() == BUS_CAPTURE)
        .collect();
    assert_eq!(served.len(), rows.len(), "one served node per captured row");
    assert!(!rows.is_empty());
    for row in &rows {
        assert_eq!(row[0], "4.93", "the captured firmware: {row:?}");
        let key = RepositoryKey {
            lpar: row[1].parse::<u64>().expect("a decimal lpar"),
            n1: hex(&row[2]),
            n2: hex(&row[3]),
            n3: hex(&row[4]),
            n4: hex(&row[5]),
        };
        assert_eq!(key.lpar, LPAR_ID_PME, "{key}");
        assert_eq!(key.n1 & !0xff, first_field(b"bus", 0), "{key}");
        let node = lookup(&key, &row[0]).unwrap_or_else(|| panic!("no 4.93 row for {key}"));
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
        assert_eq!(node.firmware, ["4.93"], "{key}");
    }
}

/// The kernel's bus search asks for type 4
/// [OFW-4.93 lv2_kernel a:0x8000000000277910], and stops at the first bus
/// of that type [OFW-4.93 lv2_kernel a:0x8000000000277818-0x8000000000277820].
#[test]
fn bus_1_is_the_first_4_93_bus_of_type_4() {
    let type_of = |index| {
        lookup(&bus_type(index), "4.93")
            .unwrap_or_else(|| panic!("no 4.93 row for bus {index}"))
            .v1
    };
    assert_eq!(type_of(0), 3);
    assert_eq!(type_of(1), 4);
    assert!(lookup(&bus_type(2), "4.93").is_none());
}

#[test]
fn a_floor_read_of_a_bus_node_is_refused() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(lookup(&bus_type(0), fw).is_none(), "{fw}");
        assert!(lookup(&bus_type(1), fw).is_none(), "{fw}");
    }
}
