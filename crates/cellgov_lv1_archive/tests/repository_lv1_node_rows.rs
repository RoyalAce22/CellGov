//! The repository nodes served from an lv1 node capture, against the
//! committed capture itself.
//!
//! The test reads only committed files.

use std::path::{Path, PathBuf};

use cellgov_ps3_abi::lv1::repository::{NodeSource, SecondValue, DEBUG_SUPPORT_CAPTURE, NODES};

fn workspace_file(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// The data rows of the capture file `name`, after a header equal to
/// `header`. Each row has exactly one cell for each header column.
fn capture_rows(capture: &str, name: &str, header: &str) -> Vec<Vec<String>> {
    let path = workspace_file(capture).join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{}", path.display());
    let columns = header.split('\t').count();
    lines
        .map(|line| {
            let row: Vec<String> = line.split('\t').map(str::to_string).collect();
            assert_eq!(row.len(), columns, "{}: {line:?}", path.display());
            row
        })
        .collect()
}

/// The node rows of `capture`: the rows of its `node.tsv`, and then the
/// rows of its `unread_node.tsv` when the capture has one.
///
/// A capture's files stay as the capture wrote them. So a node that a
/// later boot came to read can keep its row in `unread_node.tsv`.
fn node_rows(capture: &str) -> Vec<Vec<String>> {
    const HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";
    let mut rows = capture_rows(capture, "node.tsv", HEADER);
    if workspace_file(capture).join("unread_node.tsv").is_file() {
        rows.extend(capture_rows(capture, "unread_node.tsv", HEADER));
    }
    rows
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

#[test]
fn every_lv1_node_row_is_the_node_its_capture_read_twice_alike() {
    let rows: Vec<_> = NODES
        .iter()
        .filter_map(|node| match node.source {
            NodeSource::Lv1NodeCapture { capture, address } => Some((node, capture, address)),
            NodeSource::PsdevwikiDump { .. } | NodeSource::ConsoleCapture { .. } => None,
        })
        .collect();
    assert_eq!(rows.len(), 37);
    for (node, capture, address) in rows {
        let captured = node_rows(capture);
        let key = [node.key.n1, node.key.n2, node.key.n3, node.key.n4];
        let matching: Vec<&Vec<String>> = captured
            .iter()
            .filter(|row| [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])] == key)
            .collect();
        assert_eq!(matching.len(), 1, "{capture}: one row for {}", node.key);
        let row = matching[0];
        assert_eq!(node.firmware, [row[0].as_str()], "{}", node.key);
        assert_eq!(
            row[1].parse::<u64>().expect("a decimal lpar"),
            node.key.lpar
        );
        assert_eq!(
            [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])],
            [node.key.n1, node.key.n2, node.key.n3, node.key.n4],
            "{}",
            node.key
        );
        assert_eq!(hex(&row[6]), address, "{}", node.key);
        assert_eq!((row[7].as_str(), row[8].as_str()), ("2", "yes"));
        assert_eq!(hex(&row[9]), node.v1, "{}", node.key);
        assert_eq!(SecondValue::Given(hex(&row[10])), node.v2, "{}", node.key);
    }
}

/// The kernel stores the read's two values in two doublewords
/// [OFW-4.93 lv2_kernel a:0x8000000000050a70-0x8000000000050a80].
#[test]
fn the_kernel_copy_of_debug_support_holds_the_lv1_node_values() {
    let cited: Vec<_> = NODES
        .iter()
        .filter(|node| node.source.citation() == DEBUG_SUPPORT_CAPTURE)
        .collect();
    assert_eq!(
        cited.len(),
        1,
        "the ss.param.debug.support row that cites {DEBUG_SUPPORT_CAPTURE}"
    );
    let node = cited[0];
    assert_eq!(
        node.key.path(),
        "ss.param.debug.support",
        "the row that cites {DEBUG_SUPPORT_CAPTURE}"
    );
    let NodeSource::Lv1NodeCapture { capture, .. } = node.source else {
        panic!(
            "{} cites {DEBUG_SUPPORT_CAPTURE} as no lv1 node capture",
            node.key
        );
    };
    let SecondValue::Given(v2) = node.v2 else {
        panic!("{} has no second value", node.key);
    };
    let rows = capture_rows(capture, "kernel.tsv", "address\treads\tagree\tdoubleword");
    let words: Vec<(u64, &str, &str, u64)> = rows
        .iter()
        .map(|row| (hex(&row[0]), row[1].as_str(), row[2].as_str(), hex(&row[3])))
        .collect();
    assert_eq!(
        words,
        [
            (0x8000_0000_0035_4d98, "2", "yes", node.v1),
            (0x8000_0000_0035_4da0, "2", "yes", v2),
        ]
    );
}
