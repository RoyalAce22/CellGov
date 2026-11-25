//! The capture `lv1_network_vlan_fields` holds the two fields of the
//! network device's global object that `net_control` request 4 reads on
//! the reference console.

use super::*;

const PROFILE: &str = "peek/cech20-cex-493";

/// The lv1 global that holds the network device's global object, from
/// the TOC slot r2 - 24896 [OFW-4.93 lv1 a:0x353EF8-0x353EFC].
const NETWORK_GLOBAL: u64 = 0x3682b0;

fn hex(cell: &str) -> u64 {
    u64::from_str_radix(cell.trim_start_matches("0x"), 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The rows of the capture file `file` of the capture `capture`, after
/// its header `header`, split at tabs.
fn rows_in(capture: &str, file: &str, header: &str) -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(capture)
        .join(PROFILE)
        .join(file);
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(header), "{name}: header");
    lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

/// The rows of `fields.tsv`, as (field, address, width, value). Both
/// reads of each word must agree.
fn fields() -> Vec<(String, u64, u32, u64)> {
    rows_in(
        NET_CONTROL_VLAN_FIELDS_CAPTURE,
        "fields.tsv",
        "address\tfield\twidth\treads\tagree\tvalue",
    )
    .into_iter()
    .map(|row| {
        assert_eq!(row.len(), 6, "{row:?}");
        assert_eq!((row[3].as_str(), row[4].as_str()), ("2", "yes"), "{row:?}");
        let width = row[2].parse().unwrap_or_else(|e| panic!("{row:?}: {e}"));
        (row[1].clone(), hex(&row[0]), width, hex(&row[5]))
    })
    .collect()
}

/// The address, the width and the value of the field `field`.
fn captured(rows: &[(String, u64, u32, u64)], field: &str) -> (u64, u32, u64) {
    rows.iter()
        .find(|(name, _, _, _)| name == field)
        .map(|&(_, address, width, value)| (address, width, value))
        .unwrap_or_else(|| panic!("no row for {field}"))
}

/// The offset in the global object that the instruction of the field
/// `field` in the capture `lv1_network_vlan_id` reads.
fn code_offset(field: &str) -> u64 {
    let rows = rows_in(
        "tests/micro/lv1_network_vlan_id",
        "immediates.tsv",
        "address\tfield\tvalue",
    );
    let row = rows
        .iter()
        .find(|row| row.get(1).map(String::as_str) == Some(field))
        .unwrap_or_else(|| panic!("no row for {field}"));
    row[2].parse().unwrap_or_else(|e| panic!("{row:?}: {e}"))
}

#[test]
fn the_capture_reads_each_field_at_its_offset_in_the_global_object() {
    assert_eq!(
        NET_CONTROL_VLAN_FLAG_OFFSET,
        code_offset("flag_offset"),
        "flag offset"
    );
    assert_eq!(
        NET_CONTROL_VLAN_CAPABILITY_OFFSET,
        code_offset("capability_offset"),
        "capability offset"
    );
    let rows = fields();
    assert_eq!(rows.len(), 3);
    let (global_at, global_width, global) = captured(&rows, "global");
    assert_eq!((global_at, global_width), (NETWORK_GLOBAL, 8));
    let control = rows_in(
        NET_CONTROL_CAPTURE,
        "net_control.tsv",
        "address\tfield\twidth\treads\tagree\tvalue",
    );
    let control_global = control
        .iter()
        .find(|row| row.get(1).map(String::as_str) == Some("global"))
        .expect("the global row of the request 1 capture");
    assert_eq!(global, hex(&control_global[5]));
    let (flag_at, flag_width, _) = captured(&rows, "vlan_flag");
    assert_eq!(
        (flag_at, flag_width),
        (global + NET_CONTROL_VLAN_FLAG_OFFSET, 1)
    );
    let (capability_at, capability_width, _) = captured(&rows, "vlan_capability");
    assert_eq!(
        (capability_at, capability_width),
        (global + NET_CONTROL_VLAN_CAPABILITY_OFFSET, 8)
    );
}

#[test]
fn the_console_vlan_fields_are_the_captured_values() {
    let rows = fields();
    let (_, _, capability) = captured(&rows, "vlan_capability");
    let (_, _, flag) = captured(&rows, "vlan_flag");
    assert_eq!(
        CONSOLE_VLAN_FIELDS,
        VlanFields {
            capability,
            flag: u8::try_from(flag).expect("a byte"),
        }
    );
}

/// The captured doubleword has bit 63 set, so the console's lv1 fails
/// each selector of the table and each selector above it with -6.
#[test]
fn the_console_lv1_fails_every_vlan_selector_with_minus_6() {
    let rows = fields();
    let (_, _, capability) = captured(&rows, "vlan_capability");
    let (_, _, flag) = captured(&rows, "vlan_flag");
    let fields = VlanFields {
        capability,
        flag: u8::try_from(flag).expect("a byte"),
    };
    let selectors: Vec<u64> = (0..=NET_CONTROL_VLAN_SELECTOR_MAX + 1)
        .chain([u64::MAX])
        .collect();
    assert_eq!(selectors.len(), 23);
    for selector in selectors {
        assert_eq!(
            net_control_vlan_id(selector, fields),
            VlanAnswer::Failed(NET_CONTROL_VLAN_NONE_R3),
            "selector {selector:#x}"
        );
    }
}
