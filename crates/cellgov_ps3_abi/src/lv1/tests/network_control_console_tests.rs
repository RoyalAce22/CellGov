//! The network device's checks in `net_control`, held to the console
//! capture of its flag byte and state word in lv1.

use super::*;

/// The lv1 global that holds the network device's global object, from
/// the TOC slot r2 - 1848 [OFW-4.93 lv1 a:0x359900-0x359904].
const NETWORK_GLOBAL: u64 = 0x3682b0;

/// The offset in the global object of the device's object
/// [OFW-4.93 lv1 a:0x307E28].
const DEVICE_OFFSET: u64 = 16;

/// The offset in the device's object of the flag byte
/// [OFW-4.93 lv1 a:0x245108].
const FLAG_OFFSET: u64 = 0;

/// The offset in the device's object of the state word
/// [OFW-4.93 lv1 a:0x307E2C].
const STATE_OFFSET: u64 = 8;

/// The state word that the handler passes
/// [OFW-4.93 lv1 a:0x307E30-0x307E34].
const OPEN_STATE: u64 = 2;

/// The rows of the capture, as (field, address, width, value). Both
/// reads of each word must agree.
fn rows() -> Vec<(String, u64, u32, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(NET_CONTROL_CAPTURE)
        .join("peek/cech20-cex-493/net_control.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {text}: {e}"))
    };
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 6, "{name}: {line}");
            assert_eq!((row[3], row[4]), ("2", "yes"), "{name}: {line}");
            let width = row[2]
                .parse()
                .unwrap_or_else(|e| panic!("{name}: {line}: {e}"));
            (row[1].to_string(), hex(row[0]), width, hex(row[5]))
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

#[test]
fn the_capture_reads_each_word_where_the_word_before_it_points() {
    let rows = rows();
    assert_eq!(rows.len(), 4);
    let (global_at, global_width, global) = captured(&rows, "global");
    assert_eq!((global_at, global_width), (NETWORK_GLOBAL, 8));
    let (device_at, device_width, device) = captured(&rows, "device");
    assert_eq!((device_at, device_width), (global + DEVICE_OFFSET, 8));
    let (flag_at, flag_width, _) = captured(&rows, "device_flag");
    assert_eq!((flag_at, flag_width), (device + FLAG_OFFSET, 1));
    let (state_at, state_width, _) = captured(&rows, "device_state");
    assert_eq!((state_at, state_width), (device + STATE_OFFSET, 4));
}

#[test]
fn the_capture_reads_no_word_of_the_mac_address() {
    let rows = rows();
    let (_, _, device) = captured(&rows, "device");
    let mac = device + 16..device + 24;
    assert_eq!(rows.len(), 4);
    for (field, address, width, _) in &rows {
        let end = address + u64::from(*width);
        assert!(
            end <= mac.start || *address >= mac.end,
            "{field} reads the MAC address"
        );
    }
}

#[test]
fn the_open_network_device_has_state_word_2_and_a_flag_byte_other_than_0() {
    let rows = rows();
    let (_, _, state) = captured(&rows, "device_state");
    assert_eq!(state, OPEN_STATE);
    let (_, _, flag) = captured(&rows, "device_flag");
    assert_ne!(flag, 0);
}
