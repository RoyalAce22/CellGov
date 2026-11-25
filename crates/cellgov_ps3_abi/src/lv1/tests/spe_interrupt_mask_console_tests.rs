//! The capture `lv1_spe_interrupt_mask_table` holds the table and the
//! code of `set_spe_interrupt_mask` that the reference console's lv1 runs.

use super::*;

const CAPTURE: &str = "tests/micro/lv1_spe_interrupt_mask_table/peek/cech20-cex-493";

/// The r2 that the 4.93 lv1 runs with; see `crate::lv1::dispatch`.
const LV1_TOC: u64 = 0x35_A038;

/// The rows of the capture file `file`, after its header `header`, split
/// at tabs.
fn rows(file: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CAPTURE)
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{name}: header");
    lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

fn hex(cell: &str) -> u64 {
    u64::from_str_radix(cell.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The address and the value of the word `role` in `words.tsv`. Both reads
/// of the word must agree.
fn word(role: &str) -> (u64, u64) {
    let rows = rows("words.tsv", "address\trole\treads\tagree\tvalue");
    let row = rows
        .iter()
        .find(|row| row.get(1).map(String::as_str) == Some(role))
        .unwrap_or_else(|| panic!("no row for {role}"));
    assert_eq!(row.len(), 5, "{role}: columns");
    assert_eq!((row[2].as_str(), row[3].as_str()), ("2", "yes"), "{role}");
    (hex(&row[0]), hex(&row[4]))
}

/// The TOC slot at r2 - 5016 [OFW-4.93 lv1 a:0x358CA0] points to the
/// table, and the table's three doublewords are the masks of classes 0 to
/// 2.
#[test]
fn the_console_lv1_keeps_the_class_mask_table_that_cellgov_applies() {
    let (slot, table) = word("toc_slot_class_masks");
    assert_eq!(slot, LV1_TOC - 5016, "the TOC slot of the table");
    for (class, kept) in SPE_INTERRUPT_CLASS_MASKS.into_iter().enumerate() {
        let (address, value) = word(&format!("class_mask_{class}"));
        assert_eq!(address, table + 8 * class as u64, "class {class}");
        assert_eq!(value, kept, "class {class}");
    }
}

/// The slot at r2 - 4616 holds the descriptor of the method that ANDs the
/// mask [OFW-4.93 lv1 a:0x2FA23C], and the class of the interrupt object
/// leads at +48 to the method that stores it [OFW-4.93 lv1 a:0x3198F4].
#[test]
fn the_console_lv1_reaches_the_and_and_the_store_from_the_call() {
    let (entry_slot, entry_descriptor) = word("toc_slot_mask_entry");
    assert_eq!(entry_slot, LV1_TOC - 4616, "the TOC slot of the method");
    assert_eq!(word("mask_entry_descriptor"), (entry_descriptor, 0x2F_A23C));
    let (class_slot, class) = word("toc_slot_interrupt_object_class");
    assert_eq!(class_slot, LV1_TOC - 344, "the TOC slot of the class");
    let (entry, store_descriptor) = word("interrupt_object_class_entry_48");
    assert_eq!(entry, class + 48);
    assert_eq!(
        word("mask_method_descriptor"),
        (store_descriptor, 0x31_98F4)
    );
}

/// Each cited code range of the call is the shipped image's code on the
/// console, the handler of the live table entry among them.
#[test]
fn the_console_lv1_runs_the_shipped_code_of_the_call() {
    let rows = rows("code.tsv", "first\tlast\tholds\tdoublewords\tagree\tstock");
    assert_eq!(rows.len(), 11);
    for row in &rows {
        assert_eq!(row.len(), 6, "{row:?}");
        let (first, last) = (hex(&row[0]), hex(&row[1]));
        assert_eq!(
            row[3].parse::<u64>().ok(),
            Some((last + 1 - first) / 8),
            "{row:?}"
        );
        assert_eq!(
            (row[4].as_str(), row[5].as_str()),
            ("yes", "identical"),
            "{row:?}"
        );
    }
    let handler = u64::from(
        spe_interrupt_mask_handler("4.93")
            .expect("the 4.93 handler")
            .handler,
    );
    let handler_row = rows
        .iter()
        .find(|row| row[2] == "set_spe_interrupt_mask_handler")
        .expect("the handler's range");
    assert_eq!(hex(&handler_row[0]), handler);
}
