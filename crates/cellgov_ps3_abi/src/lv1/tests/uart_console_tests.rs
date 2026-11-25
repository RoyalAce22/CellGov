//! The 4.93 outlet id of the kernel's `configure_virtual_uart_irq` call,
//! held to the console capture of the kernel's virtual UART record.

use super::*;

/// The kernel's r2 [OFW-4.93 lv2_kernel a:0x8000000000003000].
const KERNEL_TOC: u64 = 0x8000_0000_0034_fbb0;

/// The TOC slot r2 - 30904 that holds the record's address
/// [OFW-4.93 lv2_kernel a:0x8000000000017c9c].
const RECORD_SLOT: u64 = KERNEL_TOC - 30904;

/// The record's address in the image's TOC slot
/// [OFW-4.93 lv2_kernel a:0x80000000003482f8].
const RECORD: u64 = 0x8000_0000_0037_dc40;

/// The offset in the record where the kernel stores r4 of the call
/// [OFW-4.93 lv2_kernel a:0x8000000000017b20].
const OUTLET_OFFSET: u64 = 0x08;

/// The `sc` of the kernel's call [OFW-4.93 lv2_kernel a:0x8000000000017b0c].
const CALL_SC: u64 = 0x8000_0000_0001_7b0c;

/// The address and the value of the field `field` in the capture. Both
/// reads of the doubleword must agree.
fn captured(field: &str) -> (u64, u64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(VIRTUAL_UART_IRQ_OUTLET_CAPTURE)
        .join("peek/cech20-cex-493/vuart_outlet.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!(row.len(), 6, "{name}: {field} columns");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("8", "2", "yes"),
        "{name}: {field}"
    );
    let hex = |text: &str| {
        u64::from_str_radix(text.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
    };
    (hex(row[0]), hex(row[5]))
}

#[test]
fn the_kernel_s_call_gets_the_outlet_id_at_8_of_its_captured_record() {
    assert_eq!(captured("toc"), (0x8000_0000_0000_3000, KERNEL_TOC));
    assert_eq!(captured("vuart"), (RECORD_SLOT, RECORD));
    let (address, outlet) = captured("outlet");
    assert_eq!(address, RECORD + OUTLET_OFFSET);
    assert_eq!(GAME_OS_VIRTUAL_UART_IRQ_CALLS.len(), 1);
    assert_eq!(game_os_virtual_uart_outlet("4.93", CALL_SC), Some(outlet));
}

#[test]
fn no_outlet_id_exists_for_another_address_or_firmware() {
    assert_eq!(game_os_virtual_uart_outlet("4.93", CALL_SC + 4), None);
    assert_eq!(game_os_virtual_uart_outlet("4.92", CALL_SC), None);
}
