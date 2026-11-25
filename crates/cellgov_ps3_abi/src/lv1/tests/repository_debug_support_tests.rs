use super::*;
use crate::lv1::partition::LPAR_ID_PME;

fn debug_support() -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"ss", 0),
        n2: field(b"param", 0),
        n3: field(b"debug", 0),
        n4: field(b"support", 0),
    }
}

/// The 4.93 kernel builds these words in r4 to r7 before its `sc`
/// [OFW-4.93 lv2_kernel a:0x8000000000050a08-0x8000000000050a48].
#[test]
fn the_served_debug_support_key_is_the_words_the_4_93_kernel_builds() {
    let served: Vec<RepositoryKey> = NODES
        .iter()
        .map(|node| node.key)
        .filter(|key| key.path() == "ss.param.debug.support")
        .collect();
    assert_eq!(served.len(), 1);
    let key = served[0];
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (
            1,
            0x0000_0000_7373_0000,
            0x7061_7261_6d00_0000,
            0x6465_6275_6700_0000,
            0x7375_7070_6f72_7400,
        )
    );
}

/// The one data row of the capture's `node.tsv`, split into its cells.
fn captured_node() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(DEBUG_SUPPORT_CAPTURE)
        .join("node.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2"),
        "{name}: header"
    );
    let row: Vec<String> = lines
        .next()
        .unwrap_or_else(|| panic!("{name}: no data row"))
        .split('\t')
        .map(str::to_string)
        .collect();
    assert_eq!(lines.next(), None, "{name}: one data row");
    assert_eq!(row.len(), 11, "{name}: {row:?}");
    assert_eq!(
        (row[7].as_str(), row[8].as_str()),
        ("2", "yes"),
        "{name}: two reads that agree"
    );
    row
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// lpar 1's update manager sets both values to all-ones on a unit whose
/// target id is not 0x81 or 0x82 [OFW-4.93 lv1 a:0x4F4874-0x4F48E4], and
/// the reference console is a retail unit.
#[test]
fn the_4_93_debug_support_node_answers_all_ones_in_both_values_cited_to_the_lv1_capture() {
    let row = captured_node();
    let key = debug_support();
    assert_eq!(row[0], "4.93", "the captured firmware");
    assert_eq!(row[1].parse::<u64>().expect("a decimal lpar"), key.lpar);
    assert_eq!(
        [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])],
        [key.n1, key.n2, key.n3, key.n4],
        "the captured key"
    );
    let node = lookup(&key, &row[0]).expect("a 4.93 row for ss.param.debug.support");
    assert_eq!(node.v1, hex(&row[9]), "v1");
    assert_eq!(node.v2, SecondValue::Given(hex(&row[10])), "v2");
    assert_eq!(node.v1, 0xffff_ffff_ffff_ffff);
    assert_eq!(node.v2, SecondValue::Given(0xffff_ffff_ffff_ffff));
    assert_eq!(
        node.source,
        NodeSource::Lv1NodeCapture {
            capture: DEBUG_SUPPORT_CAPTURE,
            address: hex(&row[6]),
        }
    );
    assert_eq!(node.source.class(), "console");
    assert_eq!(node.source.citation(), DEBUG_SUPPORT_CAPTURE);
    assert_eq!(node.source.section().as_deref(), Some(row[6].as_str()));
}

#[test]
fn a_floor_read_of_the_debug_support_node_is_refused() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(lookup(&debug_support(), fw).is_none(), "{fw}");
    }
}
