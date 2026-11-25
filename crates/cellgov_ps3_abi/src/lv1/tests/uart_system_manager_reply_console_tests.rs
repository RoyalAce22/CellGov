//! The system manager's reply to the 4.93 kernel's request of service id
//! 9, held to the console capture of the kernel's system manager object.

use super::*;

/// The kernel's r2 [OFW-4.93 lv2_kernel a:0x8000000000003000].
pub(super) const KERNEL_TOC: u64 = 0x8000_0000_0034_fbb0;

/// The TOC slot r2 + 3864 that holds the object's address
/// [OFW-4.93 lv2_kernel a:0x8000000000350ac8].
pub(super) const MANAGER_SLOT: u64 = KERNEL_TOC + 3864;

/// The object's address in the image's TOC slot.
pub(super) const MANAGER: u64 = 0x8000_0000_0047_4c38;

fn capture() -> Vec<(String, u64, u64)> {
    capture_rows("reply.tsv")
}

/// The rows of the capture file `file` as (field, address, value). Both
/// reads of each doubleword must agree.
pub(super) fn capture_rows(file: &str) -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(SYSTEM_MANAGER_REPLY_CAPTURE)
        .join("peek/cech20-cex-493")
        .join(file);
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
            assert_eq!(
                (row[2], row[3], row[4]),
                ("8", "2", "yes"),
                "{name}: {line}"
            );
            (row[1].to_string(), hex(row[0]), hex(row[5]))
        })
        .collect()
}

/// The value of `field` in `rows`. The row's address must be `address`.
pub(super) fn word(rows: &[(String, u64, u64)], field: &str, address: u64) -> u64 {
    let (_, at, value) = rows
        .iter()
        .find(|(name, ..)| name == field)
        .unwrap_or_else(|| panic!("no row for {field}"));
    assert_eq!(*at, address, "{field}");
    *value
}

/// The reply's 16 payload bytes, from the capture.
///
/// The kernel keeps payload bytes 1 and 2 at +2032, the doubleword at
/// payload byte 8 at +2040, and the word at payload byte 4 at +2048
/// [OFW-4.93 lv2_kernel a:0x8000000000261a94-0x8000000000261ab8]. lv1
/// sets byte 0 to 2 and byte 3 to 0 [OFW-4.93 lv1 a:0x44BAF4-0x44BB14].
fn captured_payload() -> [u8; 16] {
    let rows = capture();
    assert_eq!(rows.len(), 5);
    assert_eq!(word(&rows, "toc", 0x8000_0000_0000_3000), KERNEL_TOC);
    assert_eq!(word(&rows, "manager", MANAGER_SLOT), MANAGER);
    let at_2032 = word(&rows, "reply2032", MANAGER + 2032).to_be_bytes();
    let at_2040 = word(&rows, "reply2040", MANAGER + 2040).to_be_bytes();
    let at_2048 = word(&rows, "reply2048", MANAGER + 2048).to_be_bytes();
    let mut payload = [0u8; 16];
    payload[0] = 2;
    payload[1..3].copy_from_slice(&at_2032[..2]);
    payload[4..8].copy_from_slice(&at_2048[..4]);
    payload[8..].copy_from_slice(&at_2040);
    payload
}

#[test]
fn the_service_9_reply_payload_is_the_one_the_kernel_kept_on_the_console() {
    let reply = game_os_system_manager_reply("4.93", GAME_OS_FIRST_SYSTEM_MANAGER_SERVICE, 2)
        .expect("a source gives the reply");
    assert_eq!(reply.payload, captured_payload());
}
