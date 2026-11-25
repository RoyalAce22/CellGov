use super::*;
use crate::lv1::partition::LPAR_ID_PME;

fn hw_config() -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"sys", 0),
        n2: field(b"hw", 0),
        n3: field(b"config", 0),
        n4: 0,
    }
}

/// The 4.93 kernel passes the texts `sys`, `hw`, `config` and an empty
/// text, each with index 0, to its key wrapper
/// [OFW-4.93 lv2_kernel a:0x80000000000a1f3c-0x80000000000a1f68]
/// [OFW-4.93 lv2_kernel a:0x800000000034ac40-0x800000000034ac5c].
#[test]
fn the_served_hw_config_key_is_the_words_the_4_93_kernel_reads() {
    let served: Vec<RepositoryKey> = NODES
        .iter()
        .map(|node| node.key)
        .filter(|key| key.path() == "sys.hw.config")
        .collect();
    assert_eq!(served.len(), 1);
    let key = served[0];
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (
            1,
            0x0000_0000_7379_7300,
            0x6877_0000_0000_0000,
            0x636f_6e66_6967_0000,
            0
        )
    );
}

/// The one data row of the capture's `node.tsv`, split into its cells.
fn captured_node() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(HW_CONFIG_CAPTURE)
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

#[test]
fn the_4_93_hw_config_node_answers_the_values_of_the_lv1_capture_and_cites_it() {
    let row = captured_node();
    let key = hw_config();
    assert_eq!(row[0], "4.93", "the captured firmware");
    assert_eq!(row[1].parse::<u64>().expect("a decimal lpar"), key.lpar);
    assert_eq!(
        [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])],
        [key.n1, key.n2, key.n3, key.n4],
        "the captured key"
    );
    let node = lookup(&key, &row[0]).expect("a 4.93 row for sys.hw.config");
    assert_eq!(node.v1, hex(&row[9]), "v1");
    assert_eq!(node.v2, SecondValue::Given(hex(&row[10])), "v2");
    assert_eq!(
        node.source,
        NodeSource::Lv1NodeCapture {
            capture: HW_CONFIG_CAPTURE,
            address: hex(&row[6]),
        }
    );
    assert_eq!(node.source.class(), "console");
    assert_eq!(node.source.citation(), HW_CONFIG_CAPTURE);
    assert_eq!(node.source.section().as_deref(), Some(row[6].as_str()));
}

#[test]
fn a_hw_config_read_at_a_firmware_other_than_4_93_finds_no_node() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(lookup(&hw_config(), fw).is_none(), "{fw}");
    }
}
