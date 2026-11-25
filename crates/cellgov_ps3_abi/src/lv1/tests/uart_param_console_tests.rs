//! The 4.93 buffer sizes of the kernel's virtual UART ports, held to the
//! console capture of the kernel's port records.

use super::*;

/// The kernel's r2 [OFW-4.93 lv2_kernel a:0x8000000000003000].
const KERNEL_TOC: u64 = 0x8000_0000_0034_fbb0;

/// The TOC slot r2 - 30880 that holds the virtual UART record's address
/// [OFW-4.93 lv2_kernel a:0x8000000000348310].
const RECORD_SLOT: u64 = KERNEL_TOC - 30880;

/// The virtual UART record's address in the image's TOC slot
/// [OFW-4.93 lv2_kernel a:0x8000000000348310].
const RECORD: u64 = 0x8000_0000_0037_dc40;

/// The offsets of slot 0's port number and port record in the virtual
/// UART record. Each slot takes 16 bytes
/// [OFW-4.93 lv2_kernel a:0x8000000000017968-0x8000000000017978].
const SLOT_PORT: u64 = 72;
const SLOT_RECORD: u64 = 80;
const SLOT_BYTES: u64 = 16;

/// The offset of the doubleword whose high word is the count of
/// registered ports [OFW-4.93 lv2_kernel a:0x800000000001796c].
const COUNT: u64 = 136;

/// The port record's offsets of the port number, the transmit buffer
/// size and the receive buffer size
/// [OFW-4.93 lv2_kernel a:0x80000000000169d0]
/// [OFW-4.93 lv2_kernel a:0x800000000001775c]
/// [OFW-4.93 lv2_kernel a:0x8000000000017740].
const PORT: u64 = 0;
const TX: u64 = 384;
const RX: u64 = 392;

/// The initial value of an empty slot's port number
/// [OFW-4.93 lv2_kernel a:0x8000000000017c5c-0x8000000000017c6c].
const NO_PORT: u64 = 256;

/// The capture's rows as (field, address, value). Both reads of each
/// doubleword must agree.
fn capture() -> Vec<(String, u64, u64)> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(VIRTUAL_UART_PORT_CAPTURE)
        .join("peek/cech20-cex-493/vuart_ports.tsv");
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
fn word(rows: &[(String, u64, u64)], field: &str, address: u64) -> u64 {
    let (_, at, value) = rows
        .iter()
        .find(|(name, ..)| name == field)
        .unwrap_or_else(|| panic!("no row for {field}"));
    assert_eq!(*at, address, "{field}");
    *value
}

#[test]
fn the_capture_reads_the_stock_toc_slot_and_three_registered_ports() {
    let rows = capture();
    assert_eq!(rows.len(), 20);
    assert_eq!(word(&rows, "toc", 0x8000_0000_0000_3000), KERNEL_TOC);
    assert_eq!(word(&rows, "vuart", RECORD_SLOT), RECORD);
    assert_eq!(word(&rows, "count", RECORD + COUNT) >> 32, 3);
    let slot3 = RECORD + 3 * SLOT_BYTES;
    assert_eq!(word(&rows, "slot3_port", slot3 + SLOT_PORT), NO_PORT);
}

#[test]
fn each_registered_port_answers_the_sizes_in_its_captured_record() {
    let rows = capture();
    let count = word(&rows, "count", RECORD + COUNT) >> 32;
    let mut ports = Vec::new();
    for slot in 0..count {
        let base = RECORD + slot * SLOT_BYTES;
        let port = word(&rows, &format!("slot{slot}_port"), base + SLOT_PORT);
        let record = word(&rows, &format!("slot{slot}_record"), base + SLOT_RECORD);
        assert_eq!(
            word(&rows, &format!("record{slot}_port"), record + PORT),
            port
        );
        let tx = word(&rows, &format!("record{slot}_tx"), record + TX);
        let rx = word(&rows, &format!("record{slot}_rx"), record + RX);
        assert_eq!(
            game_os_virtual_uart_port("4.93", port),
            Some(&GameOsVirtualUartPort {
                firmware: "4.93",
                port,
                rx_buffer_size: rx,
                tx_buffer_size: tx,
            }),
            "port {port}"
        );
        ports.push(port);
    }
    ports.sort_unstable();
    let rows_4_93: Vec<u64> = GAME_OS_VIRTUAL_UART_PORTS
        .iter()
        .filter(|row| row.firmware == "4.93")
        .map(|row| row.port)
        .collect();
    assert_eq!(rows_4_93, ports);
}
