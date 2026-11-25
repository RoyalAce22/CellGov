use super::*;
use crate::lv1::partition::LPAR_ID_PME;

fn update_status() -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"ss", 0),
        n2: field(b"param", 0),
        n3: field(b"update", 0),
        n4: field(b"status", 0),
    }
}

/// The data rows of the capture file `name`, after a header equal to
/// `header`. Each row has one cell for each header column.
fn capture_rows(name: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LIST_CAPTURE)
        .join(name);
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{shown}: header");
    let columns = header.split('\t').count();
    lines
        .map(|line| {
            let row: Vec<String> = line.split('\t').map(str::to_string).collect();
            assert_eq!(row.len(), columns, "{shown}: {line:?}");
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

/// The one row of the capture file `name` whose four key words are those
/// of `ss.param.update.status`.
fn update_status_row(name: &str, header: &str) -> Vec<String> {
    let key = update_status();
    let rows: Vec<Vec<String>> = capture_rows(name, header)
        .into_iter()
        .filter(|row| {
            [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])]
                == [key.n1, key.n2, key.n3, key.n4]
        })
        .collect();
    assert_eq!(rows.len(), 1, "{name}: one ss.param.update.status row");
    rows.into_iter().next().expect("one row")
}

const NODE_HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";

const LIST_HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\tnode_address\tsplit_order_key\treads\tagree";

/// The 4.93 kernel's reader builds the words `ss`, `param`, `update` and
/// `status` with immediates
/// [OFW-4.93 lv2_kernel a:0x8000000000050ab4-0x8000000000050b08].
#[test]
fn the_served_update_status_key_is_the_words_the_4_93_kernel_reads() {
    let served: Vec<RepositoryKey> = NODES
        .iter()
        .map(|node| node.key)
        .filter(|key| key.path() == "ss.param.update.status")
        .collect();
    assert_eq!(served.len(), 1);
    let key = served[0];
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (
            1,
            0x0000_0000_7373_0000,
            0x7061_7261_6d00_0000,
            0x7570_6461_7465_0000,
            0x7374_6174_7573_0000
        )
    );
}

#[test]
fn the_4_93_update_status_node_answers_the_values_of_the_lv1_capture_and_cites_it() {
    let row = update_status_row("node.tsv", NODE_HEADER);
    let key = update_status();
    assert_eq!(row[0], "4.93", "the captured firmware");
    assert_eq!(row[1].parse::<u64>().expect("a decimal lpar"), key.lpar);
    assert_eq!(
        (row[7].as_str(), row[8].as_str()),
        ("2", "yes"),
        "two reads that agree"
    );
    let node = lookup(&key, &row[0]).expect("a 4.93 row for ss.param.update.status");
    assert_eq!(node.v1, hex(&row[9]), "v1");
    assert_eq!(node.v2, SecondValue::Given(hex(&row[10])), "v2");
    assert_eq!(
        node.source,
        NodeSource::Lv1NodeCapture {
            capture: LIST_CAPTURE,
            address: hex(&row[6]),
        }
    );
    assert_eq!(node.source.class(), "console");
    assert_eq!(node.source.citation(), LIST_CAPTURE);
    assert_eq!(node.source.section().as_deref(), Some(row[6].as_str()));
}

/// lv1 keeps v1 at +104 of a node. Its walk returns the node plus 64
/// [OFW-4.93 lv1 a:0x2E3A00], the read adds 32 to that
/// [OFW-4.93 lv1 a:0x2E3D48], and loads v1 at +8 of the sum
/// [OFW-4.93 lv1 a:0x2E3E2C-0x2E3E40].
#[test]
fn the_update_status_values_are_at_offset_104_of_the_node_that_the_list_walk_found() {
    let rows = capture_rows("list.tsv", LIST_HEADER);
    assert_eq!(rows.len(), 384, "the nodes of the whole list");
    for row in &rows {
        assert_eq!(
            (
                row[0].as_str(),
                row[1].as_str(),
                row[8].as_str(),
                row[9].as_str()
            ),
            ("4.93", "1", "2", "yes"),
            "{row:?}"
        );
    }
    let listed = update_status_row("list.tsv", LIST_HEADER);
    let served = update_status_row("node.tsv", NODE_HEADER);
    assert_eq!(hex(&listed[6]) + 104, hex(&served[6]), "the v1 address");
    let node = lookup(&update_status(), &listed[0]).expect("the served node");
    assert_eq!(
        node.source,
        NodeSource::Lv1NodeCapture {
            capture: LIST_CAPTURE,
            address: hex(&listed[6]) + 104,
        },
        "the served v1 address"
    );
}

#[test]
fn an_update_status_read_at_a_firmware_other_than_4_93_finds_no_node() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(lookup(&update_status(), fw).is_none(), "{fw}");
    }
}
