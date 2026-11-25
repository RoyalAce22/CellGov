//! The capture `lv1_network_vlan_id` holds the table, the immediates and
//! the code of `net_control` request 4 that the reference console's lv1
//! runs.

use super::*;

const CAPTURE: &str = "tests/micro/lv1_network_vlan_id/peek/cech20-cex-493";

/// The r2 that the 4.93 lv1 runs with; see `crate::lv1::dispatch`.
const LV1_TOC: u64 = 0x35_A038;

/// The rows of the capture file `file` under `dir`, after its header
/// `header`, split at tabs.
fn rows_in(dir: &str, file: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{name}: header");
    lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

fn rows(file: &str, header: &str) -> Vec<Vec<String>> {
    rows_in(CAPTURE, file, header)
}

fn hex(cell: &str) -> u64 {
    u64::from_str_radix(cell.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The address and the value of the TOC slot `role` in `words.tsv`. Both
/// reads of the slot must agree.
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

/// The address and the signed immediate of the field `field` in
/// `immediates.tsv`.
fn immediate(field: &str) -> (u64, i64) {
    let rows = rows("immediates.tsv", "address\tfield\tvalue");
    let row = rows
        .iter()
        .find(|row| row.get(1).map(String::as_str) == Some(field))
        .unwrap_or_else(|| panic!("no row for {field}"));
    assert_eq!(row.len(), 3, "{field}: columns");
    let value = row[2]
        .parse()
        .unwrap_or_else(|e| panic!("{field}: {}: {e}", row[2]));
    (hex(&row[0]), value)
}

/// The slot at r2 - 24896 [OFW-4.93 lv1 a:0x244DD8] holds the address of
/// the global object. The capture `lv1_network_control` reads the same
/// address through the slot at r2 - 1848. The slot at r2 - 24768
/// [OFW-4.93 lv1 a:0x244DF8] holds the table's address.
#[test]
fn the_console_lv1_reaches_the_global_object_and_the_table_from_request_4() {
    let (global_slot, global) = word("toc_slot_network_global");
    assert_eq!(global_slot, LV1_TOC - 24896, "the TOC slot of the global");
    let control = rows_in(
        &format!("{NET_CONTROL_CAPTURE}/peek/cech20-cex-493"),
        "net_control.tsv",
        "address\tfield\twidth\treads\tagree\tvalue",
    );
    let global_row = control
        .iter()
        .find(|row| row.get(1).map(String::as_str) == Some("global"))
        .expect("the global row of the request 1 capture");
    assert_eq!(global, hex(&global_row[0]));
    let (table_slot, table) = word("toc_slot_vlan_table");
    assert_eq!(table_slot, LV1_TOC - 24768, "the TOC slot of the table");
    let first = rows("table.tsv", "selector\taddress\tentry\tcase\tkind\tid");
    assert_eq!(hex(&first[0][1]), table);
}

/// The range check passes the selectors up to its immediate, and the two
/// failures load -17 and -6, which the handler zero-extends.
#[test]
fn the_console_lv1_checks_the_selector_and_fails_with_the_modelled_statuses() {
    let (_, max) = immediate("selector_max");
    assert_eq!(u64::try_from(max).ok(), Some(NET_CONTROL_VLAN_SELECTOR_MAX));
    let (_, illegal) = immediate("illegal_status");
    assert_eq!(illegal, LV1_ILLEGAL_PARAMETER_VALUE);
    assert_eq!(NET_CONTROL_VLAN_SELECTOR_R3, zero_extended(illegal));
    let (_, no_entry) = immediate("no_entry_status");
    assert_eq!(NET_CONTROL_VLAN_NONE_R3, zero_extended(no_entry));
}

#[test]
fn each_selector_of_the_console_table_gives_the_modelled_case() {
    let rows = rows("table.tsv", "selector\taddress\tentry\tcase\tkind\tid");
    let (illegal_case, _) = immediate("illegal_status");
    assert_eq!(rows.len() as u64, NET_CONTROL_VLAN_SELECTOR_MAX + 1);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.len(), 6, "{row:?}");
        let selector = index as u64;
        assert_eq!(row[0].parse::<u64>().ok(), Some(selector), "{row:?}");
        let entry = hex(&row[2]) as u32 as i32;
        assert_eq!(
            hex(&row[3]),
            hex(&rows[0][1]).wrapping_add_signed(i64::from(entry)),
            "{row:?}: the case is the table plus the entry"
        );
        let captured = match row[4].as_str() {
            "fixed" => {
                VlanSelector::Fixed(row[5].parse().unwrap_or_else(|e| panic!("{row:?}: {e}")))
            }
            "flagged" => VlanSelector::Flagged,
            "illegal" => {
                assert_eq!(hex(&row[3]), illegal_case, "{row:?}");
                VlanSelector::Illegal
            }
            kind => panic!("{row:?}: kind {kind}"),
        };
        assert_eq!(net_control_vlan_selector(selector), captured, "{row:?}");
    }
    assert_eq!(
        net_control_vlan_selector(NET_CONTROL_VLAN_SELECTOR_MAX + 1),
        VlanSelector::Illegal
    );
}

#[test]
fn the_console_lv1_runs_the_shipped_code_of_request_4() {
    let rows = rows("code.tsv", "first\tlast\tholds\tdoublewords\tagree\tstock");
    assert_eq!(rows.len(), 7);
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
    let range = |holds: &str| {
        let row = rows
            .iter()
            .find(|row| row[2] == holds)
            .unwrap_or_else(|| panic!("no range {holds}"));
        hex(&row[0])..=hex(&row[1])
    };
    let table = rows_in(
        CAPTURE,
        "table.tsv",
        "selector\taddress\tentry\tcase\tkind\tid",
    );
    assert_eq!(table.len(), 21);
    for row in &table {
        let holds = match row[4].as_str() {
            "illegal" => "switch_exit_and_dispatch",
            _ => "vlan_id_case",
        };
        assert!(
            range("vlan_id_case").contains(&hex(&row[1])),
            "{row:?}: the entry lies in the shipped range"
        );
        assert!(
            range(holds).contains(&hex(&row[3])),
            "{row:?}: the case lies in the shipped range"
        );
    }
}
