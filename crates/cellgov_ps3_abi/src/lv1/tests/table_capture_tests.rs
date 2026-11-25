//! The live 4.93 table map matches the committed console capture row for
//! row.

use super::{
    table_entry, table_site, TableEntry, HYPERCALL_TABLE_ENTRIES, LIVE_TABLE_CAPTURE,
    LIVE_TABLE_FIRMWARE,
};

/// The capture's table file, below [`LIVE_TABLE_CAPTURE`].
const TABLE_FILE: &str = "peek/cech20-cex-493/hypercall_table.tsv";

/// Each row of the capture's table file: the number, the doubleword and
/// the kind.
fn capture_rows() -> Vec<(u64, u64, String)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LIVE_TABLE_CAPTURE)
        .join(TABLE_FILE);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some("number\tentry\tkind"), "{name}: header");
    lines
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            assert_eq!(cells.len(), 3, "{name}: row {line:?}");
            let number = cells[0].parse().unwrap_or_else(|e| panic!("{name}: {e}"));
            let word = u64::from_str_radix(cells[1].trim_start_matches("0x"), 16)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            (number, word, cells[2].to_string())
        })
        .collect()
}

#[test]
fn every_capture_row_matches_the_live_table_map() {
    let invalid_function = table_site(LIVE_TABLE_FIRMWARE)
        .map(|site| u64::from(site.invalid_function))
        .expect("the 4.93 site row");
    let rows = capture_rows();
    assert_eq!(rows.len() as u64, HYPERCALL_TABLE_ENTRIES);
    for (index, (number, word, kind)) in (0u64..).zip(&rows) {
        assert_eq!(*number, index, "the rows run in number order");
        let expected = match kind.as_str() {
            "handler" => {
                assert_ne!(*word, invalid_function, "number {number}");
                TableEntry::Handler(u32::try_from(*word).expect("a 32-bit code address"))
            }
            "invalid" => {
                assert_eq!(*word, invalid_function, "number {number}");
                TableEntry::Invalid
            }
            other => panic!("number {number}: kind {other}"),
        };
        assert_eq!(
            table_entry(LIVE_TABLE_FIRMWARE, *number),
            Some(expected),
            "number {number}"
        );
    }
}
