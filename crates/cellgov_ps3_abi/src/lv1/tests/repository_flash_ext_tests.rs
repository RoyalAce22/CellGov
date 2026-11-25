use super::*;
use crate::lv1::partition::LPAR_ID_PME;

fn flash(third: &[u8]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"sys", 0),
        n2: field(b"flash", 0),
        n3: field(third, 0),
        n4: 0,
    }
}

/// The 4.93 kernel builds these words in r3 to r7 before its `sc`
/// [OFW-4.93 lv2_kernel a:0x8000000000050ddc-0x8000000000050e10].
#[test]
fn the_served_flash_ext_key_is_the_words_the_4_93_kernel_builds() {
    let served: Vec<RepositoryKey> = NODES
        .iter()
        .map(|node| node.key)
        .filter(|key| key.path() == "sys.flash.ext")
        .collect();
    assert_eq!(served.len(), 1);
    let key = served[0];
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (
            1,
            0x0000_0000_7379_7300,
            0x666c_6173_6800_0000,
            0x6578_7400_0000_0000,
            0
        )
    );
}

/// The one data row of the capture's `node.tsv`, split into its cells.
fn captured_node() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(FLASH_EXT_CAPTURE)
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
fn the_4_93_flash_ext_node_answers_the_values_of_the_lv1_capture_and_cites_it() {
    let row = captured_node();
    let key = flash(b"ext");
    assert_eq!(row[0], "4.93", "the captured firmware");
    assert_eq!(row[1].parse::<u64>().expect("a decimal lpar"), key.lpar);
    assert_eq!(
        [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])],
        [key.n1, key.n2, key.n3, key.n4],
        "the captured key"
    );
    let node = lookup(&key, &row[0]).expect("a 4.93 row for sys.flash.ext");
    assert_eq!(node.v1, hex(&row[9]), "v1");
    assert_eq!(node.v2, SecondValue::Given(hex(&row[10])), "v2");
    assert_eq!(
        node.source,
        NodeSource::Lv1NodeCapture {
            capture: FLASH_EXT_CAPTURE,
            address: hex(&row[6]),
        }
    );
    assert_eq!(node.source.class(), "console");
    assert_eq!(node.source.citation(), FLASH_EXT_CAPTURE);
    assert_eq!(node.source.section().as_deref(), Some(row[6].as_str()));
}

/// The kernel's `sys.flash.ext` reader returns 1 only when bit 1 of the
/// value is clear
/// [OFW-4.93 lv2_kernel a:0x8000000000050e30-0x8000000000050e38]. Only a
/// return of 1 leads to the `sys.flash.boot` read
/// [OFW-4.93 lv2_kernel a:0x800000000004f23c-0x800000000004f24c].
#[test]
fn bit_1_of_the_4_93_flash_ext_value_is_set_and_no_flash_boot_node_is_served() {
    let ext = lookup(&flash(b"ext"), "4.93").expect("a 4.93 row for sys.flash.ext");
    assert_eq!(ext.v1 & 2, 2);
    assert!(lookup(&flash(b"boot"), "4.93").is_none());
}

#[test]
fn a_floor_read_of_the_flash_ext_node_is_refused() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(lookup(&flash(b"ext"), fw).is_none(), "{fw}");
    }
}
