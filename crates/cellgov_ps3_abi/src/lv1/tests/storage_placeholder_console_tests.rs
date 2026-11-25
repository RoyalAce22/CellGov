use std::collections::BTreeSet;
use std::path::Path;

use super::*;

/// The console capture of the `bus.4` nodes that [`REGISTRATION_CAPTURE`]
/// holds no value for.
pub(super) const PLACEHOLDER_CAPTURE: &str =
    "tests/micro/lv1_storage_placeholder_nodes/peek/cech20-cex-493";

const NODE_HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";

pub(super) fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The key, v1 and v2 of each row of the node file `name` of the capture
/// `dir`, after a check that both reads of each node agree.
pub(super) fn nodes(dir: &str, name: &str) -> Vec<(RepositoryKey, u64, u64)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name);
    let shown = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{shown}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(NODE_HEADER), "{shown}: header");
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 11, "{shown}: {line:?}");
            assert_eq!((row[0], row[1]), ("4.93", "1"), "{shown}: {line:?}");
            assert_eq!((row[7], row[8]), ("2", "yes"), "{shown}: {line:?}");
            let key = RepositoryKey {
                lpar: LPAR_ID_PME,
                n1: hex(row[2]),
                n2: hex(row[3]),
                n3: hex(row[4]),
                n4: hex(row[5]),
            };
            (key, hex(row[9]), hex(row[10]))
        })
        .collect()
}

#[test]
fn each_uncaptured_bus_node_holds_the_value_the_model_gives_after_five_registrations() {
    let reg = registration("4.93").expect("the 4.93 registration");
    let captured = nodes(PLACEHOLDER_CAPTURE, "node.tsv");
    assert_eq!(captured.len(), 154);
    for (key, v1, v2) in captured {
        assert_eq!(reg.node(0..5, &key), Some((v1, v2)), "{}", key.path());
    }
}

#[test]
fn the_two_captures_hold_each_placeholder_key_once() {
    let mut keys = BTreeSet::new();
    let captured = nodes(PLACEHOLDER_CAPTURE, "node.tsv")
        .into_iter()
        .chain(nodes(REGISTRATION_CAPTURE, "node.tsv"))
        .chain(nodes(REGISTRATION_CAPTURE, "region.tsv"));
    for (key, _, _) in captured {
        assert!(keys.insert(key), "{} is captured twice", key.path());
    }
    let placeholders: BTreeSet<RepositoryKey> =
        placeholder_nodes().iter().map(|node| node.key).collect();
    assert_eq!(keys, placeholders);
}
